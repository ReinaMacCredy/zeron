use zeron_proto::account_usage::signs_in;
use zeron_proto::provider_instance::{
    ProviderInstancesSnapshot, acp_source_summary, has_own_login,
};
use zeron_proto::{HarnessId, HarnessUpdatePhase, HarnessUpdateStatus};
use zeron_rpc::methods;

use super::{Page, Row, RowAction, RowKind, Section};
use crate::catalog::HarnessInfo;
use crate::client::Client;
use crate::error::{ClientError, Result};

pub(super) async fn updates(client: &Client, device_id: &str) -> Result<Vec<HarnessUpdateStatus>> {
    let value = client
        .inner
        .host_first(
            device_id,
            methods::WATCH_HARNESS_UPDATES,
            serde_json::json!({}),
        )
        .await?;
    serde_json::from_value(value).map_err(|e| ClientError::HostError(e.to_string()))
}

pub(super) async fn page(client: &Client, device_id: &str) -> Result<Page> {
    let (harnesses, updates, instances) = futures::join!(
        client.list_harnesses(device_id),
        updates(client, device_id),
        client.host_call::<ProviderInstancesSnapshot>(
            device_id,
            methods::LIST_PROVIDER_INSTANCES,
            serde_json::json!({}),
        ),
    );
    Ok(build(
        &harnesses,
        &updates.unwrap_or_default(),
        &instances.map(|s| s.instances).unwrap_or_default(),
    ))
}

pub(super) async fn act(client: &Client, device_id: &str, action: &str, on: bool) -> Result<()> {
    let (method, params) = if action == "check" {
        (methods::CHECK_HARNESS_UPDATES, serde_json::json!({}))
    } else if let Some(harness) = action.strip_prefix("harness:") {
        (
            methods::SET_HARNESS_ENABLED,
            serde_json::json!({ "harness": harness, "enabled": on }),
        )
    } else if let Some(harness) = action.strip_prefix("update:") {
        (
            methods::APPLY_HARNESS_UPDATE,
            serde_json::json!({ "harness": harness }),
        )
    } else if let Some(id) = action.strip_prefix("delete:") {
        (
            methods::DELETE_PROVIDER_INSTANCE,
            serde_json::json!({ "instanceId": id }),
        )
    } else {
        return Err(ClientError::InvalidArgument(action.to_owned()));
    };
    client.inner.host_rpc(device_id, method, params).await?;
    Ok(())
}

fn harness_of(id: &str) -> Option<HarnessId> {
    serde_json::from_value(serde_json::Value::String(id.to_owned())).ok()
}

fn listed(harnesses: &[HarnessInfo]) -> impl Iterator<Item = (&HarnessInfo, HarnessId)> {
    harnesses.iter().filter_map(|h| {
        let id = harness_of(&h.id)?;
        (id != HarnessId::Mock && id != HarnessId::Acp).then_some((h, id))
    })
}

pub(super) fn summary(harnesses: &[HarnessInfo], updates: &[HarnessUpdateStatus]) -> String {
    let count = listed(harnesses)
        .filter(|(h, _)| h.installed && h.enabled != Some(false))
        .count();
    let pending = updates
        .iter()
        .filter(|u| u.phase == HarnessUpdatePhase::Available)
        .count();
    let providers = if count == 1 {
        "1 provider".to_owned()
    } else {
        format!("{count} providers")
    };
    match pending {
        0 => providers,
        1 => format!("{providers} · 1 update"),
        n => format!("{providers} · {n} updates"),
    }
}

pub(super) fn build(
    harnesses: &[HarnessInfo],
    updates: &[HarnessUpdateStatus],
    instances: &[zeron_proto::provider_instance::ProviderInstance],
) -> Page {
    let mut rows = Vec::new();
    for (info, harness) in listed(harnesses) {
        let status = updates.iter().find(|u| u.harness == harness);
        let subtitle = if info.installed {
            status.map(HarnessUpdateStatus::version_line)
        } else {
            Some("Not installed. Install the CLI on the Mac to enable.".to_owned())
        };
        rows.push(
            Row::new(
                format!("harness:{}", info.id),
                info.label.clone(),
                if info.installed {
                    RowKind::Toggle {
                        on: info.enabled != Some(false),
                    }
                } else {
                    RowKind::Info
                },
            )
            .subtitle(subtitle.filter(|s| !s.is_empty())),
        );
        if let Some(status) = status.filter(|s| {
            s.can_apply
                && matches!(
                    s.phase,
                    HarnessUpdatePhase::Available | HarnessUpdatePhase::ManualActionRequired
                )
        }) {
            rows.push(Row::new(
                format!("update:{}", info.id),
                match status.latest_version.as_deref() {
                    Some(latest) => format!("Update {} to v{latest}", info.label),
                    None => format!("Update {}", info.label),
                },
                RowKind::Action { destructive: false },
            ));
        }
    }
    rows.push(Row::new(
        "check",
        "Check now",
        RowKind::Action { destructive: false },
    ));
    let mut sections = vec![Section {
        title: Some("Providers".into()),
        footer: None,
        rows,
    }];
    let mut instance_rows: Vec<Row> = instances
        .iter()
        .map(|instance| {
            let mut detail = vec![instance.id.clone()];
            if let Some(source) = acp_source_summary(instance) {
                detail.push(source);
            } else if signs_in(instance.driver) && !has_own_login(instance) {
                detail.push(format!(
                    "Uses the default {} login",
                    instance.driver.display_name()
                ));
            }
            let title = if instance.label.is_empty() {
                instance.id.clone()
            } else {
                instance.label.clone()
            };
            let mut row = Row::link(&format!("instance:{}", instance.id), &title, None)
                .subtitle(Some(detail.join(" · ")));
            row.actions.push(RowAction {
                id: format!("delete:{}", instance.id),
                title: "Delete".into(),
                destructive: true,
                confirm: Some(format!("Delete {title}?")),
            });
            row
        })
        .collect();
    instance_rows.push(Row::link("add-provider", "Add provider", None));
    sections.push(Section {
        title: Some("Instances".into()),
        footer: Some(
            "Instances run a provider with their own name, sign-in, binary or environment.".into(),
        ),
        rows: instance_rows,
    });
    Page {
        title: "Providers".into(),
        sections,
    }
}
