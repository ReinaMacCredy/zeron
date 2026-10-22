use zeron_client as zc;

use super::{CoreClient, CoreResult, HostRef, on_runtime};

#[derive(Debug, Clone, Default, PartialEq, uniffi::Record)]
pub struct TodayPage {
    pub greeting: String,
    pub summary: String,
    pub tasks: Vec<TodayTask>,
    pub open_tasks: u32,
    pub prompts: u32,
    pub digest: Vec<TodaySentence>,
    pub slots: Vec<TodaySlot>,
    pub usage: Option<TodayUsage>,
    pub runs_loaded: bool,
    pub updated: Option<String>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TodayTask {
    pub id: String,
    pub title: String,
    pub meta: Option<String>,
    pub overdue: bool,
    pub done: bool,
    pub host: Option<HostRef>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TodaySentence {
    pub parts: Vec<TodayPart>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum TodayPart {
    Text { text: String },
    Strong { text: String },
    Bad { text: String },
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TodaySlot {
    pub label: String,
    pub now: bool,
    pub prompts: u32,
    pub hours: Vec<TodayHour>,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TodayHour {
    pub prompts: u32,
    pub future: bool,
}

#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TodayUsage {
    pub harness: String,
    pub harness_name: String,
    pub window: String,
    pub used_fraction: f32,
    pub resets_in: Option<String>,
    pub host: HostRef,
}

impl From<&zc::TodayPage> for TodayPage {
    fn from(t: &zc::TodayPage) -> Self {
        Self {
            greeting: t.greeting.clone(),
            summary: t.summary.clone(),
            tasks: t
                .tasks
                .iter()
                .map(|task| TodayTask {
                    id: task.id.clone(),
                    title: task.title.clone(),
                    meta: task.meta.clone(),
                    overdue: task.overdue,
                    done: task.done,
                    host: task.host.as_ref().map(Into::into),
                })
                .collect(),
            open_tasks: t.open_tasks,
            prompts: t.prompts,
            digest: t
                .digest
                .iter()
                .map(|parts| TodaySentence {
                    parts: parts
                        .iter()
                        .map(|part| match part {
                            zc::TodayPart::Text(text) => TodayPart::Text { text: text.clone() },
                            zc::TodayPart::Strong(text) => TodayPart::Strong { text: text.clone() },
                            zc::TodayPart::Bad(text) => TodayPart::Bad { text: text.clone() },
                        })
                        .collect(),
                })
                .collect(),
            slots: t
                .slots
                .iter()
                .map(|slot| TodaySlot {
                    label: slot.label.clone(),
                    now: slot.now,
                    prompts: slot.prompts,
                    hours: slot
                        .hours
                        .iter()
                        .map(|&(prompts, future)| TodayHour { prompts, future })
                        .collect(),
                })
                .collect(),
            usage: t.usage.as_ref().map(|u| TodayUsage {
                harness: u.harness.clone(),
                harness_name: u.harness_name.clone(),
                window: u.window.clone(),
                used_fraction: u.used_fraction,
                resets_in: u.resets_in.clone(),
                host: (&u.host).into(),
            }),
            runs_loaded: t.runs_loaded,
            updated: t.updated.clone(),
        }
    }
}

#[uniffi::export]
impl CoreClient {
    pub fn today_page(&self) -> TodayPage {
        (&self.client.workspace().today).into()
    }

    pub async fn add_today_task(&self, title: String) -> CoreResult<String> {
        let client = self.client.clone();
        on_runtime(async move { client.add_today_task(&title).await }).await
    }

    pub async fn refresh_today(&self) {
        let client = self.client.clone();
        let _ = on_runtime(async move {
            client.refresh_today().await;
            Ok(())
        })
        .await;
    }
}
