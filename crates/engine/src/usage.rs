use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Days, Local, NaiveDate, TimeZone, Timelike};
use serde::Deserialize;
use serde_json::Value;
use zeron_proto::HarnessId;
use zeron_proto::usage::{UsageBucket, UsageSessions, UsageSummary, UsageTokens};

use crate::model_prices::{self, Price};

const MTIME_SLACK: Duration = Duration::from_secs(36 * 3600);
const FORK_COPY_MAX_GAP_MS: i64 = 1000;

#[derive(Clone, Debug)]
pub struct UsageRoots {
    roots: Vec<(HarnessId, PathBuf)>,
}

impl UsageRoots {
    pub fn new(claude_config_dir: &Path, codex_home: &Path, grok_home: &Path) -> Self {
        let mut roots = Self { roots: Vec::new() };
        roots.add_home(HarnessId::ClaudeCode, claude_config_dir);
        roots.add_home(HarnessId::Codex, codex_home);
        roots.add_home(HarnessId::Grok, grok_home);
        roots
    }

    pub fn add_home(&mut self, harness: HarnessId, home: &Path) {
        let dir = match harness {
            HarnessId::ClaudeCode => home.join("projects"),
            HarnessId::Codex | HarnessId::Grok => home.join("sessions"),
            _ => return,
        };
        if !self.roots.iter().any(|(_, known)| *known == dir) {
            self.roots.push((harness, dir));
        }
    }
}

#[derive(Clone, Debug)]
struct Record {
    ts_ms: i64,
    model: Arc<str>,
    session: Arc<str>,
    tokens: UsageTokens,
    reported_usd: Option<f64>,
    key: Option<u64>,
}

struct CachedFile {
    len: u64,
    modified: SystemTime,
    records: Arc<Vec<Record>>,
}

static CACHE: LazyLock<Mutex<HashMap<PathBuf, CachedFile>>> = LazyLock::new(Default::default);
static SCAN: Mutex<()> = Mutex::new(());

pub fn summary(roots: &UsageRoots, days: u32, now: DateTime<Local>) -> UsageSummary {
    let _scan = SCAN.lock().unwrap_or_else(PoisonError::into_inner);
    let hourly = days <= 1;
    let (periods, until_ms) = window(days.clamp(1, 366), now);
    let since_ms = periods.first().copied().unwrap_or(until_ms);
    let cutoff = UNIX_EPOCH + Duration::from_millis(since_ms.max(0) as u64);
    let cutoff = cutoff.checked_sub(MTIME_SLACK).unwrap_or(UNIX_EPOCH);

    let mut files = Vec::new();
    let is_jsonl = |path: &Path| path.extension().is_some_and(|ext| ext == "jsonl");
    let is_grok_updates =
        |path: &Path| path.file_name().is_some_and(|name| name == "updates.jsonl");
    for (harness, dir) in &roots.roots {
        let keep: &dyn Fn(&Path) -> bool = match harness {
            HarnessId::Grok => &is_grok_updates,
            _ => &is_jsonl,
        };
        walk(dir, *harness, keep, cutoff, &mut files);
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));

    let mut aggregate = Aggregate::new(periods, until_ms);
    for (file, records) in files.iter().zip(load(&files)) {
        for record in records.iter() {
            aggregate.add(file.harness, record);
        }
    }
    aggregate.finish(hourly)
}

fn window(days: u32, now: DateTime<Local>) -> (Vec<i64>, i64) {
    if days == 1 {
        let hour = now
            .with_minute(0)
            .and_then(|t| t.with_second(0))
            .and_then(|t| t.with_nanosecond(0))
            .unwrap_or(now);
        let periods = (0..24)
            .rev()
            .map(|back| (hour - chrono::Duration::hours(back)).timestamp_millis())
            .collect();
        return (periods, now.timestamp_millis());
    }
    let today = now.date_naive();
    let periods = (0..days as u64)
        .rev()
        .filter_map(|back| today.checked_sub_days(Days::new(back)))
        .map(local_midnight)
        .collect();
    let until = today
        .checked_add_days(Days::new(1))
        .map_or(now.timestamp_millis(), local_midnight);
    (periods, until)
}

fn local_midnight(day: NaiveDate) -> i64 {
    let naive = day.and_time(chrono::NaiveTime::MIN);
    Local.from_local_datetime(&naive).earliest().map_or_else(
        || naive.and_utc().timestamp_millis(),
        |t| t.timestamp_millis(),
    )
}

struct FileEntry {
    path: PathBuf,
    harness: HarnessId,
    len: u64,
    modified: SystemTime,
}

fn walk(
    dir: &Path,
    harness: HarnessId,
    keep: &dyn Fn(&Path) -> bool,
    cutoff: SystemTime,
    out: &mut Vec<FileEntry>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if kind.is_dir() {
            walk(&path, harness, keep, cutoff, out);
            continue;
        }
        if !kind.is_file() || !keep(&path) {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        if modified >= cutoff {
            out.push(FileEntry {
                path,
                harness,
                len: meta.len(),
                modified,
            });
        }
    }
}

fn load(files: &[FileEntry]) -> Vec<Arc<Vec<Record>>> {
    let mut out: Vec<Option<Arc<Vec<Record>>>> = {
        let cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
        files
            .iter()
            .map(|file| {
                cache
                    .get(&file.path)
                    .filter(|hit| hit.len == file.len && hit.modified == file.modified)
                    .map(|hit| hit.records.clone())
            })
            .collect()
    };
    let misses: Vec<usize> = (0..files.len()).filter(|&i| out[i].is_none()).collect();
    let workers = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .clamp(1, 8)
        .min(misses.len().max(1));
    let parsed: Vec<(usize, Option<Vec<Record>>)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|worker| {
                let misses = &misses;
                scope.spawn(move || {
                    misses
                        .iter()
                        .skip(worker)
                        .step_by(workers)
                        .map(|&i| (i, parse_file(files[i].harness, &files[i].path)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap_or_default())
            .collect()
    });
    let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    for (i, records) in parsed {
        let Some(records) = records else {
            continue;
        };
        let records = Arc::new(records);
        cache.insert(
            files[i].path.clone(),
            CachedFile {
                len: files[i].len,
                modified: files[i].modified,
                records: records.clone(),
            },
        );
        out[i] = Some(records);
    }
    out.into_iter().map(Option::unwrap_or_default).collect()
}

fn parse_file(harness: HarnessId, path: &Path) -> Option<Vec<Record>> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = BufReader::with_capacity(1 << 16, file);
    let mut parser = Parser::new(harness);
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => return None,
        }
        if let Ok(text) = std::str::from_utf8(&line) {
            parser.line(text.trim_end());
        }
    }
    Some(parser.records)
}

fn count(value: Option<&Value>) -> u64 {
    value
        .and_then(Value::as_f64)
        .filter(|v| v.is_finite() && *v > 0.0)
        .map_or(0, |v| v.trunc() as u64)
}

fn timestamp_ms(text: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|t| t.timestamp_millis())
}

fn key_hash(parts: &[&str]) -> u64 {
    let mut hasher = DefaultHasher::new();
    parts.hash(&mut hasher);
    hasher.finish()
}

#[derive(Deserialize)]
struct ClaudeLine {
    #[serde(rename = "type")]
    kind: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    #[serde(rename = "costUSD")]
    cost_usd: Option<f64>,
    message: Option<ClaudeMessage>,
}

#[derive(Deserialize)]
struct ClaudeMessage {
    id: Option<String>,
    model: Option<String>,
    usage: Option<Value>,
}

#[derive(Default)]
struct CodexState {
    model: Option<Arc<str>>,
    session: Option<Arc<str>>,
    last_usage: Option<Value>,
    saw_meta: bool,
    fork_anchor_ms: Option<i64>,
}

struct Parser {
    harness: HarnessId,
    records: Vec<Record>,
    strings: HashMap<String, Arc<str>>,
    codex: CodexState,
}

impl Parser {
    fn new(harness: HarnessId) -> Self {
        Self {
            harness,
            records: Vec::new(),
            strings: HashMap::new(),
            codex: CodexState::default(),
        }
    }

    fn intern(&mut self, text: &str) -> Arc<str> {
        if let Some(found) = self.strings.get(text) {
            return found.clone();
        }
        let shared: Arc<str> = Arc::from(text);
        self.strings.insert(text.to_owned(), shared.clone());
        shared
    }

    fn push(
        &mut self,
        ts_ms: i64,
        model: Arc<str>,
        session: Arc<str>,
        tokens: UsageTokens,
        reported_usd: Option<f64>,
        key: Option<u64>,
    ) {
        if tokens.total() == 0 {
            return;
        }
        self.records.push(Record {
            ts_ms,
            model,
            session,
            tokens,
            reported_usd,
            key,
        });
    }

    fn line(&mut self, text: &str) {
        match self.harness {
            HarnessId::ClaudeCode => self.claude(text),
            HarnessId::Codex => self.codex(text),
            HarnessId::Grok => self.grok(text),
            _ => {}
        }
    }

    fn claude(&mut self, text: &str) {
        if !text.contains("\"usage\"") || !text.contains("\"assistant\"") {
            return;
        }
        let Ok(line) = serde_json::from_str::<ClaudeLine>(text) else {
            return;
        };
        if line.kind.as_deref() != Some("assistant") {
            return;
        }
        let Some(message) = line.message else {
            return;
        };
        let (Some(usage), Some(model), Some(ts_ms)) = (
            message.usage.filter(Value::is_object),
            message.model.filter(|m| !m.is_empty()),
            line.timestamp.as_deref().and_then(timestamp_ms),
        ) else {
            return;
        };
        let key = (message.id.is_some() || line.request_id.is_some()).then(|| {
            key_hash(&[
                message.id.as_deref().unwrap_or(""),
                line.request_id.as_deref().unwrap_or(""),
            ])
        });
        let model = self.intern(&model);
        let session = self.intern(line.session_id.as_deref().unwrap_or(""));
        let reported = line.cost_usd.filter(|v| v.is_finite());
        self.push(
            ts_ms,
            model,
            session.clone(),
            claude_tokens(&usage),
            reported,
            key,
        );
        let iterations = usage.get("iterations").and_then(Value::as_array);
        for (index, iteration) in iterations.into_iter().flatten().enumerate() {
            let Some(model) = iteration.get("model").and_then(Value::as_str) else {
                continue;
            };
            if iteration.get("type").and_then(Value::as_str) == Some("message") {
                continue;
            }
            let key = key.map(|key| key_hash(&[&key.to_string(), &index.to_string()]));
            let model = self.intern(model);
            self.push(
                ts_ms,
                model,
                session.clone(),
                claude_tokens(iteration),
                None,
                key,
            );
        }
    }

    fn codex(&mut self, text: &str) {
        if !text.contains("token_count")
            && !text.contains("turn_context")
            && !text.contains("session_meta")
        {
            return;
        }
        let Ok(line) = serde_json::from_str::<Value>(text) else {
            return;
        };
        let Some(payload) = line.get("payload").filter(|p| p.is_object()) else {
            return;
        };
        let ts_ms = line
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(timestamp_ms);
        match line.get("type").and_then(Value::as_str) {
            Some("session_meta") => {
                if self.codex.saw_meta {
                    return;
                }
                self.codex.saw_meta = true;
                let id = payload
                    .get("id")
                    .or_else(|| payload.get("session_id"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                self.codex.session = Some(self.intern(id));
                let forked = payload.get("forked_from_id").is_some_and(Value::is_string)
                    || payload
                        .pointer("/source/subagent/thread_spawn/parent_thread_id")
                        .is_some_and(Value::is_string);
                if forked {
                    self.codex.fork_anchor_ms = ts_ms;
                }
                return;
            }
            Some("turn_context") => {
                if let Some(model) = payload.get("model").and_then(Value::as_str) {
                    self.codex.model = Some(self.intern(model));
                }
                return;
            }
            _ => {}
        }
        if payload.get("type").and_then(Value::as_str) != Some("token_count") {
            return;
        }
        let Some(last) = payload
            .pointer("/info/last_token_usage")
            .filter(|v| v.is_object())
        else {
            return;
        };
        let (Some(ts_ms), Some(model)) = (ts_ms, self.codex.model.clone()) else {
            return;
        };
        if self.codex.last_usage.as_ref() == Some(last) {
            return;
        }
        self.codex.last_usage = Some(last.clone());
        if let Some(anchor) = self.codex.fork_anchor_ms {
            if ts_ms - anchor < FORK_COPY_MAX_GAP_MS {
                self.codex.fork_anchor_ms = Some(ts_ms);
                return;
            }
            self.codex.fork_anchor_ms = None;
        }
        let input = count(last.get("input_tokens"));
        let cached = count(last.get("cached_input_tokens"));
        let cache_write = count(last.get("cache_write_input_tokens"));
        let tokens = UsageTokens {
            uncached_input: input.saturating_sub(cached + cache_write),
            cached_input: cached,
            cache_write,
            output: count(last.get("output_tokens")),
        };
        let session = match self.codex.session.clone() {
            Some(session) => session,
            None => self.intern(""),
        };
        self.push(ts_ms, model, session, tokens, None, None);
    }

    fn grok(&mut self, text: &str) {
        if !text.contains("turn_completed") {
            return;
        }
        let Ok(line) = serde_json::from_str::<Value>(text) else {
            return;
        };
        let Some(params) = line.get("params") else {
            return;
        };
        let Some(update) = params.get("update") else {
            return;
        };
        if update.get("sessionUpdate").and_then(Value::as_str) != Some("turn_completed") {
            return;
        }
        let Some(usage) = update.get("usage").filter(|u| u.is_object()) else {
            return;
        };
        let ts_ms = params
            .pointer("/_meta/agentTimestampMs")
            .and_then(Value::as_f64)
            .or_else(|| {
                let seconds_or_ms = line.get("timestamp")?.as_f64()?;
                Some(if seconds_or_ms > 1e12 {
                    seconds_or_ms
                } else {
                    seconds_or_ms * 1000.0
                })
            });
        let Some(ts_ms) = ts_ms.filter(|t| t.is_finite()).map(|t| t as i64) else {
            return;
        };
        let session_id = params
            .get("sessionId")
            .and_then(Value::as_str)
            .unwrap_or("");
        let prompt_id = update.get("prompt_id").and_then(Value::as_str);
        let session = self.intern(session_id);
        let key = |model: &str| prompt_id.map(|prompt| key_hash(&[session_id, prompt, model]));
        let top_cost = grok_cost(usage);
        let entries: Vec<(&str, UsageTokens, Option<f64>)> = usage
            .get("modelUsage")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter(|(model, _)| !model.is_empty())
            .map(|(model, entry)| (model.as_str(), grok_tokens(entry), grok_cost(entry)))
            .filter(|(_, tokens, _)| tokens.total() > 0)
            .collect();
        if entries.is_empty() {
            let model = self.intern("grok");
            self.push(
                ts_ms,
                model,
                session,
                grok_tokens(usage),
                top_cost,
                key("grok"),
            );
            return;
        }
        let ticked: f64 = entries.iter().filter_map(|(_, _, cost)| *cost).sum();
        let remaining = top_cost.map(|top| (top - ticked).max(0.0));
        let unticked_tokens: u64 = entries
            .iter()
            .filter(|(_, _, cost)| cost.is_none())
            .map(|(_, tokens, _)| tokens.total())
            .sum();
        for (model, tokens, cost) in entries {
            let cost = cost.or_else(|| {
                remaining.map(|rest| rest * tokens.total() as f64 / unticked_tokens as f64)
            });
            let interned = self.intern(model);
            self.push(ts_ms, interned, session.clone(), tokens, cost, key(model));
        }
    }
}

fn claude_tokens(usage: &Value) -> UsageTokens {
    UsageTokens {
        uncached_input: count(usage.get("input_tokens")),
        cached_input: count(usage.get("cache_read_input_tokens")),
        cache_write: count(usage.get("cache_creation_input_tokens")),
        output: count(usage.get("output_tokens")),
    }
}

fn grok_tokens(usage: &Value) -> UsageTokens {
    let cached = count(usage.get("cachedReadTokens"));
    let cache_write = count(usage.get("cacheCreationTokens"));
    UsageTokens {
        uncached_input: count(usage.get("inputTokens")).saturating_sub(cached + cache_write),
        cached_input: cached,
        cache_write,
        output: count(usage.get("outputTokens")),
    }
}

fn grok_cost(usage: &Value) -> Option<f64> {
    usage
        .get("costUsdTicks")
        .and_then(Value::as_f64)
        .filter(|ticks| ticks.is_finite() && *ticks >= 0.0)
        .map(|ticks| ticks / 1e10)
}

struct Aggregate {
    periods: Vec<i64>,
    until_ms: i64,
    seen: HashSet<u64>,
    prices: HashMap<(HarnessId, Arc<str>), Option<Price>>,
    buckets: HashMap<(usize, HarnessId, Arc<str>), UsageBucket>,
    sessions: HashMap<HarnessId, HashSet<Arc<str>>>,
}

impl Aggregate {
    fn new(periods: Vec<i64>, until_ms: i64) -> Self {
        Self {
            periods,
            until_ms,
            seen: HashSet::new(),
            prices: HashMap::new(),
            buckets: HashMap::new(),
            sessions: HashMap::new(),
        }
    }

    fn add(&mut self, harness: HarnessId, record: &Record) {
        if let Some(key) = record.key
            && !self.seen.insert(key)
        {
            return;
        }
        let index = self.periods.partition_point(|&start| start <= record.ts_ms);
        if index == 0 || record.ts_ms >= self.until_ms {
            return;
        }
        let period = index - 1;
        let price = *self
            .prices
            .entry((harness, record.model.clone()))
            .or_insert_with(|| model_prices::price(harness, &record.model));
        let bucket = self
            .buckets
            .entry((period, harness, record.model.clone()))
            .or_insert_with(|| UsageBucket {
                period,
                harness,
                model: record.model.to_string(),
                tokens: UsageTokens::default(),
                cost_usd: 0.0,
                cache_savings_usd: 0.0,
                records: 0,
                unpriced_records: 0,
            });
        let tokens = &record.tokens;
        bucket.tokens.add(tokens);
        bucket.records += 1;
        match (record.reported_usd, price) {
            (Some(usd), _) => bucket.cost_usd += usd,
            (None, Some(price)) => {
                bucket.cost_usd += price.usd(
                    tokens.uncached_input,
                    tokens.output,
                    tokens.cached_input,
                    tokens.cache_write,
                )
            }
            (None, None) => bucket.unpriced_records += 1,
        }
        if let Some(price) = price {
            bucket.cache_savings_usd += price.cache_savings_usd(tokens.cached_input);
        }
        if !record.session.is_empty() {
            self.sessions
                .entry(harness)
                .or_default()
                .insert(record.session.clone());
        }
    }

    fn finish(self, hourly: bool) -> UsageSummary {
        let mut buckets: Vec<UsageBucket> = self.buckets.into_values().collect();
        buckets.sort_by(|a, b| {
            (a.period, harness_rank(a.harness), &a.model).cmp(&(
                b.period,
                harness_rank(b.harness),
                &b.model,
            ))
        });
        let mut sessions: Vec<UsageSessions> = self
            .sessions
            .into_iter()
            .map(|(harness, ids)| UsageSessions {
                harness,
                sessions: ids.len() as u64,
            })
            .collect();
        sessions.sort_by_key(|entry| harness_rank(entry.harness));
        UsageSummary {
            hourly,
            periods: self.periods,
            until_ms: self.until_ms,
            buckets,
            sessions,
            prices_loaded: model_prices::loaded(),
        }
    }
}

fn harness_rank(harness: HarnessId) -> u8 {
    match harness {
        HarnessId::Codex => 0,
        HarnessId::ClaudeCode => 1,
        _ => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    fn parse(harness: HarnessId, lines: &[&str]) -> Vec<Record> {
        let mut parser = Parser::new(harness);
        for line in lines {
            parser.line(line);
        }
        parser.records
    }

    #[test]
    fn claude_content_block_frames_and_advisor_calls_count_once() {
        let frame = |block: &str| {
            format!(
                r#"{{"type":"assistant","timestamp":"2026-10-14T01:00:00Z","requestId":"req_1","sessionId":"s1","message":{{"id":"msg_1","model":"claude-opus-5-5","content":[{{"type":"{block}"}}],"usage":{{"input_tokens":10,"cache_read_input_tokens":100,"cache_creation_input_tokens":5,"output_tokens":7,"iterations":[{{"type":"message","input_tokens":10}},{{"type":"advisor_message","model":"claude-fable-5-1","input_tokens":900,"output_tokens":30}}]}}}}}}"#
            )
        };
        let records = parse(HarnessId::ClaudeCode, &[&frame("thinking"), &frame("text")]);
        let mut aggregate = Aggregate::new(vec![0], i64::MAX);
        for record in &records {
            aggregate.add(HarnessId::ClaudeCode, record);
        }
        let summary = aggregate.finish(false);
        assert_eq!(summary.buckets.len(), 2);
        assert_eq!(summary.buckets[0].model, "claude-fable-5-1");
        assert_eq!(summary.buckets[0].tokens.total(), 930);
        assert_eq!(summary.buckets[0].records, 1);
        assert_eq!(
            summary.buckets[1].tokens,
            UsageTokens {
                uncached_input: 10,
                cached_input: 100,
                cache_write: 5,
                output: 7
            }
        );
        assert_eq!(summary.buckets[1].records, 1);
    }

    #[test]
    fn codex_counts_each_turn_once_and_skips_fork_copies() {
        let meta = r#"{"timestamp":"2026-10-14T00:00:00.000Z","type":"session_meta","payload":{"id":"child","forked_from_id":"parent"}}"#;
        let context = r#"{"timestamp":"2026-10-14T00:00:00.100Z","type":"turn_context","payload":{"model":"gpt-5.6-sol"}}"#;
        let usage = |ts: &str, input: u64| {
            format!(
                r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"token_count","info":{{"last_token_usage":{{"input_tokens":{input},"cached_input_tokens":40,"output_tokens":5}}}}}}}}"#
            )
        };
        let records = parse(
            HarnessId::Codex,
            &[
                meta,
                context,
                &usage("2026-10-14T00:00:00.200Z", 100),
                &usage("2026-10-14T00:00:00.900Z", 110),
                &usage("2026-10-14T00:00:05.000Z", 120),
                &usage("2026-10-14T00:00:06.000Z", 120),
                &usage("2026-10-14T00:00:09.000Z", 130),
            ],
        );
        let inputs: Vec<u64> = records.iter().map(|r| r.tokens.uncached_input).collect();
        assert_eq!(inputs, [80, 90]);
        assert_eq!(&*records[0].session, "child");
    }

    #[test]
    fn an_added_instance_home_is_scanned_once() {
        let temp = tempfile::tempdir().unwrap();
        let instance = temp.path().join("codex-work");
        let rollout = instance.join("sessions/2026/10/14/rollout.jsonl");
        std::fs::create_dir_all(rollout.parent().unwrap()).unwrap();
        std::fs::write(
            &rollout,
            [
                r#"{"timestamp":"2026-10-14T00:00:00.000Z","type":"session_meta","payload":{"id":"s1"}}"#,
                r#"{"timestamp":"2026-10-14T00:00:00.100Z","type":"turn_context","payload":{"model":"gpt-5.6-sol"}}"#,
                r#"{"timestamp":"2026-10-14T00:00:00.200Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"cached_input_tokens":40,"output_tokens":5}}}}"#,
            ]
            .join("\n"),
        )
        .unwrap();
        let now = Local.with_ymd_and_hms(2026, 10, 14, 12, 0, 0).unwrap();
        let records = |roots: &UsageRoots| -> u64 {
            summary(roots, 7, now)
                .buckets
                .iter()
                .map(|bucket| bucket.records)
                .sum()
        };
        let mut roots = UsageRoots::new(
            &temp.path().join("claude"),
            &temp.path().join("codex"),
            &temp.path().join("grok"),
        );
        assert_eq!(records(&roots), 0);
        roots.add_home(HarnessId::Codex, &instance);
        roots.add_home(HarnessId::Codex, &instance);
        assert_eq!(records(&roots), 1);
    }

    #[test]
    fn grok_splits_unticked_cost_by_tokens() {
        let line = r#"{"timestamp":1789658745,"params":{"sessionId":"g1","update":{"sessionUpdate":"turn_completed","prompt_id":"p1","usage":{"inputTokens":300,"outputTokens":0,"costUsdTicks":30000000000,"modelUsage":{"a":{"inputTokens":100,"outputTokens":0},"b":{"inputTokens":200,"outputTokens":0}}}},"_meta":{"agentTimestampMs":1789658745789}}}"#;
        let records = parse(HarnessId::Grok, &[line]);
        let costs: Vec<_> = records
            .iter()
            .map(|r| (r.model.to_string(), r.reported_usd))
            .collect();
        assert_eq!(
            costs,
            [("a".to_owned(), Some(1.0)), ("b".to_owned(), Some(2.0))]
        );
        assert_eq!(records[0].ts_ms, 1789658745789);
    }

    #[test]
    fn days_bucket_by_local_midnight() {
        let now = Local.with_ymd_and_hms(2026, 10, 14, 15, 30, 0).unwrap();
        let (periods, until) = window(7, now);
        assert_eq!(periods.len(), 7);
        let first = Local.timestamp_millis_opt(periods[0]).unwrap();
        assert_eq!((first.day(), first.hour()), (8, 0));
        let end = Local.timestamp_millis_opt(until).unwrap();
        assert_eq!((end.day(), end.hour()), (15, 0));
        let (hours, until) = window(1, now);
        assert_eq!(hours.len(), 24);
        assert_eq!(until, now.timestamp_millis());
        let last = Local.timestamp_millis_opt(hours[23]).unwrap();
        assert_eq!((last.hour(), last.minute()), (15, 0));
    }
}
