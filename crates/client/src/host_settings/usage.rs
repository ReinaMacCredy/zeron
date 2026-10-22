use chrono::Utc;
use zeron_proto::AgentAccountsSnapshot;
use zeron_proto::account_usage::{PROVIDERS, provider_accounts, provider_name, usage_blocks};
use zeron_proto::usage::{
    USAGE_WINDOWS, UsageSummary, UsageView, format_tokens, format_usd, percent, plural,
};
use zeron_rpc::methods;

use super::{Meter, Page, Row, RowKind, Section};
use crate::client::Client;
use crate::error::Result;

pub(super) const DEFAULT_DAYS: u32 = 30;

pub(super) fn days_of(page: &str) -> u32 {
    page.strip_prefix("usage:")
        .and_then(|days| days.parse().ok())
        .filter(|days| USAGE_WINDOWS.iter().any(|(d, _)| d == days))
        .unwrap_or(DEFAULT_DAYS)
}

pub(super) async fn page(client: &Client, device_id: &str, days: u32) -> Result<Page> {
    let (summary, accounts) = futures::join!(
        client.host_call::<UsageSummary>(
            device_id,
            methods::GET_USAGE_SUMMARY,
            serde_json::json!({ "days": days }),
        ),
        client.host_call::<AgentAccountsSnapshot>(
            device_id,
            methods::LIST_AGENT_ACCOUNTS,
            serde_json::json!({}),
        ),
    );
    Ok(build(days, &summary?, accounts.ok().as_ref()))
}

fn plan_rows(snapshot: &AgentAccountsSnapshot) -> Vec<Row> {
    let now = Utc::now();
    PROVIDERS
        .iter()
        .filter_map(|(harness, _, _)| {
            let accounts = provider_accounts(snapshot, *harness);
            let active = accounts.iter().find(|a| a.active)?;
            let block = usage_blocks(active, now).into_iter().next()?;
            Some(
                Row::new(
                    format!("plan:{}", provider_name(*harness)),
                    format!("{} {}", provider_name(*harness), block.label),
                    RowKind::Meter(Meter {
                        fill: block.fill,
                        tone: block.level.into(),
                        marker: block.marker.map(|(at, _)| at),
                        marker_tone: block.marker.map_or(block.level, |(_, l)| l).into(),
                        foot_left: None,
                        foot_right: None,
                        foot_left_alert: false,
                        foot_right_alert: false,
                    }),
                )
                .value(block.headline)
                .subtitle(block.resets),
            )
        })
        .collect()
}

pub(super) fn build(
    days: u32,
    summary: &UsageSummary,
    accounts: Option<&AgentAccountsSnapshot>,
) -> Page {
    let view = UsageView::new(summary);
    let mut sections = Vec::new();
    let plans = accounts.map(plan_rows).unwrap_or_default();
    if !plans.is_empty() {
        sections.push(Section {
            title: Some("Plan limits".into()),
            footer: Some("Refreshes after each turn".into()),
            rows: plans,
        });
    }
    let mut period = USAGE_WINDOWS
        .iter()
        .map(|(d, label)| {
            Row::new(
                format!("days:{d}"),
                *label,
                RowKind::Check { on: *d == days },
            )
        })
        .collect::<Vec<_>>();
    period.insert(
        0,
        Row::new("total", format_usd(view.cost), RowKind::Info).subtitle(Some(format!(
            "{} · API estimate",
            plural(view.sessions, "session")
        ))),
    );
    sections.push(Section {
        title: Some("Token usage".into()),
        footer: (view.unpriced > 0).then(|| {
            format!(
                "Excludes {} on models with no known price",
                plural(view.unpriced, "response")
            )
        }),
        rows: period,
    });
    let providers: Vec<Row> = view
        .active()
        .into_iter()
        .map(|i| {
            let provider = &view.providers[i];
            Row::new(
                format!("provider:{i}"),
                provider.harness.display_name(),
                RowKind::Info,
            )
            .value(Some(format_usd(provider.cost)))
            .subtitle(Some(format!(
                "{} of cost · {} tokens",
                percent(provider.cost, view.cost),
                format_tokens(provider.tokens)
            )))
        })
        .collect();
    if !providers.is_empty() {
        sections.push(Section {
            title: Some("By provider".into()),
            footer: None,
            rows: providers,
        });
    }
    let models: Vec<Row> = view
        .models
        .iter()
        .take(5)
        .enumerate()
        .map(|(ix, model)| {
            Row::new(format!("model:{ix}"), model.model.clone(), RowKind::Info)
                .value(Some(format_usd(model.cost)))
                .subtitle(Some(format!(
                    "{} · {} tokens",
                    model.harness.display_name(),
                    format_tokens(model.tokens)
                )))
        })
        .collect();
    if !models.is_empty() {
        sections.push(Section {
            title: Some("Top models".into()),
            footer: None,
            rows: models,
        });
    }
    let total = |id: &str, title: &str, value: String| {
        Row::new(id, title, RowKind::Info).value(Some(value))
    };
    sections.push(Section {
        title: Some("Totals".into()),
        footer: Some(
            "Plan limits from your agent accounts, then token spend from chats on this device."
                .into(),
        ),
        rows: vec![
            total(
                "processed",
                "Processed tokens",
                format_tokens(view.tokens.total()),
            ),
            total(
                "cached",
                "Cached input",
                format_tokens(view.tokens.cached_input),
            ),
            total(
                "uncached",
                "Uncached input",
                format_tokens(view.tokens.uncached_input + view.tokens.cache_write),
            ),
            total("output", "Output", format_tokens(view.tokens.output)),
            total("savings", "Cache savings", format_usd(view.savings)),
        ],
    });
    Page {
        title: "Usage".into(),
        sections,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_page_totals_cost_per_provider_and_marks_the_window() {
        let summary: UsageSummary = serde_json::from_value(serde_json::json!({
            "hourly": false, "periods": [0], "untilMs": 0, "pricesLoaded": true,
            "buckets": [
                { "period": 0, "harness": "claude-code", "model": "claude-opus-5-5",
                  "tokens": { "uncachedInput": 1000, "cachedInput": 9000, "cacheWrite": 0, "output": 500 },
                  "costUsd": 3.0, "cacheSavingsUsd": 1.5, "records": 4, "unpricedRecords": 0 },
                { "period": 0, "harness": "codex", "model": "gpt-5.5",
                  "tokens": { "uncachedInput": 500, "cachedInput": 0, "cacheWrite": 0, "output": 500 },
                  "costUsd": 1.0, "cacheSavingsUsd": 0.0, "records": 2, "unpricedRecords": 1 }
            ],
            "sessions": [{ "harness": "claude-code", "sessions": 2 }, { "harness": "codex", "sessions": 1 }]
        }))
        .unwrap();
        let page = build(7, &summary, None);
        let usage = &page.sections[0];
        assert_eq!(usage.rows[0].title, "$4.00");
        assert_eq!(
            usage.rows[0].subtitle.as_deref(),
            Some("3 sessions · API estimate")
        );
        assert!(
            usage
                .rows
                .iter()
                .any(|r| r.id == "days:7" && r.kind == RowKind::Check { on: true })
        );
        assert_eq!(
            usage.footer.as_deref(),
            Some("Excludes 1 response on models with no known price")
        );
        let providers = &page.sections[1].rows;
        assert_eq!(providers[0].title, "Codex");
        assert_eq!(
            providers[0].subtitle.as_deref(),
            Some("25.0% of cost · 1K tokens")
        );
        assert_eq!(days_of("usage:90"), 90);
        assert_eq!(days_of("usage:3"), DEFAULT_DAYS);
    }
}
