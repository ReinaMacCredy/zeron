use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::HarnessId;

pub const MAX_INSTANCE_ID_CHARS: usize = 64;
const MAX_LABEL_SLUG_CHARS: usize = 48;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInstance {
    pub id: String,
    pub driver: HarnessId,
    #[serde(default)]
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    #[serde(default)]
    pub config: InstanceConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum InstanceConfig {
    Native {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        executable: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        home_dir: Option<HomeDir>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        launch_args: Vec<String>,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, String>,
    },
    Acp {
        source: AcpSource,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        env: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon_path: Option<String>,
    },
}

impl ProviderInstance {
    pub fn icon_path(&self) -> Option<&str> {
        match &self.config {
            InstanceConfig::Acp { icon_path, .. } => icon_path.as_deref(),
            InstanceConfig::Native { .. } => None,
        }
    }
}

impl Default for InstanceConfig {
    fn default() -> Self {
        InstanceConfig::Native {
            executable: None,
            home_dir: None,
            launch_args: Vec::new(),
            env: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum HomeDir {
    Custom {
        path: String,
    },
    /// Engine-owned `<data_dir>/provider-homes/<instance id>`.
    Managed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AcpSource {
    Registry {
        agent_id: String,
        #[serde(default)]
        version: String,
        distribution: AcpDistribution,
    },
    Local {
        command: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AcpDistribution {
    Npx,
    Uvx,
    Binary,
}

/// ACP's `type`: only "agent" (the default) signs in through `authenticate`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpAuthMethod {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
}

impl AcpAuthMethod {
    pub fn signs_in_app(&self) -> bool {
        self.kind == "agent"
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpRegistryAgent {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    pub distributions: Vec<AcpDistribution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_path: Option<String>,
    #[serde(default)]
    pub installed: bool,
    /// Adding it downloads a binary the registry publishes no checksum for.
    #[serde(default)]
    pub unverified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpPreparedAgent {
    pub agent_id: String,
    pub version: String,
    pub distribution: AcpDistribution,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_path: Option<String>,
    pub display_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInstancesSnapshot {
    pub instances: Vec<ProviderInstance>,
}

pub fn validate_instance_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("Instance ID is required.".into());
    }
    if id.len() > MAX_INSTANCE_ID_CHARS {
        return Err(format!(
            "Instance ID must be {MAX_INSTANCE_ID_CHARS} characters or fewer."
        ));
    }
    let mut chars = id.chars();
    let starts_with_letter = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    if !starts_with_letter || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        return Err(
            "Instance ID must start with a letter and use only letters, digits, '-', or '_'."
                .into(),
        );
    }
    if let Ok(harness) =
        serde_json::from_value::<HarnessId>(serde_json::Value::String(id.to_ascii_lowercase()))
    {
        return Err(format!(
            "Instance ID '{id}' is reserved for {}.",
            harness.display_name()
        ));
    }
    Ok(())
}

pub fn derive_instance_id(
    driver: HarnessId,
    label: &str,
    taken: impl Fn(&str) -> bool,
) -> Option<String> {
    let slug = label_slug(label);
    if slug.is_empty() {
        return None;
    }
    let driver = serde_json::to_value(driver).ok()?;
    let base = format!("{}_{slug}", driver.as_str()?);
    if !taken(&base) {
        return Some(base);
    }
    (2..).map(|n| format!("{base}_{n}")).find(|id| !taken(id))
}

fn label_slug(label: &str) -> String {
    let mut slug = String::new();
    let mut gap = false;
    for c in label.trim().chars().map(|c| c.to_ascii_lowercase()) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if gap && !slug.is_empty() {
                slug.push('_');
            }
            gap = false;
            slug.push(c);
        } else {
            gap = true;
        }
    }
    slug.truncate(MAX_LABEL_SLUG_CHARS);
    slug.trim_end_matches('_').to_string()
}

pub const DRIVERS: [HarnessId; 9] = [
    HarnessId::ClaudeCode,
    HarnessId::Codex,
    HarnessId::Cursor,
    HarnessId::Devin,
    HarnessId::Grok,
    HarnessId::Hermes,
    HarnessId::Opencode,
    HarnessId::Pi,
    HarnessId::Antigravity,
];

pub const ACCENTS: [(&str, &str); 7] = [
    ("#e5484d", "Red"),
    ("#f5a524", "Amber"),
    ("#46a758", "Green"),
    ("#3e9cf0", "Blue"),
    ("#8e4ec6", "Purple"),
    ("#d6409f", "Pink"),
    ("#8b8d98", "Gray"),
];

pub fn driver_name(driver: HarnessId) -> &'static str {
    match driver {
        HarnessId::ClaudeCode => "Claude",
        HarnessId::Codex => "Codex",
        HarnessId::Cursor => "Cursor",
        HarnessId::Devin => "Devin",
        HarnessId::Grok => "Grok",
        HarnessId::Hermes => "Hermes",
        HarnessId::Opencode => "OpenCode",
        HarnessId::Pi => "Pi",
        HarnessId::Antigravity => "Antigravity",
        HarnessId::Acp => "ACP",
        HarnessId::Muse => "Muse Code",
        HarnessId::Mock => "Mock",
    }
}

pub fn cli_name(harness: HarnessId) -> &'static str {
    match harness {
        HarnessId::ClaudeCode => "claude",
        HarnessId::Codex => "codex",
        HarnessId::Cursor => "cursor-agent",
        HarnessId::Devin => "devin",
        HarnessId::Grok => "grok",
        HarnessId::Hermes => "hermes",
        HarnessId::Pi => "pi",
        HarnessId::Opencode => "opencode",
        HarnessId::Antigravity => "Antigravity",
        HarnessId::Acp => "acp",
        HarnessId::Muse => "muse",
        HarnessId::Mock => "mock",
    }
}

pub fn binary_placeholder(driver: HarnessId) -> &'static str {
    match driver {
        HarnessId::Antigravity => "antigravity-acp",
        _ => cli_name(driver),
    }
}

pub fn home_field(driver: HarnessId) -> Option<(&'static str, &'static str)> {
    match driver {
        HarnessId::ClaudeCode => Some(("CLAUDE_CONFIG_DIR path", "~/.claude")),
        HarnessId::Codex => Some(("CODEX_HOME path", "~/.codex")),
        HarnessId::Pi => Some(("PI_CODING_AGENT_DIR path", "~/.pi/agent")),
        _ => None,
    }
}

pub fn sign_in_label(driver: HarnessId) -> Option<&'static str> {
    match driver {
        HarnessId::Codex | HarnessId::Pi => Some("Continue with ChatGPT"),
        HarnessId::ClaudeCode => Some("Sign in with Claude"),
        _ => None,
    }
}

pub fn distribution_name(distribution: AcpDistribution) -> &'static str {
    match distribution {
        AcpDistribution::Npx => "npx",
        AcpDistribution::Uvx => "uvx",
        AcpDistribution::Binary => "binary",
    }
}

pub fn acp_source_summary(instance: &ProviderInstance) -> Option<String> {
    match &instance.config {
        InstanceConfig::Acp {
            source:
                AcpSource::Registry {
                    agent_id,
                    version,
                    distribution,
                },
            ..
        } => Some(format!(
            "{agent_id} · v{version} · {}",
            distribution_name(*distribution)
        )),
        InstanceConfig::Acp {
            source: AcpSource::Local { command, .. },
            ..
        } => Some(command.clone()),
        InstanceConfig::Native { .. } => None,
    }
}

pub fn uses_registry_agent(instances: &[ProviderInstance], id: &str) -> bool {
    instances.iter().any(|instance| {
        matches!(
            &instance.config,
            InstanceConfig::Acp {
                source: AcpSource::Registry { agent_id, .. },
                ..
            } if agent_id == id
        )
    })
}

pub fn has_own_login(instance: &ProviderInstance) -> bool {
    matches!(
        instance.driver,
        HarnessId::ClaudeCode | HarnessId::Codex | HarnessId::Pi
    ) && matches!(
        &instance.config,
        InstanceConfig::Native {
            home_dir: Some(_),
            ..
        }
    )
}

fn is_reserved(id: &str) -> bool {
    serde_json::from_value::<HarnessId>(serde_json::Value::String(id.to_ascii_lowercase())).is_ok()
}

fn wire_slug(driver: HarnessId) -> String {
    serde_json::to_value(driver)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// T3 copy: a bare driver slug is the default instance, so it reads as taken.
pub fn instance_id_error(id: &str, taken: impl Fn(&str) -> bool) -> Option<String> {
    let exists = || Some(format!("An instance named '{id}' already exists."));
    match validate_instance_id(id) {
        Err(_) if is_reserved(id) => exists(),
        Err(error) => Some(error),
        Ok(()) if taken(id) => exists(),
        Ok(()) => None,
    }
}

/// T3: the driver's own name derives from the bare slug, which is always taken.
pub fn default_instance_id(
    driver: HarnessId,
    driver_name: &str,
    label: &str,
    taken: impl Fn(&str) -> bool,
) -> String {
    let unavailable = |id: &str| taken(id) || is_reserved(id);
    if label.trim().is_empty() || label.trim() == driver_name {
        let base = wire_slug(driver);
        return (2..)
            .map(|n| format!("{base}_{n}"))
            .find(|id| !unavailable(id))
            .unwrap_or_default();
    }
    derive_instance_id(driver, label, unavailable).unwrap_or_default()
}

pub fn next_numbered(
    driver: HarnessId,
    driver_name: &str,
    taken: impl Fn(&str) -> bool,
) -> (String, String) {
    let base = wire_slug(driver);
    let n = (2..)
        .find(|n| {
            let id = format!("{base}_{n}");
            !taken(&id) && !is_reserved(&id)
        })
        .unwrap_or(2);
    (format!("{driver_name} {n}"), format!("{base}_{n}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_ids_follow_the_slug_rules_and_spare_harness_names() {
        for ok in ["codex_work", "Work-2", "a", &"a".repeat(64)] {
            assert_eq!(validate_instance_id(ok), Ok(()), "{ok}");
        }
        for bad in [
            "",
            "2work",
            "_work",
            "work space",
            "wörk",
            &"a".repeat(65),
            "codex",
            "claude-code",
            "acp",
            "Mock",
        ] {
            assert!(validate_instance_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn derived_ids_prefix_the_driver_and_suffix_collisions() {
        let none = |_: &str| false;
        assert_eq!(
            derive_instance_id(HarnessId::Codex, "  Work / Personal!! ", none).as_deref(),
            Some("codex_work_personal")
        );
        assert_eq!(
            derive_instance_id(HarnessId::ClaudeCode, "Work", none).as_deref(),
            Some("claude-code_work")
        );
        let taken = |id: &str| matches!(id, "codex_work" | "codex_work_2");
        assert_eq!(
            derive_instance_id(HarnessId::Codex, "Work", taken).as_deref(),
            Some("codex_work_3")
        );
        assert_eq!(derive_instance_id(HarnessId::Codex, "日本 —", none), None);
        let long = derive_instance_id(HarnessId::Antigravity, &"x".repeat(80), none).unwrap();
        assert_eq!(long.len(), "antigravity_".len() + MAX_LABEL_SLUG_CHARS);
        assert_eq!(validate_instance_id(&long), Ok(()));
    }

    #[test]
    fn ids_follow_t3_derivation_and_copy() {
        let none = |_: &str| false;
        assert_eq!(
            default_instance_id(HarnessId::Codex, "Codex", "Codex", none),
            "codex_2"
        );
        assert_eq!(
            default_instance_id(HarnessId::Codex, "Codex", "", |id| id == "codex_2"),
            "codex_3"
        );
        assert_eq!(
            default_instance_id(HarnessId::Codex, "Codex", "Codex Work", none),
            "codex_codex_work"
        );
        assert_eq!(
            default_instance_id(HarnessId::Codex, "Codex", "Work", |id| id == "codex_work"),
            "codex_work_2"
        );
        assert_eq!(
            default_instance_id(HarnessId::Codex, "Codex", "日本", none),
            ""
        );
        assert_eq!(
            instance_id_error("codex", none).as_deref(),
            Some("An instance named 'codex' already exists.")
        );
        assert_eq!(
            instance_id_error("codex_work", |id| id == "codex_work").as_deref(),
            Some("An instance named 'codex_work' already exists.")
        );
        assert_eq!(
            instance_id_error("", none).as_deref(),
            Some("Instance ID is required.")
        );
        assert!(instance_id_error("2work", none).is_some());
        assert_eq!(instance_id_error("codex_work", none), None);
        assert_eq!(
            next_numbered(HarnessId::Codex, "Codex", |id| id == "codex_2"),
            ("Codex 3".to_string(), "codex_3".to_string())
        );
    }

    #[test]
    fn instances_round_trip_and_tolerate_missing_optional_fields() {
        let native = ProviderInstance {
            id: "codex_work".into(),
            driver: HarnessId::Codex,
            label: "Work".into(),
            accent: Some("#22c55e".into()),
            config: InstanceConfig::Native {
                executable: Some("/opt/codex".into()),
                home_dir: Some(HomeDir::Managed),
                launch_args: vec!["--verbose".into()],
                env: BTreeMap::from([("OPENAI_BASE_URL".into(), "http://x".into())]),
            },
        };
        let acp = ProviderInstance {
            id: "acp_gemini".into(),
            driver: HarnessId::Acp,
            label: "Gemini".into(),
            accent: None,
            config: InstanceConfig::Acp {
                source: AcpSource::Registry {
                    agent_id: "gemini".into(),
                    version: "0.1.0".into(),
                    distribution: AcpDistribution::Npx,
                },
                env: BTreeMap::new(),
                icon_path: None,
            },
        };
        let custom = HomeDir::Custom {
            path: "/tmp/home".into(),
        };
        assert_eq!(
            serde_json::to_value(&custom).unwrap(),
            serde_json::json!({"kind": "custom", "path": "/tmp/home"})
        );
        for instance in [native, acp] {
            let json = serde_json::to_string(&instance).unwrap();
            assert_eq!(
                serde_json::from_str::<ProviderInstance>(&json).unwrap(),
                instance
            );
        }
        let bare: ProviderInstance =
            serde_json::from_value(serde_json::json!({"id": "codex_x", "driver": "codex"}))
                .unwrap();
        assert_eq!(bare.config, InstanceConfig::default());
        let local: InstanceConfig = serde_json::from_value(
            serde_json::json!({"kind": "acp", "source": {"kind": "local", "command": "agent"}}),
        )
        .unwrap();
        assert!(matches!(
            local,
            InstanceConfig::Acp { source: AcpSource::Local { args, .. }, .. } if args.is_empty()
        ));
    }
}
