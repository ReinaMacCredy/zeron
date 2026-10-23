use chrono::{DateTime, Utc};
use zeron_proto::account_usage::{
    PROVIDERS, add_account_label, keeps_one_login, provider_accounts, provider_name, provider_note,
    switches_accounts, usage_blocks,
};
use zeron_proto::{AgentAccount, AgentAccountsSnapshot, HarnessId};
use zeron_rpc::methods;

use super::{Meter, Page, Row, RowAction, RowKind, Section};
use crate::client::Client;
use crate::error::{ClientError, Result};

pub(super) async fn page(client: &Client, device_id: &str) -> Result<Page> {
    let snapshot: AgentAccountsSnapshot = client
        .host_call(
            device_id,
            methods::LIST_AGENT_ACCOUNTS,
            serde_json::json!({}),
        )
        .await?;
    Ok(build(&snapshot, Utc::now()))
}

pub(super) async fn act(client: &Client, device_id: &str, action: &str) -> Result<()> {
    let (method, rest) = if let Some(rest) = action.strip_prefix("account:") {
        (methods::ACTIVATE_AGENT_ACCOUNT, rest)
    } else if let Some(rest) = action.strip_prefix("remove:") {
        (methods::FORGET_AGENT_ACCOUNT, rest)
    } else {
        return Err(ClientError::InvalidArgument(action.to_owned()));
    };
    let (harness, account_id) = rest
        .split_once(':')
        .ok_or_else(|| ClientError::InvalidArgument(action.to_owned()))?;
    client
        .inner
        .host_rpc(
            device_id,
            method,
            serde_json::json!({ "harness": harness, "accountId": account_id }),
        )
        .await?;
    Ok(())
}

pub(super) fn harness_key(harness: HarnessId) -> String {
    serde_json::to_value(harness)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

pub(super) fn build(snapshot: &AgentAccountsSnapshot, now: DateTime<Utc>) -> Page {
    let mut sections: Vec<Section> = PROVIDERS
        .iter()
        .filter_map(|(harness, _, _)| provider_section(snapshot, *harness, now))
        .collect();
    if sections.is_empty() {
        sections.push(Section {
            rows: vec![Row::new(
                "empty",
                "No agent accounts on this Mac",
                RowKind::Info,
            )],
            ..Section::default()
        });
    }
    if let Some(last) = sections.last_mut() {
        last.footer = Some(
            [
                last.footer.take(),
                Some(
                    "Account changes affect new sessions. On macOS, Claude Code may take about \
                     30 seconds to pick up a switched account."
                        .to_owned(),
                ),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n\n"),
        );
    }
    Page {
        title: "Accounts and usage".into(),
        sections,
    }
}

fn provider_section(
    snapshot: &AgentAccountsSnapshot,
    harness: HarnessId,
    now: DateTime<Utc>,
) -> Option<Section> {
    let accounts = provider_accounts(snapshot, harness);
    let warnings: Vec<&str> = snapshot
        .warnings
        .iter()
        .filter(|w| w.harness == harness)
        .map(|w| w.message.as_str())
        .collect();
    if accounts.is_empty() && warnings.is_empty() {
        return None;
    }
    let key = harness_key(harness);
    let mut rows: Vec<Row> = accounts
        .iter()
        .find(|a| a.active)
        .map(|active| meter_rows(active, &key, now))
        .unwrap_or_default();
    rows.extend(accounts.iter().map(|account| account_row(account, &key)));
    if !(keeps_one_login(harness) && !accounts.is_empty()) {
        rows.push(Row::new(
            format!("add:{key}"),
            add_account_label(harness, accounts.is_empty()),
            RowKind::Link {
                page: format!("signin:{key}"),
            },
        ));
    }
    let footer = provider_note(harness)
        .map(str::to_owned)
        .into_iter()
        .chain(warnings.into_iter().map(str::to_owned))
        .collect::<Vec<_>>();
    Some(Section {
        title: Some(provider_name(harness).to_owned()),
        footer: (!footer.is_empty()).then(|| footer.join("\n\n")),
        rows,
    })
}

fn meter_rows(account: &AgentAccount, key: &str, now: DateTime<Utc>) -> Vec<Row> {
    usage_blocks(account, now)
        .into_iter()
        .enumerate()
        .map(|(ix, block)| {
            Row::new(
                format!("meter:{key}:{ix}"),
                block.label,
                RowKind::Meter(Meter {
                    fill: block.fill,
                    tone: block.level.into(),
                    marker: block.marker.map(|(at, _)| at),
                    marker_tone: block.marker.map_or(block.level, |(_, level)| level).into(),
                    foot_left: block.foot_left,
                    foot_right: block.foot_right,
                    foot_left_alert: block.foot_left_alert,
                    foot_right_alert: block.foot_right_alert,
                }),
            )
            .value(block.headline)
            .subtitle(block.resets)
        })
        .collect()
}

fn account_row(account: &AgentAccount, key: &str) -> Row {
    let email = account
        .email
        .clone()
        .or_else(|| account.display_name.clone())
        .unwrap_or_else(|| "Unknown account".into());
    let organization = account.organization.clone().filter(|org| {
        account
            .email
            .as_deref()
            .is_none_or(|email| !org.starts_with(email))
    });
    let usage_note = (account.usage_windows.is_empty())
        .then(|| account.usage_error.clone())
        .flatten();
    let meta = [
        account.plan_label.clone(),
        organization,
        account.active.then(|| "In use".to_owned()),
        usage_note,
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ");
    let mut row = Row::new(
        format!("account:{key}:{}", account.id),
        email.clone(),
        RowKind::Check { on: account.active },
    )
    .subtitle((!meta.is_empty()).then_some(meta));
    if account.switchable && switches_accounts(account.harness) {
        row.actions.push(RowAction {
            id: format!("remove:{key}:{}", account.id),
            title: "Remove account".into(),
            destructive: true,
            confirm: Some(format!("Remove {email} from this Mac?")),
        });
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use zeron_proto::AgentUsageWindow;

    fn account(id: &str, harness: HarnessId, active: bool) -> AgentAccount {
        serde_json::from_value(serde_json::json!({
            "id": id, "harness": harness, "email": format!("{id}@zeron.sh"),
            "planLabel": "Max", "active": active, "switchable": true,
        }))
        .unwrap()
    }

    #[test]
    fn accounts_page_meters_the_active_login_and_lists_every_account() {
        let now = Utc.with_ymd_and_hms(2026, 10, 22, 12, 0, 0).unwrap();
        let mut active = account("reina", HarnessId::ClaudeCode, true);
        active.usage_windows.push(AgentUsageWindow {
            label: "Session".into(),
            used_fraction: 0.62,
            resets_at: Some(now + chrono::Duration::minutes(130)),
        });
        let snapshot = AgentAccountsSnapshot {
            accounts: vec![active, account("work", HarnessId::ClaudeCode, false)],
            warnings: vec![],
        };
        let page = build(&snapshot, now);
        let rows = &page.sections[0].rows;
        assert_eq!(page.sections[0].title.as_deref(), Some("Claude Code"));
        assert_eq!(rows[0].title, "5h");
        assert_eq!(rows[0].value.as_deref(), Some("38% left"));
        assert_eq!(rows[0].subtitle.as_deref(), Some("Resets in 2h 10m"));
        assert_eq!(rows[1].subtitle.as_deref(), Some("Max · In use"));
        assert_eq!(rows[2].kind, RowKind::Check { on: false });
        assert_eq!(rows[2].actions[0].id, "remove:claude-code:work");
        assert_eq!(
            rows[3].kind,
            RowKind::Link {
                page: "signin:claude-code".into()
            }
        );
        assert!(
            page.sections[0]
                .footer
                .as_deref()
                .unwrap()
                .contains("30 seconds")
        );
    }
}
