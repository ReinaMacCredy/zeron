use zeron_proto::account_usage::{provider_name, used_fraction};
use zeron_proto::computer::host_runs_computer_use;
use zeron_proto::{AgentAccountsSnapshot, LimitResumes};
use zeron_rpc::methods;

use super::{Page, Row, RowAction, RowKind, Section, profiles, providers, routing, tools};
use crate::client::Client;
use crate::error::{ClientError, Result};

async fn limits(client: &Client, device_id: &str) -> Result<LimitResumes> {
    let value = client
        .inner
        .host_first(
            device_id,
            methods::WATCH_LIMIT_RESUMES,
            serde_json::json!({}),
        )
        .await?;
    serde_json::from_value(value).map_err(|e| ClientError::HostError(e.to_string()))
}

pub(super) async fn page(client: &Client, device_id: &str) -> Result<Page> {
    let (accounts, tools, limits, routing, profiles, harnesses, updates) = futures::join!(
        client.host_call::<AgentAccountsSnapshot>(
            device_id,
            methods::LIST_AGENT_ACCOUNTS,
            serde_json::json!({}),
        ),
        tools::state(client, device_id),
        limits(client, device_id),
        client.host_call::<zeron_proto::routing::RoutingSettings>(
            device_id,
            methods::GET_ROUTING_SETTINGS,
            serde_json::json!({}),
        ),
        client.host_call::<zeron_proto::AgentProfilesSnapshot>(
            device_id,
            methods::LIST_AGENT_PROFILES,
            serde_json::json!({}),
        ),
        client.list_harnesses(device_id),
        providers::updates(client, device_id),
    );
    let device = client
        .workspace()
        .devices
        .iter()
        .find(|d| d.id == device_id)
        .cloned();
    let name = device
        .as_ref()
        .map_or_else(|| "Mac".to_owned(), |d| d.name.clone());
    let mut rows = vec![
        Row::link(
            "accounts",
            "Accounts and usage",
            accounts.ok().map(|s| accounts_line(&s)),
        ),
        Row::link(
            "usage",
            "Usage",
            Some("Plan limits and token spend".to_owned()),
        ),
        Row::link(
            "providers",
            "Providers",
            Some(providers::summary(&harnesses, &updates.unwrap_or_default())),
        ),
        Row::link(
            "profiles",
            "Agent profiles",
            profiles.ok().map(|s| profiles::count_line(&s)),
        ),
        Row::link(
            "routing",
            "Routing",
            routing.ok().map(|s| routing::summary(&s)),
        ),
        Row::link(
            "tools",
            "Tools",
            tools.ok().map(|state| {
                let (live, total) = tools::counts(&state);
                format!("{live} of {total} tools on")
            }),
        ),
    ];
    if let Ok(limits) = limits {
        rows.push(
            Row::new(
                "limits",
                "Limits",
                RowKind::Toggle {
                    on: limits.auto_resume,
                },
            )
            .subtitle(Some("Auto-resume after usage limits".to_owned())),
        );
    }
    let mut sections = vec![Section {
        rows,
        ..Section::default()
    }];
    if device.is_some_and(|d| host_runs_computer_use(&d.platform)) {
        let mut stop = Row::new(
            "stop-computer",
            "Stop computer use",
            RowKind::Action { destructive: true },
        )
        .subtitle(Some(
            "Every chat on this Mac stops using apps now".to_owned(),
        ));
        stop.actions.push(RowAction {
            id: "stop-computer".into(),
            title: "Stop".into(),
            destructive: true,
            confirm: Some(format!("Stop computer use on {name}?")),
        });
        sections.push(Section {
            rows: vec![stop],
            ..Section::default()
        });
    }
    Ok(Page {
        title: name,
        sections,
    })
}

async fn stop_computer(client: &Client, device_id: &str) -> Result<()> {
    let live: Vec<String> = client
        .workspace()
        .sessions
        .values()
        .filter(|row| row.device_id == device_id && row.working_since_ms.is_some())
        .map(|row| row.id.clone())
        .collect();
    let mut first_error = None;
    for chat_id in live {
        if let Err(error) = client
            .inner
            .host_rpc(
                device_id,
                methods::COMPUTER_REVOKE_DESKTOP,
                serde_json::json!({ "chatId": chat_id }),
            )
            .await
        {
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

pub(super) async fn act(client: &Client, device_id: &str, action: &str, on: bool) -> Result<()> {
    match action {
        "limits" => {
            client
                .inner
                .host_rpc(
                    device_id,
                    methods::SET_AUTO_RESUME_LIMITS,
                    serde_json::json!({ "enabled": on }),
                )
                .await?;
            Ok(())
        }
        "stop-computer" => stop_computer(client, device_id).await,
        other => Err(ClientError::InvalidArgument(other.to_owned())),
    }
}

fn accounts_line(snapshot: &AgentAccountsSnapshot) -> String {
    let used: Vec<String> = zeron_proto::account_usage::PROVIDERS
        .iter()
        .filter_map(|(harness, _, _)| {
            let active = snapshot
                .accounts
                .iter()
                .find(|a| a.harness == *harness && a.active)?;
            let fraction = used_fraction(active)?;
            Some(format!(
                "{} {}%",
                provider_name(*harness),
                (fraction * 100.0).round() as u32
            ))
        })
        .collect();
    if !used.is_empty() {
        return used.join(" · ");
    }
    match snapshot.accounts.len() {
        0 => "No accounts".to_owned(),
        1 => "1 account".to_owned(),
        n => format!("{n} accounts"),
    }
}
