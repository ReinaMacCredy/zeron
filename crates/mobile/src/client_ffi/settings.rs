use zeron_client::host_settings as hs;

use super::{CoreClient, CoreResult, on_runtime};

#[derive(Debug, Clone, uniffi::Record)]
pub struct HostPage {
    pub title: String,
    pub sections: Vec<HostSection>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct HostSection {
    pub title: Option<String>,
    pub footer: Option<String>,
    pub rows: Vec<HostRow>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct HostRow {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub value: Option<String>,
    pub kind: HostRowKind,
    pub actions: Vec<HostRowAction>,
}

#[derive(Debug, Clone, uniffi::Enum)]
pub enum HostRowKind {
    Info,
    Link {
        page: String,
    },
    Toggle {
        on: bool,
    },
    Check {
        on: bool,
    },
    Action {
        destructive: bool,
    },
    Text {
        multiline: bool,
        placeholder: Option<String>,
    },
    Meter {
        meter: HostMeter,
    },
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct HostRowAction {
    pub id: String,
    pub title: String,
    pub destructive: bool,
    pub confirm: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum HostTone {
    Normal,
    Warn,
    Critical,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct HostMeter {
    pub fill: f32,
    pub tone: HostTone,
    pub marker: Option<f32>,
    pub marker_tone: HostTone,
    pub foot_left: Option<String>,
    pub foot_right: Option<String>,
    pub foot_left_alert: bool,
    pub foot_right_alert: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct HostActionResult {
    pub page: Option<HostPage>,
    pub page_id: Option<String>,
    pub open_url: Option<String>,
}

impl From<hs::Page> for HostPage {
    fn from(page: hs::Page) -> Self {
        Self {
            title: page.title,
            sections: page.sections.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<hs::Section> for HostSection {
    fn from(section: hs::Section) -> Self {
        Self {
            title: section.title,
            footer: section.footer,
            rows: section.rows.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<hs::Row> for HostRow {
    fn from(row: hs::Row) -> Self {
        Self {
            id: row.id,
            title: row.title,
            subtitle: row.subtitle,
            value: row.value,
            kind: row.kind.into(),
            actions: row
                .actions
                .into_iter()
                .map(|a| HostRowAction {
                    id: a.id,
                    title: a.title,
                    destructive: a.destructive,
                    confirm: a.confirm,
                })
                .collect(),
        }
    }
}

impl From<hs::RowKind> for HostRowKind {
    fn from(kind: hs::RowKind) -> Self {
        match kind {
            hs::RowKind::Info => Self::Info,
            hs::RowKind::Link { page } => Self::Link { page },
            hs::RowKind::Toggle { on } => Self::Toggle { on },
            hs::RowKind::Check { on } => Self::Check { on },
            hs::RowKind::Action { destructive } => Self::Action { destructive },
            hs::RowKind::Text {
                multiline,
                placeholder,
            } => Self::Text {
                multiline,
                placeholder,
            },
            hs::RowKind::Meter(m) => Self::Meter {
                meter: HostMeter {
                    fill: m.fill,
                    tone: m.tone.into(),
                    marker: m.marker,
                    marker_tone: m.marker_tone.into(),
                    foot_left: m.foot_left,
                    foot_right: m.foot_right,
                    foot_left_alert: m.foot_left_alert,
                    foot_right_alert: m.foot_right_alert,
                },
            },
        }
    }
}

impl From<hs::Tone> for HostTone {
    fn from(tone: hs::Tone) -> Self {
        match tone {
            hs::Tone::Normal => Self::Normal,
            hs::Tone::Warn => Self::Warn,
            hs::Tone::Critical => Self::Critical,
        }
    }
}

#[uniffi::export]
impl CoreClient {
    pub async fn host_page(&self, device_id: String, page: String) -> CoreResult<HostPage> {
        let client = self.client.clone();
        on_runtime(async move { client.host_page(&device_id, &page).await })
            .await
            .map(Into::into)
    }

    pub async fn host_action(
        &self,
        device_id: String,
        page: String,
        action: String,
        value: Option<String>,
    ) -> CoreResult<HostActionResult> {
        let client = self.client.clone();
        let result =
            on_runtime(async move { client.host_action(&device_id, &page, &action, value).await })
                .await?;
        Ok(HostActionResult {
            page: result.page.map(Into::into),
            page_id: result.page_id,
            open_url: result.open_url,
        })
    }
}
