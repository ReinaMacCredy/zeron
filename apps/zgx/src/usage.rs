//! Settings → Usage's numbers (ui/account/usage.luau): the engine's
//! `GetUsageSummary`, added up and formatted as the gpui page does
//! (zeron_proto::usage), so the two never read differently.
//!
//!   usage.summary({ days })   { hourly, sessions, unpriced, cost, tokens,
//!                               cached, uncached, output, savings,
//!                               providers, models, periods }
//!
//! Figures are text, ready to show; `cost_n` and `tokens_n` are what the chart
//! scales by. `providers`: { harness, name, sessions, cost, tokens,
//! cost_share, tokens_share }, those with any use. `models`: { key, name,
//! harness, cost, tokens, cost_share, tokens_share }, the priciest first.
//! `periods`: { start (Unix seconds), cost, tokens, cost_n, tokens_n }, oldest
//! first.

use serde_json::json;
use zeron_proto::usage::{
    ModelTotal, PeriodTotal, ProviderTotal, USAGE_WINDOWS, UsageSummary, UsageView, format_tokens, format_usd, percent,
};
use zgx_desktop::WindowContext;
use zgx_script::{ScriptHost, Value};

use crate::engine;

pub fn install(_cx: &mut WindowContext, host: &ScriptHost) {
    host.async_action("usage.summary", |args| {
        let days = args.field("days").and_then(|v| v.as_f64()).map(|d| d as u32);
        async move {
            let days = days.filter(|d| USAGE_WINDOWS.iter().any(|(w, _)| w == d)).ok_or("invalid")?;
            let answer = engine::get().call("GetUsageSummary", json!({ "days": days })).await.map_err(|error| {
                eprintln!("zeron-zgx: usage: {error}");
                engine::outcome(&error).to_string()
            })?;
            let summary: UsageSummary = serde_json::from_value(answer).map_err(|error| {
                eprintln!("zeron-zgx: usage: {error}");
                "failed".to_string()
            })?;
            Ok(summary_value(&summary))
        }
    });
}

fn summary_value(summary: &UsageSummary) -> Value {
    let view = UsageView::new(summary);
    let models = view.models.iter().map(|m| model_value(m, &view));
    let periods = view.periods.iter().map(period_value);
    Value::record([
        ("hourly", Value::Bool(summary.hourly)),
        ("sessions", Value::from(view.sessions as f64)),
        ("unpriced", Value::from(view.unpriced as f64)),
        ("cost", Value::from(format_usd(view.cost).as_str())),
        ("tokens", Value::from(format_tokens(view.tokens.total()).as_str())),
        ("cached", Value::from(format_tokens(view.tokens.cached_input).as_str())),
        // A cache write is input the model hadn't seen, as on the gpui page.
        ("uncached", Value::from(format_tokens(view.tokens.uncached_input + view.tokens.cache_write).as_str())),
        ("output", Value::from(format_tokens(view.tokens.output).as_str())),
        ("savings", Value::from(format_usd(view.savings).as_str())),
        ("providers", Value::List(view.active().into_iter().map(|i| provider_value(&view.providers[i], &view)).collect::<Vec<_>>().into())),
        ("models", Value::List(models.collect::<Vec<_>>().into())),
        ("periods", Value::List(periods.collect::<Vec<_>>().into())),
    ])
}

/// An agent's share, as the gpui page's summary rows: its sessions, figures and share of each.
fn provider_value(p: &ProviderTotal, view: &UsageView) -> Value {
    let harness = serde_json::to_value(p.harness).ok();
    Value::record([
        ("harness", Value::from(harness.as_ref().and_then(|h| h.as_str()).unwrap_or_default())),
        ("name", Value::from(p.harness.display_name())),
        ("sessions", Value::from(p.sessions as f64)),
        ("cost", Value::from(format_usd(p.cost).as_str())),
        ("tokens", Value::from(format_tokens(p.tokens).as_str())),
        ("cost_share", Value::from(percent(p.cost, view.cost).as_str())),
        ("tokens_share", Value::from(percent(p.tokens as f64, view.tokens.total() as f64).as_str())),
    ])
}

fn model_value(model: &ModelTotal, view: &UsageView) -> Value {
    let harness = serde_json::to_value(model.harness).ok();
    let harness = harness.as_ref().and_then(|h| h.as_str()).unwrap_or_default();
    Value::record([
        ("key", Value::from(format!("{harness}/{}", model.model).as_str())),
        ("name", Value::from(model.model.as_str())),
        ("harness", Value::from(harness)),
        ("cost", Value::from(format_usd(model.cost).as_str())),
        ("tokens", Value::from(format_tokens(model.tokens).as_str())),
        ("cost_share", Value::from(percent(model.cost, view.cost).as_str())),
        ("tokens_share", Value::from(percent(model.tokens as f64, view.tokens.total() as f64).as_str())),
    ])
}

fn period_value(period: &PeriodTotal) -> Value {
    Value::record([
        ("start", Value::from(period.start as f64 / 1000.0)),
        ("cost", Value::from(format_usd(period.cost).as_str())),
        ("tokens", Value::from(format_tokens(period.tokens).as_str())),
        ("cost_n", Value::from(period.cost)),
        ("tokens_n", Value::from(period.tokens as f64)),
    ])
}
