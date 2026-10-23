//! Harness + model catalogs. The phone mirrors its run device's live catalog
//! (`ListHarnesses` / `ListModels` over the relay) and falls back to these
//! curated statics (ports of crates/harness's catalogs via the legacy
//! `HarnessCatalog.swift`). Ids are wire strings (`claude-code`, `xhigh`) so a
//! newer host's harness never fails to decode here.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessInfo {
    /// Wire id (`claude-code`).
    pub id: String,
    /// Display name.
    #[serde(alias = "name")]
    pub label: String,
    #[serde(default)]
    pub supports_steering: Option<bool>,
    /// `step-boundary` / `turn-boundary`.
    #[serde(default)]
    pub steering_mode: Option<String>,
    #[serde(default)]
    pub reasoning_levels: Vec<String>,
    /// CLI present on the listing device (absent on old engines ⇒ true).
    #[serde(default = "default_true")]
    pub installed: bool,
    /// Offered by the listing device (Settings → Providers). `None` ⇒ legacy.
    #[serde(default)]
    pub enabled: Option<bool>,
}

fn default_true() -> bool {
    true
}

impl HarnessInfo {
    fn fallback(id: &str, label: &str) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            supports_steering: None,
            steering_mode: None,
            reasoning_levels: Vec::new(),
            installed: true,
            enabled: None,
        }
    }

    /// Mid-turn steering is available (known only from a live catalog).
    pub fn mid_turn_steering(&self) -> Option<bool> {
        Some(self.supports_steering? && self.steering_mode.as_deref()? == "step-boundary")
    }

    /// Listed by the device and usable (installed, not disabled).
    pub fn offered(&self) -> bool {
        self.installed && self.enabled != Some(false)
    }
}

pub use zeron_proto::{ModelOption, ModelOptionChoice};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Lowercase wire values (`low`…`ultrathink`); empty = no effort ladder.
    #[serde(default)]
    pub reasoning_levels: Vec<String>,
    #[serde(default)]
    pub options: Vec<ModelOption>,
    #[serde(skip)]
    pub is_new: bool,
}

pub fn decode_models(value: serde_json::Value) -> serde_json::Result<Vec<ModelInfo>> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Reply {
        Catalog {
            models: Vec<ModelInfo>,
            #[serde(default, rename = "newIds")]
            new_ids: Vec<String>,
        },
        List(Vec<ModelInfo>),
    }
    Ok(match serde_json::from_value(value)? {
        Reply::List(models) => models,
        Reply::Catalog {
            mut models,
            new_ids,
        } => {
            for model in &mut models {
                model.is_new = new_ids.contains(&model.id);
            }
            models
        }
    })
}

fn model(
    id: &str,
    label: &str,
    description: &str,
    ladder: &[&str],
    options: Vec<ModelOption>,
) -> ModelInfo {
    ModelInfo {
        id: id.into(),
        label: label.into(),
        description: Some(description.into()),
        reasoning_levels: ladder.iter().map(|s| (*s).to_owned()).collect(),
        options,
        is_new: false,
    }
}

fn context_window(default: &str) -> ModelOption {
    ModelOption {
        id: "contextWindow".into(),
        label: "Context Window".into(),
        choices: [("200k", "200K"), ("1m", "1M")]
            .into_iter()
            .map(|(id, label)| ModelOptionChoice {
                id: id.into(),
                label: label.into(),
            })
            .collect(),
        default_choice: default.into(),
    }
}

/// Static fallback = the engine's `default_enabled()` pair. ACP agents
/// appear only through a device's live catalog (opt-in per device).
pub fn fallback_harnesses() -> Vec<HarnessInfo> {
    vec![
        HarnessInfo::fallback("claude-code", "Claude Code"),
        HarnessInfo::fallback("codex", "Codex"),
    ]
}

/// Display name for any harness id the fleet can produce.
pub fn harness_label(id: &str) -> String {
    serde_json::from_value::<zeron_proto::HarnessId>(serde_json::Value::String(id.to_owned()))
        .map_or_else(
            |_| id.to_owned(),
            |harness| harness.display_name().to_owned(),
        )
}

/// Curated static models for a harness (first row = default).
pub fn fallback_models(harness: &str) -> Vec<ModelInfo> {
    match harness {
        "grok" => vec![model(
            "grok-4.5",
            "Grok 4.5",
            "xAI's coding model — 500k context",
            &["low", "medium", "high"],
            vec![],
        )],
        "devin" => vec![
            model(
                "swe-1-7-medium",
                "SWE-1.7 Medium",
                "Devin's default coding model",
                &[],
                vec![],
            ),
            model(
                "claude-fable-5-1-high",
                "Claude Fable 5.1 High",
                "Anthropic's frontier model through Devin",
                &[],
                vec![],
            ),
            model(
                "adaptive",
                "Adaptive",
                "Devin picks the model per request",
                &[],
                vec![],
            ),
        ],
        "hermes" => vec![
            model(
                "hermes-4-405b",
                "Hermes 4 405B",
                "Nous Research's hybrid-reasoning flagship",
                &[],
                vec![],
            ),
            model(
                "hermes-4-70b",
                "Hermes 4 70B",
                "Faster Hermes 4 — same post-training, 70B",
                &[],
                vec![],
            ),
        ],
        "pi" => vec![model(
            "default",
            "pi default",
            "Runs the model configured in pi (`pi` settings)",
            &["minimal", "low", "medium", "high", "xhigh", "max"],
            vec![],
        )],
        "opencode" => vec![
            model(
                "opencode/big-pickle",
                "Big Pickle",
                "OpenCode Zen's flagship coding model",
                &[],
                vec![],
            ),
            model(
                "opencode/mimo-v2.5-free",
                "MiMo V2.5 Free",
                "Free tier on OpenCode Zen",
                &[],
                vec![],
            ),
            model(
                "opencode/hy3-free",
                "Hy3 Free",
                "Free tier on OpenCode Zen",
                &["low", "medium", "high"],
                vec![],
            ),
        ],
        "antigravity" => vec![
            model(
                "gemini-3.7-flash",
                "Gemini 3.7 Flash",
                "Google's fast Gemini model through Antigravity",
                &["low", "medium", "high"],
                vec![],
            ),
            model(
                "gemini-3.1-pro",
                "Gemini 3.1 Pro",
                "Google's most capable Gemini model through Antigravity",
                &["low", "high"],
                vec![],
            ),
        ],
        "codex" => curated(zeron_proto::model_catalog::codex::static_models()),
        _ => curated(zeron_proto::model_catalog::claude::static_models()),
    }
}

fn curated(models: Vec<zeron_proto::Model>) -> Vec<ModelInfo> {
    serde_json::to_value(models)
        .and_then(serde_json::from_value)
        .expect("proto Model decodes as ModelInfo")
}

/// pickers.rs — High when available, then Medium, then the first level.
pub fn default_reasoning(model: &ModelInfo) -> Option<String> {
    let levels = &model.reasoning_levels;
    if levels.is_empty() {
        return None;
    }
    for preferred in ["high", "medium"] {
        if levels.iter().any(|l| l == preferred) {
            return Some(preferred.into());
        }
    }
    levels.first().cloned()
}

pub fn reasoning_label(level: &str) -> String {
    if let Ok(level) = serde_json::from_value::<zeron_proto::ReasoningLevel>(level.into()) {
        return level.label().into();
    }
    let mut chars = level.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

fn stripped_1m(id: &str) -> Option<&str> {
    id.strip_suffix("[1m]").or_else(|| id.strip_suffix("-1m"))
}

fn norm(id: &str) -> String {
    id.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase()
}

fn curated_catalog(harness: &str) -> Vec<ModelInfo> {
    if harness == "claude-code" || harness == "mock" {
        fallback_models(harness)
    } else {
        Vec::new()
    }
}

fn curated_label(id: &str, catalog: &[ModelInfo]) -> Option<String> {
    let id_norm = norm(id);
    if let Some(exact) = catalog.iter().find(|m| norm(&m.id) == id_norm) {
        return Some(exact.label.clone());
    }
    if id_norm.is_empty() || !id_norm.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    catalog
        .iter()
        .find(|m| norm(&m.id).contains(&id_norm))
        .map(|m| m.label.clone())
}

/// Clean a live `ListModels` reply: drop a `default` placeholder when real
/// rows exist, fold `[1m]`/`-1m` variants into a Context Window option, and
/// prefer curated labels (legacy `HarnessCatalog.normalize`).
pub fn normalize_models(harness: &str, models: Vec<ModelInfo>) -> Vec<ModelInfo> {
    let catalog = curated_catalog(harness);
    let ids: Vec<String> = models.iter().map(|m| m.id.clone()).collect();
    let has_real = ids.iter().any(|id| !id.eq_ignore_ascii_case("default"));
    models
        .into_iter()
        .filter_map(|mut row| {
            if has_real && row.id.eq_ignore_ascii_case("default") {
                return None;
            }
            if let Some(base) = stripped_1m(&row.id).map(str::to_owned) {
                if ids.contains(&base) {
                    return None;
                }
                row.id = base;
                if let Some(start) = row.label.rfind('(')
                    && row.label.ends_with(')')
                    && start > 0
                {
                    row.label = row.label[..start].trim().to_owned();
                }
                if !row.options.iter().any(|o| o.id == "contextWindow") {
                    row.options.push(context_window("1m"));
                }
            }
            if let Some(label) = curated_label(&row.id, &catalog) {
                row.label = label;
            }
            Some(row)
        })
        .collect()
}

/// Labels learned from hosts' live model catalogs, keyed by (harness, id):
/// harnesses without a curated list (Codex, Cursor…) still get the host's
/// human label instead of the raw model id.
static LEARNED_LABELS: std::sync::LazyLock<
    std::sync::RwLock<std::collections::HashMap<(String, String), String>>,
> = std::sync::LazyLock::new(Default::default);

pub(crate) fn learn_labels(harness: &str, models: &[ModelInfo]) {
    let Ok(mut map) = LEARNED_LABELS.write() else {
        return;
    };
    for m in models {
        if !m.label.trim().is_empty() && m.label != m.id {
            map.insert((harness.to_owned(), m.id.clone()), m.label.clone());
        }
    }
}

fn learned_label(harness: &str, model_id: &str) -> Option<String> {
    let map = LEARNED_LABELS.read().ok()?;
    map.get(&(harness.to_owned(), model_id.to_owned()))
        .or_else(|| {
            stripped_1m(model_id).and_then(|base| map.get(&(harness.to_owned(), base.to_owned())))
        })
        .cloned()
}

/// Human label for a chat's model id (chip / row subtitle).
pub fn model_label(harness: &str, model_id: &str) -> String {
    let catalog = fallback_models(harness);
    if let Some(found) = catalog
        .iter()
        .find(|m| m.id == model_id)
        .or_else(|| stripped_1m(model_id).and_then(|base| catalog.iter().find(|m| m.id == base)))
    {
        return found.label.clone();
    }
    learned_label(harness, model_id)
        .or_else(|| curated_label(model_id, &curated_catalog(harness)))
        .unwrap_or_else(|| model_id.to_owned())
}

/// Last-known live catalogs on disk (`{data_dir}/catalogs/{device}.json`):
/// pickers open instantly with the device's real list, and an unreachable
/// device still shows what it offered last time before the static fallback.
#[derive(Debug, Clone)]
pub(crate) struct DiskCatalog {
    dir: std::path::PathBuf,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct DeviceCatalog {
    #[serde(default)]
    harnesses: Option<Vec<HarnessInfo>>,
    #[serde(default)]
    models: std::collections::BTreeMap<String, Vec<ModelInfo>>,
}

fn file_safe(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

impl DiskCatalog {
    pub(crate) fn new(data_dir: &std::path::Path) -> Self {
        Self {
            dir: data_dir.join("catalogs"),
        }
    }

    fn path(&self, device_id: &str) -> std::path::PathBuf {
        self.dir.join(format!("{}.json", file_safe(device_id)))
    }

    fn load(&self, device_id: &str) -> DeviceCatalog {
        std::fs::read(self.path(device_id))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn store(&self, device_id: &str, catalog: &DeviceCatalog) {
        let write = || -> std::io::Result<()> {
            std::fs::create_dir_all(&self.dir)?;
            let path = self.path(device_id);
            let tmp = path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_vec(catalog).unwrap_or_default())?;
            std::fs::rename(tmp, path)
        };
        if let Err(err) = write() {
            tracing::debug!(error = %err, "catalog cache write failed");
        }
    }

    pub(crate) fn harnesses(&self, device_id: &str) -> Option<Vec<HarnessInfo>> {
        self.load(device_id).harnesses
    }

    pub(crate) fn put_harnesses(&self, device_id: &str, list: &[HarnessInfo]) {
        let mut catalog = self.load(device_id);
        catalog.harnesses = Some(list.to_vec());
        self.store(device_id, &catalog);
    }

    pub(crate) fn models(&self, device_id: &str, harness: &str) -> Option<Vec<ModelInfo>> {
        self.load(device_id).models.remove(harness)
    }

    /// Teach [`model_label`] every label cached on disk (client start).
    pub(crate) fn warm_labels(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(bytes) = std::fs::read(entry.path()) else {
                continue;
            };
            let Ok(catalog) = serde_json::from_slice::<DeviceCatalog>(&bytes) else {
                continue;
            };
            for (harness, models) in &catalog.models {
                learn_labels(harness, models);
            }
        }
    }

    pub(crate) fn put_models(&self, device_id: &str, harness: &str, list: &[ModelInfo]) {
        learn_labels(harness, list);
        let mut catalog = self.load(device_id);
        catalog.models.insert(harness.to_owned(), list.to_vec());
        self.store(device_id, &catalog);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_resolve_curated_and_unknown_models() {
        assert_eq!(model_label("claude-code", "claude-opus-5-5"), "Opus 5.5");
        assert_eq!(
            model_label("claude-code", "claude-opus-5-5[1m]"),
            "Opus 5.5"
        );
        assert_eq!(model_label("codex", "gpt-5.6-terra"), "GPT-5.6-Terra");
        assert!(
            fallback_models("claude-code")[0]
                .reasoning_levels
                .contains(&"ultrathink".to_owned())
        );
        assert_eq!(fallback_models("codex")[0].options[0].id, "serviceTier");
        assert_eq!(model_label("codex", "some-new-model"), "some-new-model");
        assert_eq!(harness_label("opencode"), "OpenCode");
    }

    #[test]
    fn live_catalog_labels_replace_raw_ids() {
        assert_eq!(
            model_label("cursor-test", "sonnet-9-thinking"),
            "sonnet-9-thinking"
        );
        learn_labels(
            "cursor-test",
            &[model(
                "sonnet-9-thinking",
                "Sonnet 9 Thinking",
                "x",
                &[],
                vec![],
            )],
        );
        assert_eq!(
            model_label("cursor-test", "sonnet-9-thinking"),
            "Sonnet 9 Thinking"
        );
    }

    #[test]
    fn list_models_reply_decodes_catalog_object_and_bare_array() {
        let row = serde_json::json!({"id": "claude-haiku-5-5", "label": "Haiku 5.5", "reasoningLevels": ["high"]});
        let object = serde_json::json!({"models": [row, {"id": "claude-opus-5-5", "label": "Opus 5.5"}], "newIds": ["claude-haiku-5-5"], "announceIds": []});
        let models = decode_models(object).unwrap();
        assert_eq!(models.len(), 2);
        assert!(models[0].is_new);
        assert!(!models[1].is_new);
        let bare = decode_models(serde_json::json!([row])).unwrap();
        assert_eq!(bare[0].id, "claude-haiku-5-5");
        assert!(!bare[0].is_new);
    }

    #[test]
    fn normalize_folds_1m_variants() {
        let live = vec![
            model("claude-opus-5-5[1m]", "Opus 5.5 (1M)", "x", &[], vec![]),
            model("default", "Default", "x", &[], vec![]),
        ];
        let out = normalize_models("claude-code", live);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "claude-opus-5-5");
        assert_eq!(out[0].label, "Opus 5.5");
        assert!(out[0].options.iter().any(|o| o.id == "contextWindow"));
    }

    #[test]
    fn disk_catalog_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let cache = DiskCatalog::new(dir.path());
        assert!(cache.harnesses("dev/mac").is_none());
        cache.put_harnesses("dev/mac", &fallback_harnesses());
        cache.put_models("dev/mac", "codex", &fallback_models("codex"));
        assert_eq!(cache.harnesses("dev/mac").unwrap().len(), 2);
        assert_eq!(
            cache.models("dev/mac", "codex").unwrap()[0].id,
            "gpt-6-astra"
        );
        assert!(cache.models("dev/mac", "grok").is_none());
    }
}
