use zeron_proto::account_usage::{is_openable_login_url, login_options, provider_name};
use zeron_proto::{
    AcpAuthMethod, AgentAccountsSnapshot, AgentLoginMode, AgentLoginPoll, AgentLoginStart,
    AgentLoginStatus, HarnessId, ProviderInstancesSnapshot,
};
use zeron_rpc::methods;

use super::{ActionResult, Page, Row, RowKind, Section};
use crate::client::Client;
use crate::error::{ClientError, Result};

#[derive(Debug, PartialEq)]
struct Login<'a> {
    key: &'a str,
    harness: HarnessId,
    instance: Option<&'a str>,
    id: Option<&'a str>,
    paste: bool,
    on_mac: bool,
    url: &'a str,
}

fn parse(page: &str) -> Result<Login<'_>> {
    let rest = page
        .strip_prefix("signin:")
        .ok_or_else(|| ClientError::NotFound(page.to_owned()))?;
    let mut parts = rest.splitn(4, ':');
    let key = parts.next().unwrap_or_default();
    let (harness, instance) = match key.split_once('/') {
        Some((harness, instance)) => (harness, Some(instance)),
        None => (key, None),
    };
    let harness = serde_json::from_value(serde_json::Value::String(harness.to_owned()))
        .map_err(|_| ClientError::NotFound(page.to_owned()))?;
    let id = parts.next();
    let mode = parts.next().unwrap_or_default();
    Ok(Login {
        key,
        harness,
        instance,
        id,
        paste: mode == "paste",
        on_mac: mode == "mac",
        url: parts.next().unwrap_or_default(),
    })
}

impl Login<'_> {
    fn params(&self, mut params: serde_json::Value) -> serde_json::Value {
        if let Some(instance) = self.instance {
            params["instanceId"] = instance.into();
        }
        params
    }
}

fn page_id(key: &str, id: &str, mode: &str, url: &str) -> String {
    format!("signin:{key}:{id}:{mode}:{url}")
}

fn mode_of(start: &AgentLoginStart) -> &'static str {
    match (start.mode, start.callback_port) {
        (AgentLoginMode::PasteCode, _) => "paste",
        (AgentLoginMode::Browser, Some(_)) => "mac",
        (AgentLoginMode::Browser, None) => "browser",
    }
}

pub(super) async fn page(client: &Client, device_id: &str, page: &str) -> Result<Page> {
    let login = parse(page)?;
    let Some(id) = login.id else {
        return start(client, device_id, &login).await;
    };
    let poll = client
        .host_call::<AgentLoginPoll>(
            device_id,
            methods::POLL_AGENT_LOGIN,
            login.params(serde_json::json!({ "loginId": id })),
        )
        .await?;
    Ok(progress_page(&login, &poll))
}

pub(super) async fn act(
    client: &Client,
    device_id: &str,
    page: &str,
    action: &str,
    value: Option<String>,
) -> Result<ActionResult> {
    let login = parse(page)?;
    if let Some(choice) = action.strip_prefix("start") {
        let choice = choice.strip_prefix(':');
        let harness = serde_json::to_value(login.harness).unwrap_or_default();
        let params = if login.harness == HarnessId::Acp {
            serde_json::json!({ "harness": harness, "methodId": choice })
        } else {
            serde_json::json!({ "harness": harness, "provider": choice })
        };
        let start = client
            .host_call::<AgentLoginStart>(
                device_id,
                methods::START_AGENT_LOGIN,
                login.params(params),
            )
            .await?;
        let mode = mode_of(&start);
        let id = page_id(login.key, &start.login_id, mode, &start.url);
        let started = parse(&id)?;
        let pending = AgentLoginPoll {
            status: AgentLoginStatus::Pending,
            message: None,
            url: None,
            callback_port: None,
        };
        return Ok(ActionResult {
            page: Some(progress_page(&started, &pending)),
            open_url: (mode != "mac" && is_openable_login_url(&start.url))
                .then(|| start.url.clone()),
            page_id: Some(id),
        });
    }
    let id = login
        .id
        .ok_or_else(|| ClientError::InvalidArgument(action.to_owned()))?;
    let gone = ActionResult {
        page: None,
        page_id: None,
        open_url: None,
    };
    match action {
        "code" => {
            let code = value.unwrap_or_default();
            if code.trim().is_empty() {
                return Err(ClientError::InvalidArgument(
                    "Paste the authorization code".into(),
                ));
            }
            client
                .host_call::<AgentAccountsSnapshot>(
                    device_id,
                    methods::COMPLETE_AGENT_LOGIN,
                    login.params(serde_json::json!({ "loginId": id, "code": code.trim() })),
                )
                .await?;
            Ok(gone)
        }
        "cancel" => {
            client
                .inner
                .host_rpc(
                    device_id,
                    methods::CANCEL_AGENT_LOGIN,
                    login.params(serde_json::json!({ "loginId": id })),
                )
                .await?;
            Ok(gone)
        }
        "open" => Ok(ActionResult {
            page: Some(self::page(client, device_id, page).await?),
            page_id: None,
            open_url: is_openable_login_url(login.url).then(|| login.url.to_owned()),
        }),
        "check" => {
            let poll = client
                .host_call::<AgentLoginPoll>(
                    device_id,
                    methods::POLL_AGENT_LOGIN,
                    login.params(serde_json::json!({ "loginId": id })),
                )
                .await?;
            if poll.status == AgentLoginStatus::Done {
                return Ok(gone);
            }
            let url = poll.url.as_deref().filter(|u| !u.is_empty());
            let mode = if login.paste {
                "paste"
            } else if login.on_mac || poll.callback_port.is_some() {
                "mac"
            } else {
                "browser"
            };
            let next = page_id(login.key, id, mode, url.unwrap_or(login.url));
            let opened = parse(&next)?;
            Ok(ActionResult {
                page: Some(progress_page(&opened, &poll)),
                open_url: url
                    .filter(|u| mode != "mac" && is_openable_login_url(u))
                    .map(str::to_owned),
                page_id: Some(next),
            })
        }
        _ => Err(ClientError::InvalidArgument(action.to_owned())),
    }
}

async fn start(client: &Client, device_id: &str, login: &Login<'_>) -> Result<Page> {
    let Some(instance) = login.instance else {
        return Ok(start_page(login.harness, None));
    };
    let label = client
        .host_call::<ProviderInstancesSnapshot>(
            device_id,
            methods::LIST_PROVIDER_INSTANCES,
            serde_json::json!({}),
        )
        .await
        .ok()
        .and_then(|s| s.instances.into_iter().find(|i| i.id == instance))
        .map_or_else(|| instance.to_owned(), |i| i.label);
    if login.harness != HarnessId::Acp {
        return Ok(start_page(login.harness, Some(&label)));
    }
    let methods = client
        .host_call::<Vec<AcpAuthMethod>>(
            device_id,
            methods::LIST_ACP_AUTH_METHODS,
            serde_json::json!({ "instanceId": instance }),
        )
        .await?;
    Ok(acp_start_page(&label, &methods))
}

fn acp_start_page(label: &str, methods: &[AcpAuthMethod]) -> Page {
    let rows: Vec<Row> = methods
        .iter()
        .map(|method| {
            Row::new(
                format!("start:{}", method.id),
                method.name.clone(),
                if method.signs_in_app() {
                    RowKind::Action { destructive: false }
                } else {
                    RowKind::Info
                },
            )
            .subtitle(method.description.clone())
        })
        .collect();
    let in_app = methods.iter().any(AcpAuthMethod::signs_in_app);
    Page {
        title: format!("Sign in to {label}"),
        sections: vec![Section {
            title: (!rows.is_empty()).then(|| "Sign in with".into()),
            footer: Some(if in_app {
                "The sign-in page opens in this phone's browser.".into()
            } else {
                "No in-app sign-in advertised. Follow the provider's docs to finish setup.".into()
            }),
            rows,
        }],
    }
}

fn start_page(harness: HarnessId, label: Option<&str>) -> Page {
    let options = login_options(harness);
    let rows = if options.len() == 1 {
        vec![Row::new(
            "start",
            "Start sign-in",
            RowKind::Action { destructive: false },
        )]
    } else {
        options
            .iter()
            .map(|option| {
                Row::new(
                    format!("start:{}", option.provider.unwrap_or_default()),
                    option.label,
                    RowKind::Action { destructive: false },
                )
            })
            .collect()
    };
    Page {
        title: format!(
            "Sign in to {}",
            label.unwrap_or_else(|| harness.display_name())
        ),
        sections: vec![Section {
            title: (options.len() > 1).then(|| "Sign in with".into()),
            footer: Some(
                "The sign-in page opens in this phone's browser. Agents that sign in with a \
                 device code show it here."
                    .into(),
            ),
            rows,
        }],
    }
}

fn progress_page(login: &Login, poll: &AgentLoginPoll) -> Page {
    let name = provider_name(login.harness);
    let mut rows = Vec::new();
    let status = match poll.status {
        AgentLoginStatus::Done => "Signed in.".to_owned(),
        AgentLoginStatus::Error => poll
            .message
            .clone()
            .unwrap_or_else(|| "Sign-in failed.".into()),
        AgentLoginStatus::Pending if login.on_mac => format!(
            "{name}'s sign-in returns to the Mac's own browser. Finish it on the Mac, then check \
             again here."
        ),
        AgentLoginStatus::Pending if login.paste => format!(
            "Your browser opened {name}'s sign-in page. Approve access, then paste the code it \
             shows you below."
        ),
        AgentLoginStatus::Pending => format!(
            "Approve access on {name}'s sign-in page. This page updates once the Mac sees the \
             sign-in finish."
        ),
    };
    rows.push(Row::new("status", status, RowKind::Info));
    let pending = poll.status == AgentLoginStatus::Pending;
    if pending && let Some(message) = poll.message.as_deref().filter(|m| !m.is_empty()) {
        rows.push(Row::new("message", message, RowKind::Info));
    }
    if pending && !login.on_mac && is_openable_login_url(login.url) {
        rows.push(Row::new(
            "open",
            "Reopen the sign-in page",
            RowKind::Action { destructive: false },
        ));
    }
    if pending && login.paste {
        rows.push(Row::new(
            "code",
            "Paste the authorization code",
            RowKind::Text {
                multiline: false,
                placeholder: Some("Authorization code".into()),
            },
        ));
    } else if pending {
        rows.push(Row::new(
            "check",
            "Check again",
            RowKind::Action { destructive: false },
        ));
    }
    if pending {
        rows.push(Row::new(
            "cancel",
            "Cancel",
            RowKind::Action { destructive: true },
        ));
    }
    Page {
        title: format!("Sign in to {}", login.harness.display_name()),
        sections: vec![Section {
            title: None,
            footer: None,
            rows,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_ids_round_trip_with_a_url_that_has_colons() {
        let id = page_id(
            "claude-code",
            "l1",
            "paste",
            "https://claude.ai/oauth?a=b:c",
        );
        let login = parse(&id).unwrap();
        assert_eq!(login.harness, HarnessId::ClaudeCode);
        assert_eq!(login.id, Some("l1"));
        assert!(login.paste);
        assert_eq!(login.url, "https://claude.ai/oauth?a=b:c");
        assert_eq!(parse("signin:codex").unwrap().id, None);
    }

    #[test]
    fn paste_login_asks_for_the_code_and_mac_login_only_checks() {
        let pending = AgentLoginPoll {
            status: AgentLoginStatus::Pending,
            message: None,
            url: None,
            callback_port: None,
        };
        let paste = parse("signin:claude-code:l1:paste:https://claude.ai/x").unwrap();
        let ids: Vec<String> = progress_page(&paste, &pending).sections[0]
            .rows
            .iter()
            .map(|r| r.id.clone())
            .collect();
        assert_eq!(ids, ["status", "open", "code", "cancel"]);
        let mac = parse("signin:codex:l2:mac:http://localhost:1455/x").unwrap();
        let ids: Vec<String> = progress_page(&mac, &pending).sections[0]
            .rows
            .iter()
            .map(|r| r.id.clone())
            .collect();
        assert_eq!(ids, ["status", "check", "cancel"]);
    }
}
