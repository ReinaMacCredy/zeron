use std::io::Cursor;
use std::sync::{Arc, OnceLock};

use gpui::{Image, ImageFormat};
use zeron_update::version_newer;

pub use zeron_docs::changelog::{Entry, Feature, entries, entry_for};

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

const IMAGES: &[(&str, &[u8])] = &[
    ("inbox.jpg", include_bytes!("../assets/whats_new/inbox.jpg")),
    (
        "kanban.jpg",
        include_bytes!("../assets/whats_new/kanban.jpg"),
    ),
    ("usage.jpg", include_bytes!("../assets/whats_new/usage.jpg")),
    (
        "mission-control.jpg",
        include_bytes!("../assets/whats_new/mission-control.jpg"),
    ),
    (
        "supervisor.jpg",
        include_bytes!("../assets/whats_new/supervisor.jpg"),
    ),
    (
        "add-provider.jpg",
        include_bytes!("../assets/whats_new/add-provider.jpg"),
    ),
    (
        "mission-tree.jpg",
        include_bytes!("../assets/whats_new/mission-tree.jpg"),
    ),
    (
        "task-card-context.jpg",
        include_bytes!("../assets/whats_new/task-card-context.jpg"),
    ),
    (
        "getting-around-window.jpg",
        include_bytes!("../assets/whats_new/getting-around-window.jpg"),
    ),
    (
        "getting-around-tabs.jpg",
        include_bytes!("../assets/whats_new/getting-around-tabs.jpg"),
    ),
    (
        "new-chat-canvas.jpg",
        include_bytes!("../assets/whats_new/new-chat-canvas.jpg"),
    ),
    (
        "new-chat-mission.jpg",
        include_bytes!("../assets/whats_new/new-chat-mission.jpg"),
    ),
    (
        "composer-queue.jpg",
        include_bytes!("../assets/whats_new/composer-queue.jpg"),
    ),
    (
        "composer-footer.jpg",
        include_bytes!("../assets/whats_new/composer-footer.jpg"),
    ),
    (
        "conversation-hover.jpg",
        include_bytes!("../assets/whats_new/conversation-hover.jpg"),
    ),
    (
        "conversation-tools.jpg",
        include_bytes!("../assets/whats_new/conversation-tools.jpg"),
    ),
    (
        "commands-slash-menu.jpg",
        include_bytes!("../assets/whats_new/commands-slash-menu.jpg"),
    ),
    (
        "commands-mention-menu.jpg",
        include_bytes!("../assets/whats_new/commands-mention-menu.jpg"),
    ),
    (
        "models-compact-picker.jpg",
        include_bytes!("../assets/whats_new/models-compact-picker.jpg"),
    ),
    (
        "models-full-picker.jpg",
        include_bytes!("../assets/whats_new/models-full-picker.jpg"),
    ),
    (
        "sidebar-view-options.jpg",
        include_bytes!("../assets/whats_new/sidebar-view-options.jpg"),
    ),
    (
        "sidebar-inbox-shelves.jpg",
        include_bytes!("../assets/whats_new/sidebar-inbox-shelves.jpg"),
    ),
    (
        "sidebar-mission.jpg",
        include_bytes!("../assets/whats_new/sidebar-mission.jpg"),
    ),
    (
        "palette-all.jpg",
        include_bytes!("../assets/whats_new/palette-all.jpg"),
    ),
    (
        "palette-switcher.jpg",
        include_bytes!("../assets/whats_new/palette-switcher.jpg"),
    ),
    (
        "shortcuts-recording.jpg",
        include_bytes!("../assets/whats_new/shortcuts-recording.jpg"),
    ),
    (
        "appearance-themes.jpg",
        include_bytes!("../assets/whats_new/appearance-themes.jpg"),
    ),
    (
        "appearance-background.jpg",
        include_bytes!("../assets/whats_new/appearance-background.jpg"),
    ),
    (
        "home-widgets-grid.jpg",
        include_bytes!("../assets/whats_new/home-widgets-grid.jpg"),
    ),
    (
        "updates-whats-new-card.jpg",
        include_bytes!("../assets/whats_new/updates-whats-new-card.jpg"),
    ),
    (
        "updates-agent-card.jpg",
        include_bytes!("../assets/whats_new/updates-agent-card.jpg"),
    ),
    (
        "updates-new-since.jpg",
        include_bytes!("../assets/whats_new/updates-new-since.jpg"),
    ),
    (
        "providers-add.jpg",
        include_bytes!("../assets/whats_new/providers-add.jpg"),
    ),
    (
        "agent-profiles-editor.jpg",
        include_bytes!("../assets/whats_new/agent-profiles-editor.jpg"),
    ),
    (
        "agent-profiles-chip.jpg",
        include_bytes!("../assets/whats_new/agent-profiles-chip.jpg"),
    ),
    (
        "delegation-finished.jpg",
        include_bytes!("../assets/whats_new/delegation-finished.jpg"),
    ),
    (
        "threads-from-row.jpg",
        include_bytes!("../assets/whats_new/threads-from-row.jpg"),
    ),
    (
        "side-chat-tabs.jpg",
        include_bytes!("../assets/whats_new/side-chat-tabs.jpg"),
    ),
    (
        "handoff-menu.jpg",
        include_bytes!("../assets/whats_new/handoff-menu.jpg"),
    ),
    (
        "handoff-divider.jpg",
        include_bytes!("../assets/whats_new/handoff-divider.jpg"),
    ),
    (
        "review-findings.jpg",
        include_bytes!("../assets/whats_new/review-findings.jpg"),
    ),
    (
        "review-diff-card.jpg",
        include_bytes!("../assets/whats_new/review-diff-card.jpg"),
    ),
    (
        "compaction-popover.jpg",
        include_bytes!("../assets/whats_new/compaction-popover.jpg"),
    ),
    (
        "compaction-classifier.jpg",
        include_bytes!("../assets/whats_new/compaction-classifier.jpg"),
    ),
    (
        "agent-questions-panel.jpg",
        include_bytes!("../assets/whats_new/agent-questions-panel.jpg"),
    ),
    (
        "agent-questions-pill.jpg",
        include_bytes!("../assets/whats_new/agent-questions-pill.jpg"),
    ),
    (
        "agent-tools-settings.jpg",
        include_bytes!("../assets/whats_new/agent-tools-settings.jpg"),
    ),
    (
        "projects-add.jpg",
        include_bytes!("../assets/whats_new/projects-add.jpg"),
    ),
    (
        "projects-actions.jpg",
        include_bytes!("../assets/whats_new/projects-actions.jpg"),
    ),
    (
        "right-pane-menu.jpg",
        include_bytes!("../assets/whats_new/right-pane-menu.jpg"),
    ),
    (
        "right-pane-diffs.jpg",
        include_bytes!("../assets/whats_new/right-pane-diffs.jpg"),
    ),
    (
        "live-state-column.jpg",
        include_bytes!("../assets/whats_new/live-state-column.jpg"),
    ),
    (
        "terminal-drawer.jpg",
        include_bytes!("../assets/whats_new/terminal-drawer.jpg"),
    ),
    (
        "git-commit-panel.jpg",
        include_bytes!("../assets/whats_new/git-commit-panel.jpg"),
    ),
    (
        "git-commit.jpg",
        include_bytes!("../assets/whats_new/git-commit.jpg"),
    ),
    (
        "tasks-card.jpg",
        include_bytes!("../assets/whats_new/tasks-card.jpg"),
    ),
    (
        "tasks-list.jpg",
        include_bytes!("../assets/whats_new/tasks-list.jpg"),
    ),
    (
        "mission-control-tree.jpg",
        include_bytes!("../assets/whats_new/mission-control-tree.jpg"),
    ),
    (
        "mission-control-strip.jpg",
        include_bytes!("../assets/whats_new/mission-control-strip.jpg"),
    ),
    (
        "supervisor-feed.jpg",
        include_bytes!("../assets/whats_new/supervisor-feed.jpg"),
    ),
    (
        "supervisor-authority.jpg",
        include_bytes!("../assets/whats_new/supervisor-authority.jpg"),
    ),
    (
        "supervisor-conversation.jpg",
        include_bytes!("../assets/whats_new/supervisor-conversation.jpg"),
    ),
    (
        "split-view.jpg",
        include_bytes!("../assets/whats_new/split-view.jpg"),
    ),
    (
        "computer-use-consent.jpg",
        include_bytes!("../assets/whats_new/computer-use-consent.jpg"),
    ),
    (
        "computer-use-settings.jpg",
        include_bytes!("../assets/whats_new/computer-use-settings.jpg"),
    ),
    (
        "browser-annotate.jpg",
        include_bytes!("../assets/whats_new/browser-annotate.jpg"),
    ),
    (
        "browser-html-card.jpg",
        include_bytes!("../assets/whats_new/browser-html-card.jpg"),
    ),
    (
        "appshots-settings.jpg",
        include_bytes!("../assets/whats_new/appshots-settings.jpg"),
    ),
    (
        "device-hub-device.jpg",
        include_bytes!("../assets/whats_new/device-hub-device.jpg"),
    ),
    (
        "notifications-settings.jpg",
        include_bytes!("../assets/whats_new/notifications-settings.jpg"),
    ),
    (
        "devices-settings.jpg",
        include_bytes!("../assets/whats_new/devices-settings.jpg"),
    ),
    (
        "command-line-status.jpg",
        include_bytes!("../assets/whats_new/command-line-status.jpg"),
    ),
];

pub struct Picture {
    pub image: Arc<Image>,
    pub aspect: f32,
}

#[derive(Debug, PartialEq)]
pub enum Resolution<'a> {
    Show(&'a Entry),
    Advance,
    Noop,
}

pub fn resolve<'a>(entries: &'a [Entry], current: &str, last_seen: Option<&str>) -> Resolution<'a> {
    let Some(last_seen) = last_seen else {
        return Resolution::Advance;
    };
    if !version_newer(current, last_seen) {
        return Resolution::Noop;
    }
    entries
        .iter()
        .filter(|entry| {
            version_newer(&entry.version, last_seen) && !version_newer(&entry.version, current)
        })
        .reduce(|newest, entry| {
            if version_newer(&entry.version, &newest.version) {
                entry
            } else {
                newest
            }
        })
        .map_or(Resolution::Advance, Resolution::Show)
}

pub fn picture(name: &str) -> Option<&'static Picture> {
    static PICTURES: OnceLock<Vec<(&'static str, Picture)>> = OnceLock::new();
    PICTURES
        .get_or_init(|| {
            IMAGES
                .iter()
                .filter_map(|(name, bytes)| Some((*name, decode(bytes)?)))
                .collect()
        })
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, picture)| picture)
}

fn decode(bytes: &[u8]) -> Option<Picture> {
    let reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let format = match reader.format()? {
        image::ImageFormat::Jpeg => ImageFormat::Jpeg,
        image::ImageFormat::WebP => ImageFormat::Webp,
        image::ImageFormat::Png => ImageFormat::Png,
        _ => return None,
    };
    let (width, height) = reader.into_dimensions().ok()?;
    (width > 0 && height > 0).then(|| Picture {
        image: Arc::new(Image::from_bytes(format, bytes.to_vec())),
        aspect: width as f32 / height as f32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(version: &str) -> Entry {
        Entry {
            version: version.into(),
            date: "Oct 17".into(),
            hero: None,
            hero_visual: None,
            features: Vec::new(),
        }
    }

    #[test]
    fn fresh_install_records_the_current_version_silently() {
        let entries = [entry("0.2.113")];
        assert_eq!(resolve(&entries, "0.2.113", None), Resolution::Advance);
    }

    #[test]
    fn same_or_older_version_does_nothing() {
        let entries = [entry("0.2.113")];
        assert_eq!(
            resolve(&entries, "0.2.113", Some("0.2.113")),
            Resolution::Noop
        );
        assert_eq!(
            resolve(&entries, "0.2.112", Some("0.2.113")),
            Resolution::Noop
        );
    }

    #[test]
    fn upgrade_with_an_entry_shows_it() {
        let entries = [entry("0.2.112"), entry("0.2.113")];
        assert_eq!(
            resolve(&entries, "0.2.113", Some("0.2.99")),
            Resolution::Show(&entries[1])
        );
        assert_eq!(
            resolve(&entries, "0.2.113", Some("0.0.0")),
            Resolution::Show(&entries[1])
        );
    }

    #[test]
    fn upgrade_without_an_entry_advances_silently() {
        let entries = [entry("0.2.112")];
        assert_eq!(
            resolve(&entries, "0.2.113", Some("0.2.112")),
            Resolution::Advance
        );
    }

    #[test]
    fn upgrade_past_an_unseen_entry_shows_the_newest_unseen_one() {
        let entries = [entry("0.2.115"), entry("0.2.113"), entry("0.2.112")];
        assert_eq!(
            resolve(&entries, "0.2.114", Some("0.2.112")),
            Resolution::Show(&entries[1])
        );
    }

    #[test]
    fn bundled_changelog_parses_newest_first_with_every_image() {
        let entries = entries();
        assert!(!entries.is_empty());
        assert_eq!(
            entries[0].version, CURRENT_VERSION,
            "a version bump needs its whats_new.toml entry"
        );
        assert!(
            entries
                .windows(2)
                .all(|pair| version_newer(&pair[0].version, &pair[1].version))
        );
        for entry in entries {
            assert!(
                !entry.features.is_empty(),
                "{} has no features",
                entry.version
            );
            let images = entry
                .hero
                .iter()
                .chain(entry.features.iter().filter_map(|f| f.image.as_ref()));
            for name in images {
                assert!(picture(name).is_some(), "{name} is not a bundled image");
            }
        }
    }
}
