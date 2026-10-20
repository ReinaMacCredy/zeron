//! Thresholds, questions and input shape are benched in .maestro/topic/compact-nudge-bench/jev (report.md); change them only with a rerun.

use std::sync::LazyLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use zeron_doc::{ClassifierRecord, ClassifierVerdict};

use crate::agent_accounts::keychain;
use crate::predictions::{head, key_account, valid_api_key};
use crate::registry::{CompactionSettings, HarnessRegistry, JevSource};

const MODEL_LABEL: &str = "Jev 1.14";
const QUESTIONS: &str = include_str!("jev_questions.json");
const TIMEOUT: Duration = Duration::from_secs(20);
const STATE_CHARS: usize = 10_000;
const BODY_CHARS: usize = 300;
const NEW_MIN: f64 = 0.55;
const NEW_TASK_MIN: f64 = 0.55;
const CONTINUE_SHARE_MIN: f64 = 0.85;

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .unwrap_or_default()
});

impl JevSource {
    fn endpoint(self) -> &'static str {
        match self {
            Self::TypeSafe => "https://api.typesafe.ai/v1/systemone",
            Self::OpenRouter => "https://openrouter.ai/api/v1/systemone",
        }
    }

    fn model(self) -> &'static str {
        match self {
            Self::TypeSafe => "jev-1.14.0",
            Self::OpenRouter => "typesafe/jev-1.14",
        }
    }

    fn key_service(self) -> &'static str {
        match self {
            Self::TypeSafe => "Zeron TypeSafe API key",
            Self::OpenRouter => "Zeron OpenRouter API key",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionSettingsUpdate {
    #[serde(flatten)]
    pub settings: CompactionSettings,
    /// Write-only, for the source in `jev`; an empty string removes the stored key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jev_api_key: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionSettingsState {
    #[serde(flatten)]
    pub settings: CompactionSettings,
    pub has_jev_key: bool,
}

async fn api_key(registry: &HarnessRegistry, source: JevSource) -> Option<String> {
    keychain::read_secret(source.key_service(), &key_account(registry)?).await
}

pub(crate) async fn settings_state(registry: &HarnessRegistry) -> CompactionSettingsState {
    let settings = registry.compaction_settings();
    let has_jev_key = match settings.jev {
        Some(source) => api_key(registry, source).await.is_some(),
        None => false,
    };
    CompactionSettingsState {
        settings,
        has_jev_key,
    }
}

pub(crate) async fn update_settings(
    registry: &HarnessRegistry,
    update: CompactionSettingsUpdate,
) -> Result<CompactionSettingsState, String> {
    let api_key = update.jev_api_key.map(|key| key.trim().to_string());
    if let Some(key) = api_key.as_deref()
        && !key.is_empty()
        && !valid_api_key(key)
    {
        return Err("That does not look like an API key".into());
    }
    let source = update.settings.jev;
    if api_key.is_some() && source.is_none() {
        return Err("Pick a Jev source before saving its key".into());
    }
    if let (Some(key), Some(source)) = (api_key, source) {
        let account =
            key_account(registry).ok_or_else(|| "The engine has no data directory".to_string())?;
        if key.is_empty() {
            keychain::delete_secret(source.key_service(), &account).await;
        } else {
            keychain::write_secret(source.key_service(), &account, &key)
                .await
                .map_err(|error| error.to_string())?;
        }
    }
    registry.set_compaction_settings(update.settings)?;
    Ok(settings_state(registry).await)
}

fn request_body(source: JevSource, input: &str) -> serde_json::Value {
    let questions: serde_json::Value =
        serde_json::from_str(QUESTIONS).expect("jev_questions.json is JSON");
    serde_json::json!({
        "model": source.model(),
        "state": crate::compact_boundary::clip_start(input, STATE_CHARS),
        "questions": questions,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Deserialize)]
struct Probabilities {
    #[serde(default, rename = "NEW")]
    new: f64,
    #[serde(default, rename = "CARRY")]
    carry: f64,
    #[serde(default, rename = "CONTINUE")]
    cont: f64,
}

#[derive(Debug, Deserialize)]
struct Verdict {
    probabilities: Probabilities,
}

#[derive(Debug, Deserialize)]
struct Noul {
    noul: f64,
}

#[derive(Debug, Deserialize)]
struct Answers {
    verdict: Verdict,
    new_task: Noul,
}

#[derive(Debug, Default, Deserialize)]
struct Usage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct Reply {
    answers: Answers,
    #[serde(default)]
    usage: Usage,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Scores {
    p: Probabilities,
    new_task: f64,
}

impl Scores {
    fn verdict(&self) -> ClassifierVerdict {
        if self.p.new + self.p.carry + self.p.cont <= 0.0 {
            return ClassifierVerdict::Failed;
        }
        if self.p.new >= NEW_MIN && self.new_task >= NEW_TASK_MIN {
            return ClassifierVerdict::New;
        }
        let pair = self.p.carry + self.p.cont;
        if pair > 0.0 && self.p.cont / pair >= CONTINUE_SHARE_MIN {
            ClassifierVerdict::Continue
        } else {
            ClassifierVerdict::Carry
        }
    }

    fn line(&self) -> String {
        format!(
            "NEW {:.2} · CARRY {:.2} · CONTINUE {:.2} · new task {:.2}",
            self.p.new, self.p.carry, self.p.cont, self.new_task
        )
    }
}

fn parse_reply(body: &str) -> Result<(Scores, Usage), String> {
    let reply: Reply = serde_json::from_str(body)
        .map_err(|error| format!("Unreadable reply ({error}): {}", head(body, BODY_CHARS)))?;
    Ok((
        Scores {
            p: reply.answers.verdict.probabilities,
            new_task: reply.answers.new_task.noul,
        },
        reply.usage,
    ))
}

async fn ask(
    registry: &HarnessRegistry,
    source: JevSource,
    input: &str,
) -> Result<(Scores, Usage), String> {
    let key = api_key(registry, source)
        .await
        .ok_or_else(|| "No Jev API key in the Keychain".to_string())?;
    // A bench hook: a release build never sends the key anywhere but the source.
    let endpoint = std::env::var("ZERON_JEV_ENDPOINT")
        .ok()
        .filter(|_| cfg!(debug_assertions))
        .unwrap_or_else(|| source.endpoint().to_string());
    let response = HTTP
        .post(&endpoint)
        .bearer_auth(&key)
        .json(&request_body(source, input))
        .send()
        .await
        .map_err(|error| format!("Jev unreachable: {}", error.without_url()))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| format!("Jev reply lost: {}", error.without_url()))?
        .replace(&key, "[key]");
    if !status.is_success() {
        return Err(format!("HTTP {status}: {}", head(&body, BODY_CHARS)));
    }
    parse_reply(&body)
}

pub(crate) async fn classify(
    registry: &HarnessRegistry,
    source: JevSource,
    input: String,
) -> ClassifierRecord {
    let started = std::time::Instant::now();
    let mut record = ClassifierRecord {
        model: Some(MODEL_LABEL.into()),
        ..Default::default()
    };
    match ask(registry, source, &input).await {
        Ok((scores, usage)) => {
            record.verdict = scores.verdict();
            record.reply = scores.line();
            record.input_tokens = usage.input_tokens;
            record.output_tokens = usage.output_tokens;
        }
        Err(reason) => {
            tracing::debug!(reason = %reason, "Jev classifier skipped");
            record.reply = reason;
        }
    }
    record.input = input;
    record.duration_ms = started.elapsed().as_millis() as u64;
    record
}

#[cfg(test)]
mod tests {
    use super::*;
    use ClassifierVerdict::*;

    fn scores(new: f64, carry: f64, cont: f64, new_task: f64) -> Scores {
        Scores {
            p: Probabilities { new, carry, cont },
            new_task,
        }
    }

    #[test]
    fn rule_h_needs_both_new_signals_and_a_clear_continue_share() {
        assert_eq!(scores(0.55, 0.3, 0.15, 0.55).verdict(), New);
        assert_eq!(scores(0.54, 0.3, 0.16, 0.9).verdict(), Carry);
        assert_eq!(scores(0.9, 0.05, 0.05, 0.54).verdict(), Carry);
        assert_eq!(scores(0.0, 0.15, 0.85, 0.0).verdict(), Continue);
        assert_eq!(scores(0.0, 0.16, 0.84, 0.0).verdict(), Carry);
        assert_eq!(scores(0.6, 0.0, 0.4, 0.1).verdict(), Continue);
        assert_eq!(scores(1.0, 0.0, 0.0, 0.2).verdict(), Carry);
        assert_eq!(scores(0.0, 0.0, 0.0, 0.9).verdict(), Failed);
    }

    #[test]
    fn the_openrouter_sample_parses_with_missing_keys_as_zero() {
        let sample = r#"{"id":"gen-1","provider":"TypeSafe","model":"typesafe/jev-1.14-20261014","answers":{"verdict":{"type":"choice","choice":"CARRY","probabilities":{"NEW":0,"CARRY":0.83,"CONTINUE":0.17},"confidence":0.74},"new_task":{"type":"noul","noul":0.03}},"usage":{"input_tokens":2019,"output_tokens":40,"cost":0.0000848}}"#;
        let (scores, usage) = parse_reply(sample).unwrap();
        assert_eq!(scores.verdict(), Carry);
        assert_eq!(
            scores.line(),
            "NEW 0.00 · CARRY 0.83 · CONTINUE 0.17 · new task 0.03"
        );
        assert_eq!(
            (usage.input_tokens, usage.output_tokens),
            (Some(2019), Some(40))
        );
        let sparse =
            r#"{"answers":{"verdict":{"probabilities":{"CONTINUE":0.9}},"new_task":{"noul":0.1}}}"#;
        let (scores, usage) = parse_reply(sparse).unwrap();
        assert_eq!(
            scores.p,
            Probabilities {
                new: 0.0,
                carry: 0.0,
                cont: 0.9
            }
        );
        assert_eq!(scores.verdict(), Continue);
        assert_eq!(usage.input_tokens, None);
        let error = parse_reply(r#"{"error":"bad"}"#).unwrap_err();
        assert!(error.ends_with(r#"{"error":"bad"}"#), "{error}");
    }

    #[test]
    fn each_source_sends_its_model_id_the_benched_questions_and_a_clipped_state() {
        let questions: serde_json::Value = serde_json::from_str(QUESTIONS).unwrap();
        assert_eq!(questions["verdict"]["type"], "choice");
        assert_eq!(questions["new_task"]["type"], "noul");
        for (source, model) in [
            (JevSource::TypeSafe, "jev-1.14.0"),
            (JevSource::OpenRouter, "typesafe/jev-1.14"),
        ] {
            let body = request_body(source, "Recent conversation:\nuser: hi");
            assert_eq!(body["model"], model);
            assert_eq!(body["state"], "Recent conversation:\nuser: hi");
            assert_eq!(body["questions"], questions);
        }
        let long = format!("{}{}", "a".repeat(5), "é".repeat(STATE_CHARS));
        let body = request_body(JevSource::TypeSafe, &long);
        assert_eq!(
            body["state"].as_str().unwrap(),
            format!("...{}", "é".repeat(STATE_CHARS))
        );
    }
}
