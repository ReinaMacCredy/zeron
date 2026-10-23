use crate::{Model, ModelOption, ModelOptionChoice, ReasoningLevel};

fn model(
    id: &str,
    label: &str,
    description: &str,
    ladder: &[ReasoningLevel],
    options: Vec<ModelOption>,
) -> Model {
    Model {
        id: id.into(),
        label: label.into(),
        description: (!description.is_empty()).then(|| description.into()),
        reasoning_levels: ladder.to_vec(),
        options,
    }
}

pub mod claude {
    use super::*;

    /// A boolean toggle rendered as an off/on select (the Rust `ModelOption` wire
    /// type has no dedicated boolean kind).
    pub fn toggle(id: &str, label: &str) -> ModelOption {
        ModelOption {
            id: id.into(),
            label: label.into(),
            choices: vec![
                ModelOptionChoice {
                    id: "off".into(),
                    label: "Off".into(),
                },
                ModelOptionChoice {
                    id: "on".into(),
                    label: "On".into(),
                },
            ],
            default_choice: "off".into(),
        }
    }

    /// The 200K/1M context-window select carried by the long-context models. The
    /// CLI's model table marks every one of them `native_1m`: first-party runs get
    /// the 1M window with or without the `<model>[1m]` suffix (live-verified on
    /// 2.1.274, `modelUsage.contextWindow` = 1000000 for a bare id), so 1M is the
    /// honest default and 200K is the opt-in, applied through
    /// `CLAUDE_CODE_DISABLE_1M_CONTEXT` — the CLI's only kill switch for it.
    pub fn context_window() -> ModelOption {
        ModelOption {
            id: "contextWindow".into(),
            label: "Context Window".into(),
            choices: vec![
                ModelOptionChoice {
                    id: "200k".into(),
                    label: "200K".into(),
                },
                ModelOptionChoice {
                    id: "1m".into(),
                    label: "1M".into(),
                },
            ],
            default_choice: "1m".into(),
        }
    }

    /// The CLI's `autoCompactWindow` (100K to 1M tokens). `auto` sends nothing,
    /// so the CLI keeps the window tuned for the model; the CLI compacts 33K
    /// before the chosen window (live-probed on 2.1.293 via `get_context_usage`).
    pub fn auto_compact() -> ModelOption {
        ModelOption {
            id: "autoCompact".into(),
            label: "Auto-compact".into(),
            choices: AUTO_COMPACT_CHOICES
                .iter()
                .map(|(id, label)| ModelOptionChoice {
                    id: (*id).into(),
                    label: (*label).into(),
                })
                .collect(),
            default_choice: "auto".into(),
        }
    }

    const AUTO_COMPACT_CHOICES: &[(&str, &str)] = &[
        ("auto", "Auto"),
        ("300k", "300K"),
        ("400k", "400K"),
        ("600k", "600K"),
        ("800k", "800K"),
    ];

    pub const FULL_LADDER: &[ReasoningLevel] = &[
        ReasoningLevel::Low,
        ReasoningLevel::Medium,
        ReasoningLevel::High,
        ReasoningLevel::XHigh,
        ReasoningLevel::Max,
        ReasoningLevel::Ultracode,
        ReasoningLevel::Ultrathink,
    ];

    /// opus-4-7 / sonnet-5+ tier (claude.ts `claudeEffortsFor`): xhigh native,
    /// no ultracode.
    const XHIGH_LADDER: &[ReasoningLevel] = &[
        ReasoningLevel::Low,
        ReasoningLevel::Medium,
        ReasoningLevel::High,
        ReasoningLevel::XHigh,
        ReasoningLevel::Max,
        ReasoningLevel::Ultrathink,
    ];

    pub fn static_models() -> Vec<Model> {
        vec![
            model(
                "claude-fable-5-1",
                "Fable 5.1",
                "Most intelligent model for building agents",
                FULL_LADDER,
                vec![context_window(), auto_compact()],
            ),
            model(
                "claude-fable-5",
                "Fable 5",
                "Previous generation Fable",
                FULL_LADDER,
                vec![context_window(), auto_compact()],
            ),
            model(
                "claude-opus-5-5",
                "Opus 5.5",
                "Best for everyday, complex tasks",
                FULL_LADDER,
                vec![
                    context_window(),
                    auto_compact(),
                    toggle("fastMode", "Fast Mode"),
                ],
            ),
            model(
                "claude-opus-4-8",
                "Opus 4.8",
                "Previous generation Opus",
                FULL_LADDER,
                vec![toggle("fastMode", "Fast Mode")],
            ),
            model(
                "claude-opus-4-7",
                "Opus 4.7",
                "Older generation Opus",
                XHIGH_LADDER,
                vec![toggle("fastMode", "Fast Mode")],
            ),
            model(
                "claude-sonnet-5-5",
                "Sonnet 5.5",
                "Most efficient for simpler tasks",
                XHIGH_LADDER,
                vec![context_window(), auto_compact()],
            ),
            model(
                "claude-sonnet-5",
                "Sonnet 5",
                "Balanced speed and intelligence",
                XHIGH_LADDER,
                vec![context_window(), auto_compact()],
            ),
            model(
                "claude-haiku-5-5",
                "Haiku 5.5",
                "Fastest for quick answers",
                XHIGH_LADDER,
                vec![context_window(), auto_compact()],
            ),
            model(
                "claude-haiku-4-5",
                "Haiku 4.5",
                "Fastest model for everyday tasks",
                &[],
                vec![toggle("thinking", "Thinking")],
            ),
        ]
    }
}

pub mod codex {
    use super::*;

    const ULTRA_LADDER: &[ReasoningLevel] = &[
        ReasoningLevel::Low,
        ReasoningLevel::Medium,
        ReasoningLevel::High,
        ReasoningLevel::XHigh,
        ReasoningLevel::Max,
        ReasoningLevel::Ultra,
    ];

    const MAX_LADDER: &[ReasoningLevel] = &[
        ReasoningLevel::Low,
        ReasoningLevel::Medium,
        ReasoningLevel::High,
        ReasoningLevel::XHigh,
        ReasoningLevel::Max,
    ];

    /// The service-tier select the app server reports per model (`serviceTiers` /
    /// `additionalSpeedTiers` in `model/list`); "default" means Standard and is
    /// omitted from the wire params entirely.
    fn service_tier() -> ModelOption {
        ModelOption {
            id: "serviceTier".into(),
            label: "Service Tier".into(),
            choices: vec![
                ModelOptionChoice {
                    id: "default".into(),
                    label: "Standard".into(),
                },
                ModelOptionChoice {
                    id: "fast".into(),
                    label: "Fast".into(),
                },
            ],
            default_choice: "default".into(),
        }
    }

    /// The curated fallback catalog: newest family first, with efforts as the
    /// app server reports them. Daybreak Blue reports NO service tiers, so it
    /// carries no trait — sending `serviceTier: priority` for it would be
    /// rejected. The live `model/list` remains authoritative whenever available.
    pub fn static_models() -> Vec<Model> {
        vec![
            model(
                "gpt-6.1-sol",
                "GPT-6.1-Sol",
                "Latest workhorse model for coding and everyday work.",
                ULTRA_LADDER,
                vec![service_tier()],
            ),
            model(
                "gpt-6-astra",
                "GPT-6-Astra",
                "Frontier intelligence for the most demanding work.",
                ULTRA_LADDER,
                vec![service_tier()],
            ),
            model(
                "gpt-6-sol",
                "GPT-6-Sol",
                "Previous generation workhorse model.",
                ULTRA_LADDER,
                vec![service_tier()],
            ),
            model(
                "gpt-6-luna",
                "GPT-6-Luna",
                "Fast and affordable model for easier tasks.",
                MAX_LADDER,
                vec![service_tier()],
            ),
            model(
                "gpt-5.6-sol",
                "GPT-5.6-Sol",
                "Older generation workhorse model.",
                ULTRA_LADDER,
                vec![service_tier()],
            ),
            model(
                "gpt-5.6-terra",
                "GPT-5.6-Terra",
                "Older balanced model for straightforward work.",
                ULTRA_LADDER,
                vec![service_tier()],
            ),
            model(
                "gpt-5.6-luna",
                "GPT-5.6-Luna",
                "Older fast and efficient model.",
                MAX_LADDER,
                vec![service_tier()],
            ),
            model(
                "gpt-daybreak-blue-latest",
                "Daybreak Blue",
                "Latest frontier agentic coding model for broad defensive cybersecurity work.",
                ULTRA_LADDER,
                Vec::new(),
            ),
        ]
    }
}
