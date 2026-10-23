use std::cmp::Reverse;

use zeron_proto::chat_mentions::{
    MentionScope, chat_mention_link, thread_matches, thread_status_label,
};

use crate::Client;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadMention {
    pub chat_id: String,
    pub title: String,
    pub link: String,
    pub status: Option<String>,
    pub project: String,
}

impl Client {
    pub fn thread_mentions(
        &self,
        device_id: &str,
        space_id: Option<&str>,
        exclude: Option<&str>,
        query: &str,
    ) -> Vec<ThreadMention> {
        let (state, _) = self.inner.workspace.state();
        let snapshot = self.inner.workspace.snapshot();
        let mut chats: Vec<_> = state
            .chats
            .iter()
            .filter(|chat| !chat.archived && chat.is_top_level())
            .collect();
        chats.sort_by_key(|chat| {
            Reverse(
                snapshot
                    .session(&chat.id)
                    .map_or(0, |row| row.last_activity_ms),
            )
        });
        let scope = MentionScope {
            device_id,
            space_id,
            exclude,
        };
        thread_matches(chats.into_iter(), query, scope, |_| false)
            .into_iter()
            .filter_map(|(chat, title)| {
                let row = snapshot.session(&chat.id);
                Some(ThreadMention {
                    link: chat_mention_link(&chat.id, &title)?,
                    status: row
                        .and_then(|row| thread_status_label(row.indicator))
                        .map(str::to_owned),
                    project: match (row.and_then(|row| row.project.as_ref()), &chat.space_id) {
                        (Some(project), _) => project.name.clone(),
                        (None, None) => "~".into(),
                        (None, Some(_)) => "?".into(),
                    },
                    chat_id: chat.id.clone(),
                    title,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::Client;
    use crate::config::{ClientConfig, Credentials, DemoOptions};
    use crate::events::NullListener;

    #[test]
    fn threads_list_the_projects_chats_first_and_link_as_chat_chips() {
        let dir = tempfile::tempdir().unwrap();
        let config = ClientConfig::new("https://edge.invalid", dir.path());
        let client = Client::new(
            config,
            Credentials::Demo(DemoOptions::default()),
            Arc::new(NullListener),
        )
        .unwrap();
        let all = client.thread_mentions("dev-mac", Some("space-zeron"), Some("chat-veil"), "");
        assert!(!all.is_empty() && all.len() <= 6);
        assert!(
            all.iter()
                .all(|t| t.chat_id != "chat-veil" && t.chat_id != "chat-oklch")
        );
        let zeron = all[0].project.clone();
        let first_other = all
            .iter()
            .position(|t| t.project != zeron)
            .unwrap_or(all.len());
        assert!(all[first_other..].iter().all(|t| t.project != zeron));

        let picker = client.thread_mentions("dev-mac", None, None, "catalog");
        assert_eq!(picker.len(), 1);
        assert_eq!(
            picker[0].link,
            "[Model picker catalog sync](zeron-chat:chat-picker)"
        );
        assert_eq!(picker[0].status.as_deref(), Some("Input"));
        assert!(
            client
                .thread_mentions("dev-other", None, None, "catalog")
                .is_empty()
        );
    }
}
