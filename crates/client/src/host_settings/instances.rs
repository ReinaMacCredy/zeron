use std::collections::BTreeMap;

use zeron_proto::HarnessId;
use zeron_proto::account_usage::login_options;
use zeron_proto::provider_instance::{
    ACCENTS, AcpPreparedAgent, AcpRegistryAgent, AcpSource, DRIVERS, HomeDir, InstanceConfig,
    ProviderInstance, ProviderInstancesSnapshot, acp_source_summary, binary_placeholder,
    derive_instance_id, distribution_name, driver_name, has_own_login, home_field, next_numbered,
    sign_in_label, uses_registry_agent,
};
use zeron_rpc::methods;

use super::{ActionResult, Page, Row, RowAction, RowKind, Section, signin};
use crate::client::Client;
use crate::error::{ClientError, Result};

fn wire(driver: HarnessId) -> String {
    serde_json::to_value(driver)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

async fn list(client: &Client, device_id: &str) -> Result<Vec<ProviderInstance>> {
    Ok(client
        .host_call::<ProviderInstancesSnapshot>(
            device_id,
            methods::LIST_PROVIDER_INSTANCES,
            serde_json::json!({}),
        )
        .await?
        .instances)
}

async fn upsert(client: &Client, device_id: &str, instance: &ProviderInstance) -> Result<()> {
    client
        .inner
        .host_rpc(
            device_id,
            methods::UPSERT_PROVIDER_INSTANCE,
            serde_json::json!({ "instance": instance }),
        )
        .await?;
    Ok(())
}

fn taken(instances: &[ProviderInstance]) -> impl Fn(&str) -> bool + '_ {
    move |id: &str| instances.iter().any(|i| i.id.eq_ignore_ascii_case(id))
}

async fn opened(client: &Client, device_id: &str, id: &str) -> Result<ActionResult> {
    let page_id = format!("instance:{id}");
    Ok(ActionResult {
        page: Some(page(client, device_id, &page_id).await?),
        page_id: Some(page_id),
        open_url: None,
    })
}

fn split(page: &str) -> Option<(&str, Option<&str>)> {
    let rest = page.strip_prefix("instance:")?;
    Some(match rest.split_once(':') {
        Some((id, sub)) => (id, Some(sub)),
        None => (rest, None),
    })
}

pub(super) async fn page(client: &Client, device_id: &str, page: &str) -> Result<Page> {
    if page == "add-provider" {
        return Ok(add_page());
    }
    if let Some(query) = page.strip_prefix("acp-search") {
        let query = query.strip_prefix(':').unwrap_or_default();
        let (results, instances) = futures::join!(
            client.host_call::<Vec<AcpRegistryAgent>>(
                device_id,
                methods::SEARCH_ACP_REGISTRY,
                serde_json::json!({ "query": query }),
            ),
            list(client, device_id),
        );
        return Ok(search_page(query, results, &instances.unwrap_or_default()));
    }
    let (id, sub) = split(page).ok_or_else(|| ClientError::NotFound(page.to_owned()))?;
    let instances = list(client, device_id).await?;
    let instance = instances
        .iter()
        .find(|i| i.id == id)
        .ok_or_else(|| ClientError::NotFound(id.to_owned()))?;
    match sub {
        None => Ok(edit_page(instance)),
        Some("accent") => Ok(accent_page(instance)),
        Some(other) => Err(ClientError::NotFound(other.to_owned())),
    }
}

pub(super) async fn act(
    client: &Client,
    device_id: &str,
    page: &str,
    action: &str,
    value: Option<String>,
) -> Result<ActionResult> {
    if page == "add-provider" {
        return add(client, device_id, action, value).await;
    }
    if page.starts_with("acp-search") {
        if action == "query" {
            let page_id = format!("acp-search:{}", value.unwrap_or_default().trim());
            return Ok(ActionResult {
                page: Some(self::page(client, device_id, &page_id).await?),
                page_id: Some(page_id),
                open_url: None,
            });
        }
        let agent = action
            .strip_prefix("prepare:")
            .ok_or_else(|| ClientError::InvalidArgument(action.to_owned()))?;
        return prepare(client, device_id, page, agent).await;
    }
    let (id, _) = split(page).ok_or_else(|| ClientError::InvalidArgument(page.to_owned()))?;
    let instances = list(client, device_id).await?;
    let mut instance = instances
        .iter()
        .find(|i| i.id == id)
        .cloned()
        .ok_or_else(|| ClientError::NotFound(id.to_owned()))?;
    match action {
        "delete" => {
            client
                .inner
                .host_rpc(
                    device_id,
                    methods::DELETE_PROVIDER_INSTANCE,
                    serde_json::json!({ "instanceId": id }),
                )
                .await?;
            return Ok(ActionResult::default());
        }
        "signin" => {
            if let InstanceConfig::Native { home_dir, .. } = &mut instance.config {
                *home_dir = Some(HomeDir::Managed);
            }
            upsert(client, device_id, &instance).await?;
            let provider = login_options(instance.driver)
                .first()
                .and_then(|option| option.provider);
            let start = match provider {
                Some(provider) => format!("start:{provider}"),
                None => "start".to_owned(),
            };
            let signin = format!("signin:{}/{}", wire(instance.driver), instance.id);
            return signin::act(client, device_id, &signin, &start, None).await;
        }
        _ => {}
    }
    edit(&mut instance, action, value)?;
    upsert(client, device_id, &instance).await?;
    Ok(ActionResult {
        page: Some(self::page(client, device_id, page).await?),
        page_id: None,
        open_url: None,
    })
}

async fn add(
    client: &Client,
    device_id: &str,
    action: &str,
    value: Option<String>,
) -> Result<ActionResult> {
    let instances = list(client, device_id).await?;
    let instance = if action == "local" {
        let line = value.unwrap_or_default();
        let mut words = line.split_whitespace().map(str::to_owned);
        let command = words
            .next()
            .ok_or_else(|| ClientError::InvalidArgument("Executable is required.".into()))?;
        ProviderInstance {
            id: derive_instance_id(HarnessId::Acp, "Local ACP", taken(&instances))
                .unwrap_or_default(),
            driver: HarnessId::Acp,
            label: "Local ACP".into(),
            accent: None,
            config: InstanceConfig::Acp {
                source: AcpSource::Local {
                    command,
                    args: words.collect(),
                },
                env: BTreeMap::new(),
                icon_path: None,
            },
        }
    } else {
        let driver = action
            .strip_prefix("add:")
            .and_then(|key| {
                serde_json::from_value::<HarnessId>(serde_json::Value::String(key.into())).ok()
            })
            .filter(|driver| DRIVERS.contains(driver))
            .ok_or_else(|| ClientError::InvalidArgument(action.to_owned()))?;
        let (label, id) = next_numbered(driver, driver_name(driver), taken(&instances));
        ProviderInstance {
            id,
            driver,
            label,
            accent: None,
            config: InstanceConfig::default(),
        }
    };
    upsert(client, device_id, &instance).await?;
    opened(client, device_id, &instance.id).await
}

async fn prepare(
    client: &Client,
    device_id: &str,
    page: &str,
    agent: &str,
) -> Result<ActionResult> {
    let query = page.strip_prefix("acp-search:").unwrap_or_default();
    let results: Vec<AcpRegistryAgent> = client
        .host_call(
            device_id,
            methods::SEARCH_ACP_REGISTRY,
            serde_json::json!({ "query": query }),
        )
        .await?;
    let unverified = results.iter().any(|a| a.id == agent && a.unverified);
    let prepared: AcpPreparedAgent = client
        .host_call(
            device_id,
            methods::PREPARE_ACP_AGENT,
            serde_json::json!({ "agentId": agent, "allowUnverified": unverified }),
        )
        .await?;
    let instances = list(client, device_id).await?;
    let label = prepared.display_name.clone();
    let instance = ProviderInstance {
        id: derive_instance_id(HarnessId::Acp, &label, taken(&instances))
            .or_else(|| derive_instance_id(HarnessId::Acp, "custom", taken(&instances)))
            .unwrap_or_default(),
        driver: HarnessId::Acp,
        label,
        accent: None,
        config: InstanceConfig::Acp {
            source: AcpSource::Registry {
                agent_id: prepared.agent_id,
                version: prepared.version,
                distribution: prepared.distribution,
            },
            env: BTreeMap::new(),
            icon_path: prepared.icon_path,
        },
    };
    upsert(client, device_id, &instance).await?;
    opened(client, device_id, &instance.id).await
}

fn edit(instance: &mut ProviderInstance, action: &str, value: Option<String>) -> Result<()> {
    let text = value.unwrap_or_default();
    let trimmed = text.trim().to_owned();
    let words = || trimmed.split_whitespace().map(str::to_owned).collect();
    if action == "label" {
        instance.label = if trimmed.is_empty() {
            driver_name(instance.driver).to_owned()
        } else {
            trimmed
        };
        return Ok(());
    }
    if let Some(accent) = action.strip_prefix("accent:") {
        instance.accent = (!accent.is_empty()).then(|| accent.to_owned());
        return Ok(());
    }
    let env = match &mut instance.config {
        InstanceConfig::Native { env, .. } | InstanceConfig::Acp { env, .. } => env,
    };
    if action == "env-add" {
        let (key, value) = trimmed
            .split_once('=')
            .map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned()))
            .filter(|(k, _)| !k.is_empty())
            .ok_or_else(|| ClientError::InvalidArgument("Use KEY=value".into()))?;
        env.insert(key, value);
        return Ok(());
    }
    if let Some(key) = action.strip_prefix("env:") {
        env.insert(key.to_owned(), text);
        return Ok(());
    }
    if let Some(key) = action.strip_prefix("remove-env:") {
        env.remove(key);
        return Ok(());
    }
    match (&mut instance.config, action) {
        (InstanceConfig::Native { executable, .. }, "executable") => {
            *executable = (!trimmed.is_empty()).then_some(trimmed);
        }
        (InstanceConfig::Native { home_dir, .. }, "home") => {
            *home_dir = match home_dir {
                _ if !trimmed.is_empty() => Some(HomeDir::Custom { path: trimmed }),
                Some(HomeDir::Managed) => Some(HomeDir::Managed),
                _ => None,
            };
        }
        (InstanceConfig::Native { launch_args, .. }, "args") => *launch_args = words(),
        (
            InstanceConfig::Acp {
                source: AcpSource::Local { command, .. },
                ..
            },
            "command",
        ) => {
            if trimmed.is_empty() {
                return Err(ClientError::InvalidArgument(
                    "Executable is required.".into(),
                ));
            }
            *command = trimmed;
        }
        (
            InstanceConfig::Acp {
                source: AcpSource::Local { args, .. },
                ..
            },
            "args",
        ) => *args = words(),
        _ => return Err(ClientError::InvalidArgument(action.to_owned())),
    }
    Ok(())
}

fn add_page() -> Page {
    let drivers = DRIVERS
        .iter()
        .map(|driver| {
            Row::new(
                format!("add:{}", wire(*driver)),
                driver_name(*driver),
                RowKind::Action { destructive: false },
            )
        })
        .collect();
    Page {
        title: "Add provider".into(),
        sections: vec![
            Section {
                title: Some("Provider".into()),
                footer: Some(
                    "Adds a numbered instance, like Codex 2. Rename it, give it its own sign-in \
                     or point it at another binary next."
                        .into(),
                ),
                rows: drivers,
            },
            Section {
                title: Some("ACP".into()),
                footer: Some(
                    "Install an agent from the ACP Registry, or run any command that speaks ACP."
                        .into(),
                ),
                rows: vec![
                    Row::link("acp-search", "ACP Registry", None),
                    Row::new(
                        "local",
                        "Local ACP command",
                        RowKind::Text {
                            multiline: false,
                            placeholder: Some("my-agent --acp".into()),
                        },
                    ),
                ],
            },
        ],
    }
}

fn search_page(
    query: &str,
    results: Result<Vec<AcpRegistryAgent>>,
    instances: &[ProviderInstance],
) -> Page {
    let search = Row::new(
        "query",
        "Search",
        RowKind::Text {
            multiline: false,
            placeholder: Some("Search the ACP Registry".into()),
        },
    )
    .value(Some(query.to_owned()));
    let mut rows = match results {
        Err(error) => vec![
            Row::new("error", "Couldn't search the registry", RowKind::Info)
                .subtitle(Some(error.to_string())),
        ],
        Ok(agents) if agents.is_empty() => {
            vec![Row::new("empty", "No agents found", RowKind::Info)]
        }
        Ok(agents) => agents
            .iter()
            .map(|agent| {
                let mut detail = vec![format!("v{}", agent.version)];
                if let Some(distribution) = agent.distributions.first() {
                    detail.push(distribution_name(*distribution).to_owned());
                }
                if !agent.description.is_empty() {
                    detail.push(agent.description.clone());
                }
                let added = uses_registry_agent(instances, &agent.id);
                let id = format!("prepare:{}", agent.id);
                let mut row = Row::new(
                    &id,
                    agent.name.clone(),
                    RowKind::Action { destructive: false },
                )
                .subtitle(Some(detail.join(" · ")))
                .value(if added {
                    Some("Added".to_owned())
                } else if agent.installed {
                    Some("Installed".to_owned())
                } else {
                    None
                });
                if agent.unverified {
                    row.actions.push(RowAction {
                        id,
                        title: "Add anyway".into(),
                        destructive: false,
                        confirm: Some(format!(
                            "The ACP Registry publishes no checksum for {}'s download, so Zeron \
                             cannot check it. Choose Add anyway to install it unchecked.",
                            agent.name
                        )),
                    });
                }
                row
            })
            .collect(),
    };
    if !query.is_empty() && !rows.iter().any(|row| row.id == format!("prepare:{query}")) {
        rows.push(
            Row::new(
                format!("prepare:{query}"),
                format!("Add \u{201c}{query}\u{201d} by ID"),
                RowKind::Action { destructive: false },
            )
            .subtitle(Some("For an agent the search does not list.".into())),
        );
    }
    Page {
        title: "ACP Registry".into(),
        sections: vec![
            Section {
                rows: vec![search],
                ..Section::default()
            },
            Section {
                title: Some("Agents".into()),
                footer: Some("Adding an agent installs it on the Mac.".into()),
                rows,
            },
        ],
    }
}

fn text(id: &str, title: &str, value: &str, placeholder: &str) -> Row {
    Row::new(
        id,
        title,
        RowKind::Text {
            multiline: false,
            placeholder: Some(placeholder.into()),
        },
    )
    .value(Some(value.to_owned()))
}

fn accent_name(accent: Option<&str>) -> String {
    match accent {
        None => "None".into(),
        Some(hex) => ACCENTS
            .iter()
            .find(|(candidate, _)| *candidate == hex)
            .map_or_else(|| hex.to_owned(), |(_, name)| (*name).to_owned()),
    }
}

fn edit_page(instance: &ProviderInstance) -> Page {
    let name = driver_name(instance.driver);
    let mut sections = vec![Section {
        title: Some("Identity".into()),
        footer: None,
        rows: vec![
            text("label", "Name", &instance.label, name),
            Row::new("id", "Instance ID", RowKind::Info).value(Some(instance.id.clone())),
            Row::new(
                "accent",
                "Accent",
                RowKind::Link {
                    page: format!("instance:{}:accent", instance.id),
                },
            )
            .value(Some(accent_name(instance.accent.as_deref()))),
        ],
    }];
    let env = match &instance.config {
        InstanceConfig::Native {
            executable,
            home_dir,
            launch_args,
            env,
        } => {
            if instance.driver == HarnessId::Cursor {
                sections.push(Section {
                    title: Some("Config".into()),
                    footer: Some(
                        "Cursor instances run on Zeron's managed Cursor SDK and take only \
                         environment variables."
                            .into(),
                    ),
                    rows: Vec::new(),
                });
            } else {
                let mut rows = vec![text(
                    "executable",
                    "Binary path",
                    executable.as_deref().unwrap_or_default(),
                    binary_placeholder(instance.driver),
                )];
                if let Some((title, placeholder)) = home_field(instance.driver) {
                    let (value, placeholder) = match home_dir {
                        Some(HomeDir::Custom { path }) => (path.as_str(), placeholder),
                        Some(HomeDir::Managed) => ("", "Managed by Zeron"),
                        None => ("", placeholder),
                    };
                    rows.push(text("home", title, value, placeholder));
                }
                rows.push(text(
                    "args",
                    "Launch arguments",
                    &launch_args.join(" "),
                    "None",
                ));
                sections.push(Section {
                    title: Some("Config".into()),
                    footer: Some(format!(
                        "Binary path runs {name} from another install. Launch arguments are \
                         added on session start."
                    )),
                    rows,
                });
            }
            env
        }
        InstanceConfig::Acp { source, env, .. } => {
            let rows = match source {
                AcpSource::Registry { .. } => vec![
                    Row::new("source", "ACP Registry", RowKind::Info)
                        .subtitle(acp_source_summary(instance)),
                ],
                AcpSource::Local { command, args } => vec![
                    text("command", "Executable", command, "my-agent"),
                    text("args", "Arguments", &args.join(" "), "None"),
                ],
            };
            sections.push(Section {
                title: Some("Source".into()),
                footer: None,
                rows,
            });
            env
        }
    };
    let mut env_rows: Vec<Row> = env
        .iter()
        .map(|(key, value)| {
            let mut row = text(&format!("env:{key}"), key, value, "Empty");
            row.actions.push(RowAction {
                id: format!("remove-env:{key}"),
                title: "Remove".into(),
                destructive: true,
                confirm: None,
            });
            row
        })
        .collect();
    env_rows.push(text("env-add", "Add variable", "", "KEY=value"));
    sections.push(Section {
        title: Some("Environment".into()),
        footer: Some("Set for this instance only. Swipe a variable to remove it.".into()),
        rows: env_rows,
    });
    let signin = format!("signin:{}/{}", wire(instance.driver), instance.id);
    let account = if instance.driver == HarnessId::Acp || has_own_login(instance) {
        Some((
            Row::link(&signin, "Sign in", None),
            "This instance keeps its own sign-in, separate from the default login.".to_owned(),
        ))
    } else {
        sign_in_label(instance.driver).map(|label| {
            (
                Row::new("signin", label, RowKind::Action { destructive: false }),
                format!(
                    "Gives this instance its own sign-in, separate from the default {name} login."
                ),
            )
        })
    };
    if let Some((row, footer)) = account {
        sections.push(Section {
            title: Some("Account".into()),
            footer: Some(footer),
            rows: vec![row],
        });
    }
    let mut delete = Row::new(
        "delete",
        "Delete instance",
        RowKind::Action { destructive: true },
    );
    delete.actions.push(RowAction {
        id: "delete".into(),
        title: "Delete".into(),
        destructive: true,
        confirm: Some(format!("Delete {}?", instance.label)),
    });
    sections.push(Section {
        rows: vec![delete],
        ..Section::default()
    });
    Page {
        title: instance.label.clone(),
        sections,
    }
}

fn accent_page(instance: &ProviderInstance) -> Page {
    let mut rows = vec![Row::new(
        "accent:",
        "None",
        RowKind::Check {
            on: instance.accent.is_none(),
        },
    )];
    rows.extend(ACCENTS.iter().map(|(hex, name)| {
        Row::new(
            format!("accent:{hex}"),
            *name,
            RowKind::Check {
                on: instance.accent.as_deref() == Some(*hex),
            },
        )
    }));
    Page {
        title: "Accent".into(),
        sections: vec![Section {
            footer: Some("Tints this instance's chip in the model picker.".into()),
            rows,
            ..Section::default()
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codex() -> ProviderInstance {
        ProviderInstance {
            id: "codex_2".into(),
            driver: HarnessId::Codex,
            label: "Codex 2".into(),
            accent: None,
            config: InstanceConfig::Native {
                executable: None,
                home_dir: Some(HomeDir::Managed),
                launch_args: Vec::new(),
                env: BTreeMap::new(),
            },
        }
    }

    #[test]
    fn edits_write_the_instance_and_a_cleared_home_keeps_the_managed_one() {
        let mut instance = codex();
        edit(&mut instance, "label", Some("  ".into())).unwrap();
        assert_eq!(instance.label, "Codex");
        edit(&mut instance, "home", Some("".into())).unwrap();
        edit(&mut instance, "args", Some("--a  --b".into())).unwrap();
        edit(
            &mut instance,
            "env-add",
            Some("OPENAI_BASE_URL = http://x".into()),
        )
        .unwrap();
        edit(&mut instance, "accent:#3e9cf0", None).unwrap();
        assert!(edit(&mut instance, "env-add", Some("novalue".into())).is_err());
        assert!(edit(&mut instance, "command", Some("x".into())).is_err());
        let InstanceConfig::Native {
            home_dir,
            launch_args,
            env,
            ..
        } = &instance.config
        else {
            panic!("native");
        };
        assert_eq!(home_dir, &Some(HomeDir::Managed));
        assert_eq!(launch_args, &["--a", "--b"]);
        assert_eq!(env["OPENAI_BASE_URL"], "http://x");
        edit(&mut instance, "home", Some("/tmp/h".into())).unwrap();
        assert!(matches!(
            &instance.config,
            InstanceConfig::Native { home_dir: Some(HomeDir::Custom { path }), .. } if path == "/tmp/h"
        ));
        assert_eq!(accent_name(instance.accent.as_deref()), "Blue");
    }

    #[test]
    fn an_instance_with_its_own_home_links_to_sign_in_and_others_offer_one() {
        let own = edit_page(&codex());
        let ids: Vec<&str> = own
            .sections
            .iter()
            .flat_map(|s| &s.rows)
            .map(|r| r.id.as_str())
            .collect();
        assert!(ids.contains(&"signin:codex/codex_2"));
        let mut shared = codex();
        shared.config = InstanceConfig::default();
        let ids: Vec<String> = edit_page(&shared)
            .sections
            .iter()
            .flat_map(|s| &s.rows)
            .map(|r| r.id.clone())
            .collect();
        assert!(ids.contains(&"signin".to_owned()));
        assert!(ids.contains(&"home".to_owned()));
    }
}
