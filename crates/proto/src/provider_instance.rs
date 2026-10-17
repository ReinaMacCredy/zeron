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
