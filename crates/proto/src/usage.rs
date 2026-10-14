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
