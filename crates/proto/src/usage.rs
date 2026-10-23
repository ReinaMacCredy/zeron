use serde::{Deserialize, Serialize};

use crate::agent::HarnessId;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub hourly: bool,
    pub periods: Vec<i64>,
    pub until_ms: i64,
    pub buckets: Vec<UsageBucket>,
    pub sessions: Vec<UsageSessions>,
    pub prices_loaded: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageTokens {
    pub uncached_input: u64,
    pub cached_input: u64,
    pub cache_write: u64,
    pub output: u64,
}

impl UsageTokens {
    pub fn total(&self) -> u64 {
        self.uncached_input + self.cached_input + self.cache_write + self.output
    }

    pub fn add(&mut self, other: &UsageTokens) {
        self.uncached_input += other.uncached_input;
        self.cached_input += other.cached_input;
        self.cache_write += other.cache_write;
        self.output += other.output;
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageBucket {
    pub period: usize,
    pub harness: HarnessId,
    pub model: String,
    pub tokens: UsageTokens,
    pub cost_usd: f64,
    pub cache_savings_usd: f64,
    pub records: u64,
    pub unpriced_records: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSessions {
    pub harness: HarnessId,
    pub sessions: u64,
}

pub const USAGE_PROVIDERS: [HarnessId; 3] =
    [HarnessId::Codex, HarnessId::ClaudeCode, HarnessId::Grok];
pub const USAGE_WINDOWS: [(u32, &str); 4] = [
    (1, "Past 24h"),
    (7, "7 days"),
    (30, "30 days"),
    (90, "90 days"),
];

pub struct ProviderTotal {
    pub harness: HarnessId,
    pub tokens: u64,
    pub cost: f64,
    pub sessions: u64,
}

pub struct ModelTotal {
    pub harness: HarnessId,
    pub model: String,
    pub tokens: u64,
    pub cost: f64,
}

pub struct PeriodTotal {
    pub start: i64,
    pub by_provider: Vec<(f64, u64)>,
    pub cost: f64,
    pub tokens: u64,
}

pub struct UsageView {
    pub tokens: UsageTokens,
    pub cost: f64,
    pub savings: f64,
    pub sessions: u64,
    pub unpriced: u64,
    pub providers: Vec<ProviderTotal>,
    pub models: Vec<ModelTotal>,
    pub periods: Vec<PeriodTotal>,
}

impl UsageView {
    pub fn new(summary: &UsageSummary) -> Self {
        let slot = |harness: HarnessId| USAGE_PROVIDERS.iter().position(|&h| h == harness);
        let mut providers: Vec<ProviderTotal> = USAGE_PROVIDERS
            .iter()
            .map(|&harness| ProviderTotal {
                harness,
                tokens: 0,
                cost: 0.0,
                sessions: summary
                    .sessions
                    .iter()
                    .find(|entry| entry.harness == harness)
                    .map_or(0, |entry| entry.sessions),
            })
            .collect();
        let mut periods: Vec<PeriodTotal> = summary
            .periods
            .iter()
            .map(|&start| PeriodTotal {
                start,
                by_provider: vec![(0.0, 0); USAGE_PROVIDERS.len()],
                cost: 0.0,
                tokens: 0,
            })
            .collect();
        let mut models: Vec<ModelTotal> = Vec::new();
        let mut view_tokens = UsageTokens::default();
        let (mut cost, mut savings, mut unpriced) = (0.0, 0.0, 0);
        for bucket in &summary.buckets {
            let tokens = bucket.tokens.total();
            view_tokens.add(&bucket.tokens);
            cost += bucket.cost_usd;
            savings += bucket.cache_savings_usd;
            unpriced += bucket.unpriced_records;
            let index = slot(bucket.harness);
            if let Some(provider) = index.map(|i| &mut providers[i]) {
                provider.tokens += tokens;
                provider.cost += bucket.cost_usd;
            }
            if let Some(period) = periods.get_mut(bucket.period) {
                period.cost += bucket.cost_usd;
                period.tokens += tokens;
                if let Some(i) = index {
                    period.by_provider[i].0 += bucket.cost_usd;
                    period.by_provider[i].1 += tokens;
                }
            }
            match models
                .iter_mut()
                .find(|m| m.harness == bucket.harness && m.model == bucket.model)
            {
                Some(model) => {
                    model.tokens += tokens;
                    model.cost += bucket.cost_usd;
                }
                None => models.push(ModelTotal {
                    harness: bucket.harness,
                    model: bucket.model.clone(),
                    tokens,
                    cost: bucket.cost_usd,
                }),
            }
        }
        models.sort_by(|a, b| b.cost.total_cmp(&a.cost).then(b.tokens.cmp(&a.tokens)));
        let sessions = providers.iter().map(|p| p.sessions).sum();
        Self {
            tokens: view_tokens,
            cost,
            savings,
            sessions,
            unpriced,
            providers,
            models,
            periods,
        }
    }

    pub fn active(&self) -> Vec<usize> {
        (0..self.providers.len())
            .filter(|&i| self.providers[i].tokens > 0 || self.providers[i].cost > 0.0)
            .collect()
    }
}

pub fn percent(part: f64, whole: f64) -> String {
    if whole > 0.0 {
        format!("{:.1}%", part / whole * 100.0)
    } else {
        "0.0%".into()
    }
}

pub fn plural(n: u64, one: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{} {one}s", group_thousands(n))
    }
}

pub fn group_thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

pub fn format_tokens(n: u64) -> String {
    let trim = |s: String| {
        if s.contains('.') {
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            s
        }
    };
    let n = n as f64;
    if n >= 1e10 {
        format!("{}B", trim(format!("{:.1}", n / 1e9)))
    } else if n >= 999_500_000.0 {
        format!("{}B", trim(format!("{:.2}", n / 1e9)))
    } else if n >= 1e7 {
        format!("{}M", trim(format!("{:.1}", n / 1e6)))
    } else if n >= 999_500.0 {
        format!("{}M", trim(format!("{:.2}", n / 1e6)))
    } else if n >= 1e5 {
        format!("{:.0}K", n / 1e3)
    } else if n >= 1e3 {
        format!("{}K", trim(format!("{:.1}", n / 1e3)))
    } else {
        format!("{n}")
    }
}

pub fn format_usd(x: f64) -> String {
    if x < 1.0 {
        return format!("${x:.3}");
    }
    let fixed = format!("{x:.2}");
    let (whole, cents) = fixed.split_once('.').unwrap_or((&fixed, "00"));
    let mut grouped = String::new();
    for (i, ch) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    format!("${grouped}.{cents}")
}
