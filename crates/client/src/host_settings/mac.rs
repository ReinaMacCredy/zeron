use zeron_proto::account_usage::{provider_name, used_fraction};
use zeron_proto::{AgentAccountsSnapshot, LimitResumes};
use zeron_rpc::methods;

use super::{Page, Row, RowKind, Section, profiles, providers, routing, tools};
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
    let name = client
        .workspace()
        .devices
        .iter()
        .find(|d| d.id == device_id)
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
    Ok(Page {
        title: name,
        sections: vec![Section {
            rows,
            ..Section::default()
        }],
    })
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
