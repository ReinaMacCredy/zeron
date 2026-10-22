mod accounts;
mod instances;
mod mac;
mod profiles;
mod providers;
mod routing;
mod signin;
mod tools;
mod usage;

use serde::de::DeserializeOwned;

use crate::client::Client;
use crate::error::{ClientError, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub title: String,
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Section {
    pub title: Option<String>,
    pub footer: Option<String>,
    pub rows: Vec<Row>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub value: Option<String>,
    pub kind: RowKind,
    pub actions: Vec<RowAction>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RowKind {
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
    Meter(Meter),
}

#[derive(Debug, Clone, PartialEq)]
pub struct RowAction {
    pub id: String,
    pub title: String,
    pub destructive: bool,
    pub confirm: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Normal,
    Warn,
    Critical,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Meter {
    pub fill: f32,
    pub tone: Tone,
    pub marker: Option<f32>,
    pub marker_tone: Tone,
    pub foot_left: Option<String>,
    pub foot_right: Option<String>,
    pub foot_left_alert: bool,
    pub foot_right_alert: bool,
}

impl Row {
    fn new(id: impl Into<String>, title: impl Into<String>, kind: RowKind) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            subtitle: None,
            value: None,
            kind,
            actions: Vec::new(),
        }
    }

    fn subtitle(mut self, subtitle: impl Into<Option<String>>) -> Self {
        self.subtitle = subtitle.into();
        self
    }

    fn value(mut self, value: impl Into<Option<String>>) -> Self {
        self.value = value.into();
        self
    }

    fn link(id: &str, title: &str, subtitle: Option<String>) -> Self {
        Self::new(id, title, RowKind::Link { page: id.into() }).subtitle(subtitle)
    }
}

impl From<zeron_proto::account_usage::UsageLevel> for Tone {
    fn from(level: zeron_proto::account_usage::UsageLevel) -> Self {
        use zeron_proto::account_usage::UsageLevel;
        match level {
            UsageLevel::Normal => Tone::Normal,
            UsageLevel::Warn => Tone::Warn,
            UsageLevel::Critical => Tone::Critical,
        }
    }
}

/// `page: None` means the page is gone (its item was removed); go back.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ActionResult {
    pub page: Option<Page>,
    pub page_id: Option<String>,
    pub open_url: Option<String>,
}

impl Client {
    pub async fn host_page(&self, device_id: &str, page: &str) -> Result<Page> {
        match page {
            "mac" => mac::page(self, device_id).await,
            "accounts" => accounts::page(self, device_id).await,
            "tools" => tools::page(self, device_id).await,
            "providers" => providers::page(self, device_id).await,
            usage if usage.starts_with("usage") => {
                usage::page(self, device_id, usage::days_of(usage)).await
            }
            routing if routing.starts_with("routing") => {
                routing::page(self, device_id, routing).await
            }
            profiles if profiles.starts_with("profile") => {
                profiles::page(self, device_id, profiles).await
            }
            signin if signin.starts_with("signin:") => signin::page(self, device_id, signin).await,
            instance
                if instance == "add-provider"
                    || instance.starts_with("acp-search")
                    || instance.starts_with("instance:") =>
            {
                instances::page(self, device_id, instance).await
            }
            other => Err(ClientError::NotFound(other.to_owned())),
        }
    }

    pub async fn host_action(
        &self,
        device_id: &str,
        page: &str,
        action: &str,
        value: Option<String>,
    ) -> Result<ActionResult> {
        if page.starts_with("usage")
            && let Some(days) = action.strip_prefix("days:")
        {
            let page_id = format!("usage:{days}");
            return Ok(ActionResult {
                page: Some(self.host_page(device_id, &page_id).await?),
                page_id: Some(page_id),
                open_url: None,
            });
        }
        if page.starts_with("signin:") {
            return signin::act(self, device_id, page, action, value).await;
        }
        if page == "add-provider" || page.starts_with("acp-search") || page.starts_with("instance:")
        {
            return instances::act(self, device_id, page, action, value).await;
        }
        let on = value.as_deref() == Some("true");
        let mut gone = false;
        match page {
            "mac" => mac::act(self, device_id, action, on).await?,
            "accounts" => accounts::act(self, device_id, action).await?,
            "tools" => tools::act(self, device_id, action, on).await?,
            "providers" => providers::act(self, device_id, action, on).await?,
            routing if routing.starts_with("routing") => {
                routing::act(self, device_id, routing, action).await?
            }
            profiles if profiles.starts_with("profile") => {
                gone = profiles::act(self, device_id, profiles, action, value).await?
            }
            other => return Err(ClientError::NotFound(other.to_owned())),
        }
        Ok(ActionResult {
            page: if gone {
                None
            } else {
                Some(self.host_page(device_id, page).await?)
            },
            page_id: None,
            open_url: None,
        })
    }

    async fn host_call<T: DeserializeOwned>(
        &self,
        device_id: &str,
        method: &'static str,
        params: serde_json::Value,
    ) -> Result<T> {
        let value = self.inner.host_rpc(device_id, method, params).await?;
        serde_json::from_value(value).map_err(|e| ClientError::HostError(e.to_string()))
    }
}
