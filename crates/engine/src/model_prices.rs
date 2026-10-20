//! Public per-token API prices from models.dev, for segments whose harness
//! reports no cost of its own.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use zeron_proto::{HarnessId, SessionStats};

const SOURCE: &str = "https://models.dev/api.json";
const MAX_AGE: Duration = Duration::from_secs(24 * 3600);

/// USD per million tokens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Price {
    #[serde(default)]
    input: f64,
    #[serde(default)]
    output: f64,
    #[serde(default)]
    cache_read: Option<f64>,
    #[serde(default)]
    cache_write: Option<f64>,
}

impl Price {
    fn micros(&self, stats: &SessionStats) -> u64 {
        self.per_million(
            stats.input_tokens,
            stats.output_tokens,
            stats.cache_read_tokens,
            stats.cache_write_tokens,
        )
        .max(0.0)
        .round() as u64
    }

    fn per_million(&self, input: u64, output: u64, cache_read: u64, cache_write: u64) -> f64 {
        input as f64 * self.input
            + output as f64 * self.output
            + cache_read as f64 * self.cache_read.unwrap_or(self.input)
            + cache_write as f64 * self.cache_write.unwrap_or(self.input)
    }

    pub(crate) fn usd(&self, input: u64, output: u64, cache_read: u64, cache_write: u64) -> f64 {
        self.per_million(input, output, cache_read, cache_write) / 1e6
    }

    fn blended(&self) -> f64 {
        (3.0 * self.input + self.output) / 4.0
    }

    pub(crate) fn cache_savings_usd(&self, cache_read: u64) -> f64 {
        cache_read as f64 * (self.input - self.cache_read.unwrap_or(self.input)) / 1e6
    }
}

type Table = BTreeMap<String, BTreeMap<String, Price>>;

#[derive(Serialize, Deserialize)]
struct Saved {
    fetched_at: u64,
    providers: Table,
}

static TABLE: RwLock<Option<Arc<Table>>> = RwLock::new(None);

fn location(root: &Path) -> PathBuf {
    root.join("model-prices.json")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn install(table: Table) {
    *TABLE.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(table));
}

fn parse(api: &Value) -> Table {
    let Some(providers) = api.as_object() else {
        return Table::new();
    };
    providers
        .iter()
        .filter_map(|(provider, body)| {
            let models: BTreeMap<String, Price> = body
                .get("models")?
                .as_object()?
                .iter()
                .filter_map(|(id, model)| {
                    Some((
                        id.clone(),
                        serde_json::from_value(model.get("cost")?.clone()).ok()?,
                    ))
                })
                .collect();
            (!models.is_empty()).then(|| (provider.clone(), models))
        })
        .collect()
}

pub(crate) fn start(root: &Path) {
    let path = location(root);
    let saved = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Saved>(&bytes).ok());
    let fresh = saved
        .as_ref()
        .is_some_and(|saved| now_secs().saturating_sub(saved.fetched_at) < MAX_AGE.as_secs());
    if let Some(saved) = saved {
        install(saved.providers);
    }
    if fresh || tokio::runtime::Handle::try_current().is_err() {
        return;
    }
    tokio::spawn(async move {
        if let Err(err) = refresh(&path).await {
            tracing::warn!(error = %err, "model price refresh failed");
        }
    });
}

async fn refresh(path: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let api: Value = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?
        .get(SOURCE)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let providers = parse(&api);
    if providers.is_empty() {
        return Err("models.dev returned no prices".into());
    }
    let bytes = serde_json::to_vec(&Saved {
        fetched_at: now_secs(),
        providers: providers.clone(),
    })?;
    let staging = path.with_extension("json.partial");
    std::fs::write(&staging, bytes)?;
    std::fs::rename(&staging, path)?;
    install(providers);
    Ok(())
}

fn lookup(table: &Table, harness: HarnessId, model: &str) -> Option<Price> {
    let model = model.split('[').next().unwrap_or(model).trim();
    if let Some((provider, id)) = model.split_once('/') {
        return table
            .get(provider)
            .and_then(|models| models.get(id))
            .copied()
            .or_else(|| lookup(table, harness, id));
    }
    if model.is_empty() || model == "default" {
        return None;
    }
    let maker = match harness {
        HarnessId::ClaudeCode => "anthropic",
        HarnessId::Codex => "openai",
        HarnessId::Grok => "xai",
        HarnessId::Antigravity => "google",
        _ => "",
    };
    [maker, "anthropic", "openai", "google", "xai"]
        .into_iter()
        .chain(table.keys().map(String::as_str))
        .find_map(|provider| table.get(provider)?.get(model))
        .copied()
}

/// Micro-USD.
pub(crate) fn estimate(
    harness: HarnessId,
    model: Option<&str>,
    stats: &SessionStats,
) -> Option<u64> {
    let table = TABLE.read().unwrap_or_else(|e| e.into_inner()).clone()?;
    Some(lookup(&table, harness, model?)?.micros(stats))
}

pub(crate) fn loaded() -> bool {
    TABLE.read().unwrap_or_else(|e| e.into_inner()).is_some()
}

pub(crate) fn price(harness: HarnessId, model: &str) -> Option<Price> {
    let table = TABLE.read().unwrap_or_else(|e| e.into_inner()).clone()?;
    lookup(&table, harness, model)
}

/// USD per million tokens at three input tokens to each output token.
pub(crate) fn blended(harness: HarnessId, model: &str) -> Option<f64> {
    price(harness, model).map(|price| price.blended())
}

#[cfg(test)]
pub(crate) fn blended_in(api: &Value, harness: HarnessId, model: &str) -> Option<f64> {
    lookup(&parse(api), harness, model).map(|price| price.blended())
}

#[cfg(test)]
pub(crate) fn install_for_test(api: Value) {
    install(parse(&api));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Table {
        parse(&serde_json::json!({
            "anthropic": {"models": {
                "claude-opus-5-5": {"cost": {"input": 4, "output": 20, "cache_read": 0.2, "cache_write": 5}}
            }},
            "openai": {"models": {
                "gpt-5.6-sol": {"cost": {"input": 4, "output": 20, "cache_read": 0.4}},
                "no-cost": {"name": "x"}
            }},
            "openrouter": {"models": {
                "gpt-5.6-sol": {"cost": {"input": 9, "output": 9}},
                "only-here": {"cost": {"input": 1, "output": 2}}
            }},
            "broken": {"models": "nope"}
        }))
    }

    #[test]
    fn parse_keeps_priced_models_only() {
        let table = table();
        assert_eq!(
            table.keys().collect::<Vec<_>>(),
            ["anthropic", "openai", "openrouter"]
        );
        assert!(!table["openai"].contains_key("no-cost"));
    }

    #[test]
    fn lookup_prefers_the_named_provider_then_the_model_maker() {
        let table = table();
        let sol = |harness, model| lookup(&table, harness, model).map(|p| p.input);
        assert_eq!(sol(HarnessId::Codex, "gpt-5.6-sol"), Some(4.0));
        assert_eq!(sol(HarnessId::Pi, "openrouter/gpt-5.6-sol"), Some(9.0));
        assert_eq!(sol(HarnessId::Opencode, "openai/gpt-5.6-sol"), Some(4.0));
        assert_eq!(sol(HarnessId::Opencode, "zen/gpt-5.6-sol"), Some(4.0));
        assert_eq!(sol(HarnessId::Cursor, "only-here"), Some(1.0));
        assert_eq!(sol(HarnessId::ClaudeCode, "claude-opus-5-5[1m]"), Some(4.0));
        assert_eq!(sol(HarnessId::Codex, "gpt-6-unknown"), None);
        assert_eq!(sol(HarnessId::Pi, "default"), None);
    }

    #[test]
    fn micros_price_each_token_kind_per_million() {
        let table = table();
        let stats = SessionStats {
            input_tokens: 1_000,
            output_tokens: 2_000,
            cache_read_tokens: 1_000_000,
            cache_write_tokens: 10_000,
            ..Default::default()
        };
        let opus = lookup(&table, HarnessId::ClaudeCode, "claude-opus-5-5").unwrap();
        assert_eq!(opus.micros(&stats), 4_000 + 40_000 + 200_000 + 50_000);
        let sol = lookup(&table, HarnessId::Codex, "gpt-5.6-sol").unwrap();
        assert_eq!(sol.micros(&stats), 4_000 + 40_000 + 400_000 + 40_000);
    }
}
