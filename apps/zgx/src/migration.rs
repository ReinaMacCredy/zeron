//! The other interface's (crates/ui's) settings and drafts, carried into
//! this one once, the first time it opens as the Zeron app (src/identity.rs
//! `migrates`), or with `ZERON_ZGX_MIGRATE=1` on a debug build.
//!
//! Every `ui-settings.json` key that interface writes is in [`TABLE`]: where
//! it goes here, or why it has no home. A value this app holds already stays,
//! unless it's this app's default. Older shapes of the file are read too
//! ([`normalized`]). A file that isn't JSON changes nothing and says so; with
//! no marker kept, a launch after it's mended carries it then. Drafts
//! (`composer-drafts.json`) go into the composer's drafts for the run, each
//! with the attachments whose files are still there.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value as Json, json};

use crate::carried_settings;

/// In this app's own folder: what was carried, and when. There, it never runs again.
pub const MARKER: &str = "migrated-from-zeron.json";

/// Where a key of `ui-settings.json` goes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Home {
    /// Into these `ui.persist` keys (`module::name`).
    Persist(&'static [&'static str]),
    /// Read where it is: this app reads and writes `ui-settings.json` itself.
    InPlace(&'static str),
    /// The engine's `theme-choice.json`, which both themes pages share.
    ThemeChoice,
    /// Not carried, and why.
    None(&'static str),
}

const LOOK: &str = "screens/shell/appearance.luau";
const PREFS: &str = "screens/settings/prefs.luau";

/// Every key of crates/ui `UiSettings` (camelCase, as written) and the older ones it still reads.
pub const TABLE: &[(&str, Home)] = &[
    ("dictationEnabled", Home::Persist(&["ds/dictation.luau::dictation_enabled"])),
    ("dictationInput", Home::Persist(&["ds/dictation.luau::dictation_input"])),
    ("windowGeometry", Home::None("the window's frame is the system's (frame autosave), not a file")),
    ("composerSendBehavior", Home::Persist(&["screens/settings/prefs.luau::settings_send_with"])),
    ("followUpBehavior", Home::Persist(&["screens/settings/prefs.luau::settings_follow_up"])),
    ("reasoningShortcutModifier", Home::None("no thinking-level step shortcuts here")),
    ("switcherWindow", Home::Persist(&["screens/shell/switcher.luau::shell_switcher_window"])),
    ("switcherLimit", Home::Persist(&["screens/shell/switcher.luau::shell_switcher_limit"])),
    ("switcherCyclesTabs", Home::Persist(&["screens/shell/switcher.luau::shell_switcher_cycles_tabs"])),
    ("skillsInSlashMenu", Home::Persist(&["screens/settings/prefs.luau::settings_skill_completion"])),
    ("skillCompletionByHarness", Home::Persist(&["screens/settings/prefs.luau::settings_skill_completion"])),
    ("disabledSkills", Home::Persist(&["screens/chat/invocations.luau::skills_disabled"])),
    ("compactModelPicker", Home::Persist(&["ds/model_picker.luau::model_picker_compact"])),
    ("compactEffortSlider", Home::None("one effort slider here, no style to pick")),
    ("computerClickSounds", Home::Persist(&["screens/settings/computer.luau::click_sounds"])),
    ("sidebarWidth", Home::Persist(&["screens/shell/sidebar.luau::shell_sidebar_width"])),
    ("sidebarCollapsed", Home::Persist(&["screens/shell/view.luau::shell_collapsed"])),
    ("sidebarHoverEnabled", Home::Persist(&["screens/shell/view.luau::shell_sidebar_hover"])),
    ("insetShell", Home::Persist(&["ds/look.luau::inset_card"])),
    ("sidebarOrganization", Home::Persist(&["screens/shell/sidebar.luau::shell_sidebar_organize", "screens/shell/sidebar.luau::shell_sidebar_inbox"])),
    ("sidebarSort", Home::Persist(&["screens/shell/sidebar.luau::shell_sidebar_sort"])),
    ("sidebarActiveFirst", Home::Persist(&["screens/shell/sidebar.luau::shell_sidebar_active_first"])),
    ("sidebarWorkingShelf", Home::Persist(&["screens/shell/sidebar.luau::shell_sidebar_show_working"])),
    ("sidebarShowProjectLabel", Home::Persist(&["screens/shell/sidebar.luau::shell_sidebar_show_location"])),
    ("sidebarCompact", Home::Persist(&["screens/shell/sidebar.luau::shell_sidebar_compact"])),
    ("sidebarShowProjectIcon", Home::Persist(&["screens/shell/sidebar.luau::shell_sidebar_show_project_icon"])),
    ("sidebarShowHarness", Home::Persist(&["screens/shell/sidebar.luau::shell_sidebar_show_harness"])),
    ("sidebarShowBranch", Home::Persist(&["screens/shell/sidebar.luau::shell_sidebar_show_branch"])),
    ("sidebarShowPullRequest", Home::Persist(&["screens/shell/sidebar.luau::shell_sidebar_show_pr"])),
    ("sidebarShowProjectFilter", Home::Persist(&["screens/shell/sidebar.luau::shell_sidebar_show_project_filter"])),
    ("sidebarShowArchived", Home::Persist(&["screens/shell/sidebar.luau::shell_sidebar_show_archived"])),
    ("sidebarCollapsedLeads", Home::None("no sub-agent lead rows in the sidebar here")),
    ("whatsNewSeenVersion", Home::None("no What's New card here")),
    ("lastSpaceId", Home::Persist(&["screens/shell/route.luau::shell_project"])),
    ("lastProjectActionBySpaceId", Home::None("no project Actions here")),
    ("openTabs", Home::Persist(&["screens/shell/split.luau::shell_session_tabs"])),
    ("openTabsStable", Home::None("a flag of gpui's own tab pruning")),
    ("collapsedTabGroups", Home::None("no tab groups here")),
    ("spaceFilter", Home::Persist(&["screens/shell/sidebar.luau::shell_sidebar_project"])),
    ("sidebarSectionsByProfile", Home::None("sections are the engine's, for either app")),
    ("sidebarPinnedSessionIdsByProfile", Home::None("pins are the engine's, for either app")),
    ("soundEnabled", Home::Persist(&["screens/settings/prefs.luau::settings_sounds"])),
    ("soundCompletionEnabled", Home::Persist(&["screens/settings/prefs.luau::settings_sound_done"])),
    ("soundInputEnabled", Home::Persist(&["screens/settings/prefs.luau::settings_sound_request"])),
    ("soundAttentionEnabled", Home::Persist(&["screens/settings/prefs.luau::settings_sound_attention"])),
    ("notificationsEnabled", Home::InPlace("the engine's banners read it there (src/settings.rs notifications.*)")),
    ("notificationsBackgroundOnly", Home::InPlace("the engine's banners read it there (src/settings.rs notifications.*)")),
    ("agentUpdateNotifications", Home::InPlace("the engine's banners read it there (src/settings.rs notifications.*)")),
    ("autoUpdate", Home::InPlace("the Zeron app's updater reads it there (src/about.rs)")),
    ("rightPaneWidth", Home::Persist(&["screens/shell/page.luau::shell_files_width", "screens/shell/page.luau::shell_panel_width_chosen"])),
    ("splitRatio", Home::None("a split's ratios are each tab's own here")),
    ("terminalHeight", Home::None("terminals are panel tabs here, no drawer")),
    ("keymap", Home::Persist(&["screens/shell/keys.luau::shell_keymap", "screens/chat/appshots.luau::appshots_shortcut", "ds/dictation.luau::dictation_key"])),
    ("appshotsEnabled", Home::Persist(&["screens/chat/appshots.luau::appshots_enabled"])),
    ("appshotSoundEnabled", Home::Persist(&["screens/chat/appshots.luau::appshots_sound"])),
    ("appshotDestination", Home::Persist(&["screens/chat/appshots.luau::appshots_destination"])),
    ("computerGettingStartedAcknowledged", Home::Persist(&["screens/settings/computer.luau::guide_acknowledged"])),
    ("computerEffortHintDismissed", Home::None("no effort hint on computer use here")),
    ("escapeStopsActiveAgent", Home::Persist(&["screens/settings/prefs.luau::settings_escape_stops"])),
    ("keepAwakeWhileWorking", Home::InPlace("Keep awake reads and writes it there (src/settings.rs keep_awake.*)")),
    ("settingsSection", Home::None("Settings opens where its route was left")),
    ("tasksView", Home::Persist(&["screens/tasks/init.luau::tasks_view"])),
    ("treeView", Home::None("no tree views here")),
    ("appearance", Home::Persist(&["screens/onboarding/state.luau::appearance"])),
    ("onboardingCompleted", Home::InPlace("read there (src/main.rs finished_in_zeron_app)")),
    ("gitHistoryColumns", Home::None("Git History keeps no columns here")),
    ("gitHistoryColumnWidths", Home::None("Git History keeps no columns here")),
    ("gitHistoryColumnOrder", Home::None("Git History keeps no columns here")),
    ("gitHistoryAuthorDisplay", Home::None("Git History keeps no columns here")),
    ("gitCommitPanelWidth", Home::None("no commit panel width here")),
    ("uiFontFamily", Home::Persist(&["ds/look.luau::interface_font"])),
    ("uiFontSize", Home::Persist(&["ds/look.luau::text_size"])),
    ("terminalFontFamily", Home::None("terminals take Code's font here")),
    ("terminalFontSize", Home::None("terminals take Code's size here")),
    ("codeFontFamily", Home::Persist(&["ds/look.luau::code_font"])),
    ("codeFontSize", Home::Persist(&["screens/settings/prefs.luau::settings_code_font_size"])),
    ("filesEditorFontSize", Home::Persist(&["screens/settings/prefs.luau::settings_code_font_size"])),
    ("themeSelection", Home::ThemeChoice),
    ("diffSplit", Home::Persist(&["ds/git.luau::ds.git.diff_split"])),
    ("diffWrap", Home::Persist(&["ds/git.luau::ds.git.diff_wrap"])),
    ("codeFencesFitContent", Home::Persist(&["ds/markdown.luau::markdown_code_fit"])),
    ("renderMath", Home::Persist(&["ds/math.luau::math_render"])),
    ("mathFont", Home::Persist(&["ds/math.luau::math_face"])),
    ("mathSize", Home::Persist(&["ds/math.luau::math_size"])),
    ("transcriptWidth", Home::Persist(&["screens/settings/prefs.luau::settings_conversation_width"])),
    ("openWebLinksInZeron", Home::Persist(&["screens/settings/prefs.luau::settings_open_links"])),
    ("htmlOpenApp", Home::Persist(&["ds/html_card.luau::html_open_app"])),
    ("transcriptCompactMode", Home::Persist(&["screens/settings/prefs.luau::settings_compact_transcript"])),
    ("pullRequestOpenIn", Home::None("pull requests are the GitHub plugin's, which keeps its own")),
    ("pullRequestDestination", Home::None("pull requests are the GitHub plugin's, which keeps its own")),
    ("lastPullRequestRepository", Home::None("pull requests are the GitHub plugin's, which keeps its own")),
    ("lastPullRequestDevice", Home::None("pull requests are the GitHub plugin's, which keeps its own")),
    ("pullRequestsTab", Home::None("pull requests are the GitHub plugin's, which keeps its own")),
    ("pullRequestStars", Home::None("pull requests are the GitHub plugin's, which keeps its own")),
    ("projectFolderPins", Home::None("the New project palette here keeps no pinned folders")),
    ("filesAutosaveEnabled", Home::Persist(&["screens/settings/prefs.luau::settings_files_autosave"])),
    ("filesAutosaveDelayMs", Home::Persist(&["screens/settings/prefs.luau::settings_files_autosave_delay"])),
    ("filesWordWrap", Home::Persist(&["screens/settings/prefs.luau::settings_files_word_wrap"])),
    ("filesShowAll", Home::Persist(&["screens/settings/prefs.luau::settings_files_show_all"])),
    ("sidecarCollapsed", Home::Persist(&["screens/sidecar.luau::sidecar_collapsed"])),
    ("accent", Home::Persist(&["ds/look.luau::accent"])),
    ("accentColor", Home::Persist(&["ds/look.luau::accent"])),
    ("surface", Home::Persist(&["ds/material.luau::surface"])),
    ("frost", Home::Persist(&["ds/material.luau::frost_tint", "ds/material.luau::frost_dim_backdrop", "ds/material.luau::frost_backdrop_brightness"])),
    ("newThreadComposerBackground", Home::Persist(&["screens/shell/appearance.luau::shell_wallpaper_image", "screens/shell/appearance.luau::shell_wallpaper"])),
    ("wallpaperFolder", Home::Persist(&["screens/shell/appearance.luau::shell_wallpaper_folder"])),
    ("wallpaperSource", Home::Persist(&["screens/shell/appearance.luau::shell_wallpaper_source"])),
    ("wallpaperHistory", Home::Persist(&["screens/shell/appearance.luau::shell_wallpaper_history"])),
    ("wallpaperThemeColors", Home::Persist(&["ds/look.luau::wallpaper_theme_colors"])),
    ("wallpaperColor", Home::Persist(&["screens/shell/appearance.luau::shell_wallpaper_color"])),
    ("newThreadBackgroundEffect", Home::Persist(&["screens/shell/appearance.luau::shell_wallpaper_effect"])),
    (
        "chatBackground",
        Home::Persist(&[
            "screens/shell/appearance.luau::shell_chat_background",
            "screens/shell/appearance.luau::shell_chat_background_placement",
            "screens/shell/appearance.luau::shell_chat_background_opacity",
            "screens/shell/appearance.luau::shell_chat_background_blur",
            "screens/shell/appearance.luau::shell_chat_background_scrim",
            "screens/shell/appearance.luau::shell_chat_background_tab_shade",
        ]),
    ),
    ("terminalBackground", Home::None("terminal tabs draw no art here")),
    (
        "shellFrame",
        Home::Persist(&[
            "screens/shell/appearance.luau::shell_frame_style",
            "screens/shell/appearance.luau::shell_frame_shade",
            "screens/shell/appearance.luau::shell_frame_blur",
            "screens/shell/appearance.luau::shell_frame_sidebar",
            "screens/shell/appearance.luau::shell_frame_blend",
        ]),
    ),
    ("reduceMotion", Home::Persist(&["ds/motion.luau::motion_reduce", "ds/motion.luau::motion_follow_system"])),
    ("pauseAnimationsInBackground", Home::Persist(&["ds/motion.luau::motion_pause_background"])),
    ("commitMessageInstruction", Home::None("commit messages are written by the engine's own instruction here")),
    ("commitMessageModel", Home::None("commit messages are written by the engine's own instruction here")),
    ("prChatHistory", Home::None("no pull request chats here")),
    ("prChatIdleExpiry", Home::None("no pull request chats here")),
    ("prChatSessions", Home::None("no pull request chats here")),
    ("gitAgentBusyPolicy", Home::None("no git busy policy here")),
    ("deviceAutoFloat", Home::Persist(&["screens/settings/device_hub.luau::device_auto_float"])),
    ("deviceFloatFrame", Home::None("no saved frame for the device preview here (U28 keeps none)")),
    ("homeWidgets", Home::None("no widget grid here (U27c)")),
    ("smoothScrolling", Home::None("zgx scrolls its own way, no switch")),
    ("codexVoice", Home::Persist(&["screens/settings/voice.luau::codex_voice"])),
    ("codexVoiceDevice", Home::Persist(&["screens/settings/voice.luau::codex_voice_device"])),
];

/// The `ui.persist` file for `key` (`module::name`), as zgx names it.
pub fn file_of(key: &str) -> String {
    let mut out = String::new();
    for (i, byte) in key.bytes().enumerate() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_') || (byte == b'.' && i > 0) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out + ".json"
}

/// This app's own default for `key`'s file, where a value equal to it is no choice of yours.
fn default_of(file: &str) -> Option<Json> {
    DEFAULTS.iter().find(|(key, _)| file_of(key) == file).and_then(|(_, raw)| serde_json::from_str(raw).ok())
}

/// Equal as values, an empty table either way.
fn same(a: &Json, b: &Json) -> bool {
    let empty = |v: &Json| v.as_array().is_some_and(Vec::is_empty) || v.as_object().is_some_and(Map::is_empty);
    a == b || (empty(a) && empty(b)) || a.as_f64().zip(b.as_f64()).is_some_and(|(x, y)| (x - y).abs() < 1e-9)
}

/// The scripts' defaults (`ui.persist(name, default)`), as JSON.
const DEFAULTS: &[(&str, &str)] = &[
    ("screens/shell/sidebar.luau::shell_sidebar_compact", "false"),
    ("screens/shell/sidebar.luau::shell_sidebar_inbox", "false"),
    ("screens/shell/sidebar.luau::shell_sidebar_active_first", "false"),
    ("screens/shell/sidebar.luau::shell_sidebar_organize", "[]"),
    ("screens/shell/sidebar.luau::shell_sidebar_sort", "\"updated\""),
    ("screens/shell/sidebar.luau::shell_sidebar_width", "256"),
    ("screens/shell/sidebar.luau::shell_sidebar_show_branch", "true"),
    ("screens/shell/sidebar.luau::shell_sidebar_show_pr", "true"),
    ("screens/shell/sidebar.luau::shell_sidebar_show_harness", "true"),
    ("screens/shell/sidebar.luau::shell_sidebar_show_project_icon", "true"),
    ("screens/shell/sidebar.luau::shell_sidebar_show_location", "true"),
    ("screens/shell/sidebar.luau::shell_sidebar_show_archived", "true"),
    ("screens/shell/sidebar.luau::shell_sidebar_show_project_filter", "true"),
    ("screens/shell/sidebar.luau::shell_sidebar_show_working", "false"),
    ("screens/shell/sidebar.luau::shell_sidebar_project", "null"),
    ("screens/shell/view.luau::shell_collapsed", "false"),
    ("screens/shell/view.luau::shell_sidebar_hover", "true"),
    ("screens/sidecar.luau::sidecar_collapsed", "false"),
    ("ds/look.luau::inset_card", "true"),
    ("ds/look.luau::wallpaper_theme_colors", "false"),
    ("ds/look.luau::accent", "\"default\""),
    ("ds/look.luau::text_size", "13"),
    ("ds/look.luau::interface_font", "\"\""),
    ("ds/look.luau::code_font", "\"\""),
    ("ds/model_picker.luau::model_picker_compact", "true"),
    ("ds/material.luau::surface", "\"frosted\""),
    ("ds/material.luau::frost_tint", "0.8"),
    ("ds/material.luau::frost_dim_backdrop", "true"),
    ("ds/material.luau::frost_backdrop_brightness", "0.3"),
    ("ds/math.luau::math_render", "true"),
    ("ds/math.luau::math_face", "\"fira\""),
    ("ds/math.luau::math_size", "100"),
    ("ds/html_card.luau::html_open_app", "\"default\""),
    ("ds/git.luau::ds.git.diff_split", "false"),
    ("ds/git.luau::ds.git.diff_wrap", "false"),
    ("ds/markdown.luau::markdown_code_fit", "false"),
    ("ds/motion.luau::motion_reduce", "false"),
    ("ds/motion.luau::motion_follow_system", "true"),
    ("ds/motion.luau::motion_pause_background", "false"),
    ("screens/shell/appearance.luau::shell_wallpaper", "\"none\""),
    ("screens/shell/appearance.luau::shell_wallpaper_image", "null"),
    ("screens/shell/appearance.luau::shell_wallpaper_effect", "\"dither\""),
    ("screens/shell/appearance.luau::shell_frame_style", "\"dark\""),
    ("screens/shell/appearance.luau::shell_frame_shade", "0.58"),
    ("screens/shell/appearance.luau::shell_frame_blur", "28"),
    ("screens/shell/appearance.luau::shell_frame_sidebar", "\"glass\""),
    ("screens/shell/appearance.luau::shell_frame_blend", "\"whole\""),
    ("screens/shell/appearance.luau::shell_chat_background", "false"),
    ("screens/shell/appearance.luau::shell_chat_background_placement", "\"hero\""),
    ("screens/shell/appearance.luau::shell_chat_background_opacity", "0.2"),
    ("screens/shell/appearance.luau::shell_chat_background_blur", "24"),
    ("screens/shell/appearance.luau::shell_chat_background_scrim", "true"),
    ("screens/shell/appearance.luau::shell_chat_background_tab_shade", "0.55"),
    ("screens/shell/appearance.luau::shell_wallpaper_folder", "\"\""),
    ("screens/shell/appearance.luau::shell_wallpaper_source", "\"\""),
    ("screens/shell/appearance.luau::shell_wallpaper_history", "[]"),
    ("screens/shell/appearance.luau::shell_wallpaper_color", "\"\""),
    ("screens/settings/computer.luau::click_sounds", "\"foreground\""),
    ("screens/settings/computer.luau::guide_acknowledged", "false"),
    ("screens/settings/prefs.luau::settings_send_with", "\"enter\""),
    ("screens/settings/prefs.luau::settings_follow_up", "\"queue\""),
    ("screens/settings/prefs.luau::settings_escape_stops", "false"),
    ("screens/settings/prefs.luau::settings_compact_transcript", "false"),
    ("screens/settings/prefs.luau::settings_open_links", "\"zeron\""),
    ("screens/settings/prefs.luau::settings_files_autosave", "false"),
    ("screens/settings/prefs.luau::settings_files_autosave_delay", "900"),
    ("screens/settings/prefs.luau::settings_files_word_wrap", "false"),
    ("screens/settings/prefs.luau::settings_files_show_all", "false"),
    ("screens/settings/prefs.luau::settings_code_font_size", "13"),
    ("screens/settings/prefs.luau::settings_conversation_width", "660"),
    ("screens/settings/prefs.luau::settings_sounds", "true"),
    ("screens/settings/prefs.luau::settings_sound_done", "true"),
    ("screens/settings/prefs.luau::settings_sound_request", "true"),
    ("screens/settings/prefs.luau::settings_sound_attention", "true"),
    ("screens/settings/prefs.luau::settings_skill_completion", "{}"),
    ("screens/shell/switcher.luau::shell_switcher_window", "\"all\""),
    ("screens/shell/switcher.luau::shell_switcher_limit", "0"),
    ("screens/shell/switcher.luau::shell_switcher_cycles_tabs", "false"),
    ("screens/chat/appshots.luau::appshots_enabled", "false"),
    ("screens/chat/appshots.luau::appshots_sound", "true"),
    ("screens/chat/appshots.luau::appshots_destination", "\"automatic\""),
    ("screens/chat/appshots.luau::appshots_shortcut", "\"ctrl+alt+space\""),
    ("screens/chat/invocations.luau::skills_disabled", "{}"),
    ("screens/tasks/init.luau::tasks_view", "\"list\""),
    ("screens/onboarding/state.luau::appearance", "\"system\""),
    ("screens/shell/route.luau::shell_project", "null"),
    ("screens/shell/page.luau::shell_files_width", "660"),
    ("screens/shell/page.luau::shell_panel_width_chosen", "false"),
    ("screens/shell/split.luau::shell_session_tabs", "[]"),
    ("screens/shell/keys.luau::shell_keymap", "{}"),
    ("ds/dictation.luau::dictation_enabled", "false"),
    ("ds/dictation.luau::dictation_input", "\"\""),
    ("ds/dictation.luau::dictation_key", "\"cmd+d\""),
    ("screens/settings/voice.luau::codex_voice", "\"\""),
    ("screens/settings/voice.luau::codex_voice_device", "\"\""),
    ("screens/settings/device_hub.luau::device_auto_float", "true"),
];

/// `settings` read as the shipping app reads older files of it
/// (crates/ui settings.rs `load` and `migrated`, theme.rs's accent names).
pub fn normalized(mut settings: Json) -> Json {
    let Some(map) = settings.as_object_mut() else { return settings };
    if !map.contains_key("appshotSoundEnabled")
        && let Some(sound) = map.get("soundEnabled").and_then(Json::as_bool)
    {
        map.insert("appshotSoundEnabled".into(), json!(sound));
    }
    if let Some(old) = map.remove("filesEditorFontSize") {
        map.entry("codeFontSize").or_insert(old);
    }
    if let Some(old) = map.remove("pullRequestDestination") {
        map.entry("pullRequestOpenIn").or_insert(old);
    }
    if let Some(old) = map.remove("accentColor") {
        let preset = match old.as_str() {
            Some("violet" | "indigo" | "red" | "purple" | "zeron") => Some("zeron"),
            Some("teal" | "cyan") => Some("cyan"),
            Some(name @ ("orange" | "amber" | "green" | "blue" | "pink")) => Some(name),
            _ => None,
        };
        let theirs = map.get("accent").is_none_or(|a| a.as_str() == Some("themeDefault"));
        if let (Some(preset), true) = (preset, theirs) {
            map.insert("accent".into(), json!({ "preset": preset }));
        }
    }
    settings
}

/// What this module carries beside `carried_settings::carried`, as files of `ui.persist`.
fn more(settings: &Json) -> Vec<(String, Json)> {
    let mut out = Vec::new();
    let mut put = |key: &str, value: Json| out.push((file_of(key), value));
    let s = |key: &str| settings[key].as_str();
    if let Some(id) = s("lastSpaceId") {
        put("screens/shell/route.luau::shell_project", json!(id));
    }
    if let Some(tabs) = settings["openTabs"].as_array() {
        let ids: Vec<&str> = tabs.iter().filter_map(Json::as_str).filter(|id| !id.is_empty()).collect();
        if !ids.is_empty() {
            put("screens/shell/split.luau::shell_session_tabs", json!(ids));
        }
    }
    if let Some(w) = settings["rightPaneWidth"].as_f64().filter(|w| w.is_finite() && (*w - 520.0).abs() > 0.5) {
        put("screens/shell/page.luau::shell_files_width", json!(w.max(360.0).round()));
        put("screens/shell/page.luau::shell_panel_width_chosen", json!(true));
    }
    match s("tasksView") {
        Some("list") => put("screens/tasks/init.luau::tasks_view", json!("list")),
        Some("kanban") => put("screens/tasks/init.luau::tasks_view", json!("board")),
        _ => {}
    }
    if let Some(mode @ ("system" | "light" | "dark")) = s("appearance") {
        put("screens/onboarding/state.luau::appearance", json!(mode));
    }
    if let Some(family) = s("uiFontFamily").and_then(|f| font(f, "geist")) {
        put("ds/look.luau::interface_font", json!(family));
    }
    if let Some(family) = s("codeFontFamily").and_then(|f| font(f, "geistMono")) {
        put("ds/look.luau::code_font", json!(family));
    }
    if let Some(px) = settings["uiFontSize"].as_f64().filter(|px| px.is_finite()) {
        put("ds/look.luau::text_size", json!(nearest(px * 13.0 / 16.0, &[12.0, 13.0, 14.0, 15.0, 16.0])));
    }
    if let Some(px) = settings["codeFontSize"].as_f64().filter(|px| px.is_finite()) {
        put("screens/settings/prefs.luau::settings_code_font_size", json!(nearest(px, &[11.0, 12.0, 13.0, 14.0, 15.0, 16.0])));
    }
    for (from, to) in [
        ("diffSplit", "ds/git.luau::ds.git.diff_split"),
        ("diffWrap", "ds/git.luau::ds.git.diff_wrap"),
        ("codeFencesFitContent", "ds/markdown.luau::markdown_code_fit"),
        ("renderMath", "ds/math.luau::math_render"),
    ] {
        if let Some(on) = settings[from].as_bool() {
            put(to, json!(on));
        }
    }
    let face = match s("mathFont") {
        Some("fira") => Some("fira"),
        Some("stixTwo") => Some("stix"),
        Some("computerModern") => Some("computer-modern"),
        Some(f @ ("libertinus" | "pagella" | "euler")) => Some(f),
        _ => None,
    };
    if let Some(face) = face {
        put("ds/math.luau::math_face", json!(face));
    }
    if let Some(size) = settings["mathSize"].as_f64().filter(|n| n.is_finite()) {
        put("ds/math.luau::math_size", json!(nearest(size.round().clamp(75.0, 200.0), &[90.0, 100.0, 110.0, 120.0, 135.0, 150.0])));
    }
    match settings["htmlOpenApp"]["kind"].as_str() {
        Some("systemDefault") => put("ds/html_card.luau::html_open_app", json!("default")),
        Some("zeron") => put("ds/html_card.luau::html_open_app", json!("zeron")),
        _ => {}
    }
    let accent = &settings["accent"];
    let accent = match (accent.as_str(), accent["preset"].as_str()) {
        (Some("themeDefault"), _) => Some("default"),
        (_, Some(p @ ("zeron" | "orange" | "amber" | "green" | "cyan" | "blue" | "pink"))) => Some(p),
        _ => None,
    };
    if let Some(accent) = accent {
        put("ds/look.luau::accent", json!(accent));
    }
    let surface = match s("surface") {
        Some("themeDefault") => Some("theme"),
        Some(v @ ("frosted" | "opaque")) => Some(v),
        _ => None,
    };
    if let Some(surface) = surface {
        put("ds/material.luau::surface", json!(surface));
    }
    let frost = &settings["frost"];
    if let Some(tint) = frost["tint"].as_f64().filter(|n| n.is_finite()) {
        put("ds/material.luau::frost_tint", json!((tint.clamp(0.5, 1.0) * 100.0).round() / 100.0));
    }
    if let Some(dim) = frost["dimBackdrop"].as_bool() {
        put("ds/material.luau::frost_dim_backdrop", json!(dim));
    }
    if let Some(level) = frost["backdropBrightness"].as_f64().filter(|n| n.is_finite()) {
        put("ds/material.luau::frost_backdrop_brightness", json!((level.clamp(0.1, 1.0) * 100.0).round() / 100.0));
    }
    for (from, to) in [("wallpaperFolder", "shell_wallpaper_folder"), ("wallpaperSource", "shell_wallpaper_source")] {
        if let Some(path) = s(from).filter(|p| !p.is_empty()) {
            put(&format!("{LOOK}::{to}"), json!(path));
        }
    }
    if let Some(history) = settings["wallpaperHistory"].as_array() {
        let paths: Vec<&str> = history.iter().filter_map(Json::as_str).take(8).collect();
        if !paths.is_empty() {
            put(&format!("{LOOK}::shell_wallpaper_history"), json!(paths));
        }
    }
    if let Some(completion) = skill_completion(settings) {
        put(&format!("{PREFS}::settings_skill_completion"), completion);
    }
    let (bound, appshot) = keymap(&settings["keymap"]);
    if !bound.is_empty() {
        put("screens/shell/keys.luau::shell_keymap", Json::Object(bound));
    }
    if let Some(combo) = appshot {
        put("screens/chat/appshots.luau::appshots_shortcut", json!(combo));
    }
    if let Some(hold) = settings["keymap"]["toggleDictation"].as_str().filter(|c| !c.is_empty() && *c != "mod-d").and_then(combo) {
        put("ds/dictation.luau::dictation_key", json!(hold));
    }
    for (from, to) in [("dictationEnabled", "ds/dictation.luau::dictation_enabled"), ("deviceAutoFloat", "screens/settings/device_hub.luau::device_auto_float")] {
        if let Some(on) = settings[from].as_bool() {
            put(to, json!(on));
        }
    }
    for (from, to) in [
        ("dictationInput", "ds/dictation.luau::dictation_input"),
        ("codexVoice", "screens/settings/voice.luau::codex_voice"),
        ("codexVoiceDevice", "screens/settings/voice.luau::codex_voice_device"),
    ] {
        if let Some(word) = s(from).filter(|w| !w.is_empty()) {
            put(to, json!(word));
        }
    }
    out
}

/// A font family as this app names it ("" for the theme's own), from the shipping app's word.
fn font(word: &str, own: &str) -> Option<String> {
    if word == own {
        return Some(String::new());
    }
    match word {
        "geist" => Some("Geist".into()),
        "geistMono" => Some("Geist Mono".into()),
        "inter" => Some("Inter".into()),
        "atkinsonHyperlegibleNext" => Some("Atkinson Hyperlegible Next".into()),
        _ => word.strip_prefix("installed:").filter(|name| !name.is_empty()).map(Into::into),
    }
}

fn nearest(value: f64, choices: &[f64]) -> f64 {
    choices.iter().copied().min_by(|a, b| (a - value).abs().total_cmp(&(b - value).abs())).unwrap_or(value)
}

/// The agents that read the old `skillsInSlashMenu` (crates/ui settings.rs SKILL_COMPLETION_HARNESSES).
const COMPLETION_AGENTS: &[&str] = &["antigravity", "claude-code", "codex", "cursor", "devin", "grok", "hermes", "fx", "pi", "opencode"];

fn skill_completion(settings: &Json) -> Option<Json> {
    let mut out = Map::new();
    if settings["skillsInSlashMenu"].as_bool() == Some(true) {
        for agent in COMPLETION_AGENTS {
            out.insert((*agent).into(), json!({ "dollar": true, "separate": false }));
        }
    }
    if let Some(each) = settings["skillCompletionByHarness"].as_object() {
        for (agent, choice) in each {
            if let (Some(dollar), Some(separate)) = (choice["dollar"].as_bool(), choice["separateFromSlash"].as_bool()) {
                out.insert(agent.clone(), json!({ "dollar": dollar, "separate": separate }));
            }
        }
    }
    out.retain(|_, c| c != &json!({ "dollar": true, "separate": true }));
    (!out.is_empty()).then_some(Json::Object(out))
}

/// The shipping app's shortcut ids and this app's commands for them.
const SHORTCUTS: &[(&str, &str)] = &[
    ("toggleSidebar", "shell.toggle_sidebar"),
    ("toggleChanges", "shell.toggle_files"),
    ("toggleFiles", "shell.toggle_files_tab"),
    ("toggleTerminal", "shell.toggle_terminal"),
    ("newSession", "shell.new_chat"),
    ("newProject", "shell.new_project"),
    ("openModelPicker", "shell.model_picker"),
    ("toggleBtw", "chat.side_chat"),
    ("nextSession", "shell.next_session"),
    ("prevSession", "shell.previous_session"),
    ("archiveSession", "shell.archive"),
    ("quickCapture", "shell.quick_capture"),
    ("randomWallpaper", "shell.random_wallpaper"),
    ("browserReload", "panel.reload_page"),
];

/// Its defaults, either platform's spelling (crates/ui settings.rs `default_combo_on`).
fn default_combo(field: &str) -> &'static [&'static str] {
    match field {
        "toggleSidebar" => &["mod-b"],
        "toggleChanges" => &["mod-r"],
        "toggleFiles" => &["mod-e"],
        "toggleTerminal" => &["mod-j"],
        "newSession" => &["mod-n"],
        "newProject" => &["mod-shift-n"],
        "openModelPicker" => &["mod-/"],
        "toggleBtw" => &["mod-;"],
        "nextSession" => &["ctrl-tab", "mod-tab"],
        "prevSession" => &["ctrl-shift-tab", "mod-shift-tab"],
        "archiveSession" => &["mod-shift-a"],
        "quickCapture" => &["mod-alt-n", "mod-shift-space", "alt-space"],
        "randomWallpaper" => &["mod-u"],
        "browserReload" => &["mod-shift-r"],
        "captureAppshot" => &["mod-alt-space", "cmd-cmd", "both-cmd", ""],
        _ => &[],
    }
}

const JUMP_DEFAULTS: [&str; 9] = ["mod-1", "mod-2", "mod-3", "mod-4", "mod-5", "mod-6", "mod-7", "mod-8", "mod-9"];

/// A combination as this app writes it (`cmd+shift+b`), from the shipping app's (`mod-shift-b`).
pub fn combo(theirs: &str) -> Option<String> {
    if theirs.is_empty() {
        return Some(String::new());
    }
    let (mods, key) = match theirs.strip_suffix("--") {
        Some(rest) => (rest, "-"),
        None => theirs.rsplit_once('-').unwrap_or(("", theirs)),
    };
    if key.is_empty() {
        return None;
    }
    let mut out: Vec<&str> = Vec::new();
    for m in mods.split('-').filter(|m| !m.is_empty()) {
        out.push(match m {
            "mod" | "cmd" | "super" => "cmd",
            "ctrl" | "control" => "ctrl",
            "alt" | "option" => "alt",
            "shift" => "shift",
            _ => return None,
        });
    }
    out.push(key);
    Some(out.join("+"))
}

/// The shortcuts the shipping app's keymap rebinds, by this app's command, and Appshots' combination.
fn keymap(map: &Json) -> (Map<String, Json>, Option<String>) {
    let mut bound = Map::new();
    let Some(keys) = map.as_object() else { return (bound, None) };
    // A map from before Save File held Cmd+S and Cmd+B as Toggle sidebar's and Toggle changes' defaults.
    let older = !keys.contains_key("saveFile");
    for (field, command) in SHORTCUTS {
        let Some(theirs) = keys.get(*field).and_then(Json::as_str) else { continue };
        let old_default = older && matches!((*field, theirs), ("toggleSidebar", "mod-s") | ("toggleChanges", "mod-b"));
        if default_combo(field).contains(&theirs) || old_default {
            continue;
        }
        if let Some(ours) = combo(theirs) {
            bound.insert((*command).into(), json!(ours));
        }
    }
    if let Some(jumps) = keys.get("jumpSession").and_then(Json::as_array) {
        for (i, theirs) in jumps.iter().filter_map(Json::as_str).enumerate().take(9) {
            if theirs != JUMP_DEFAULTS[i]
                && let Some(ours) = combo(theirs)
            {
                bound.insert(format!("shell.session_{}", i + 1), json!(ours));
            }
        }
    }
    // A key with modifiers, not macOS's two-Command tap (which this app's recorder can't hold).
    let appshot = keys
        .get("captureAppshot")
        .and_then(Json::as_str)
        .filter(|c| !default_combo("captureAppshot").contains(c) && c.contains('-') && !c.ends_with("cmd"))
        .and_then(combo);
    (bound, appshot)
}

/// What the run found and did, as the marker keeps it.
#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    pub written: Vec<String>,
    pub theme: bool,
    pub drafts: usize,
}

/// Why nothing was carried.
#[derive(Debug, PartialEq)]
pub enum Refused {
    Done,
    Broken(String),
}

/// The run itself: `data` (the engine's, with the other interface's files),
/// `app` (this app's folder, where `ui.persist` and the marker are).
pub fn migrate(data: &Path, app: &Path) -> Result<(Outcome, Json), Refused> {
    let marker = app.join(MARKER);
    if marker.exists() {
        return Err(Refused::Done);
    }
    let raw = match std::fs::read(data.join("ui-settings.json")) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(Refused::Broken(e.to_string())),
    };
    let original = match raw.as_deref().map(serde_json::from_slice::<Json>) {
        Some(Ok(value)) if value.is_object() => value,
        Some(Ok(_)) => return Err(Refused::Broken("not a JSON object".into())),
        Some(Err(e)) => return Err(Refused::Broken(e.to_string())),
        None => json!({}),
    };
    let settings = normalized(original.clone());
    let state = app.join("script-state").join("app.luau");
    let held = |file: &str| match std::fs::read(state.join(file)).ok().and_then(|b| serde_json::from_slice::<Json>(&b).ok()) {
        None => false,
        Some(mine) => default_of(file).is_none_or(|d| !same(&d, &mine)),
    };
    let more = more(&settings);
    // (carried_settings still writes the pins the beta read once; nothing reads them now.)
    let mut carried = settings.clone();
    if let Some(map) = carried.as_object_mut() {
        map.remove("projectFolderPins");
    }
    let pictures = data.join(crate::settings::WALLPAPER_DIR);
    // (A value that is this app's default too is no choice to keep: left unwritten, so a later default follows.)
    let worth = |file: &str, value: &Json| default_of(file).is_none_or(|d| !same(&d, value));
    let written = carried_settings::carry_with(&state, &pictures, &carried, more, &held, &worth);
    let theme = carry_theme(data, &settings["themeSelection"]);
    let drafts = drafts(data, &app.join("migrated-drafts"));
    let outcome = Outcome { written, theme, drafts: drafts.len() };
    let row = |key: &str| TABLE.iter().find(|(k, _)| *k == key).map(|(_, home)| *home);
    let keys: Vec<&String> = original.as_object().map(|m| m.keys().collect()).unwrap_or_default();
    let left: Map<String, Json> = keys
        .iter()
        .filter_map(|k| match row(k) {
            Some(Home::None(why)) => Some(((*k).clone(), json!(why))),
            _ => None,
        })
        .collect();
    let unknown: Vec<&&String> = keys.iter().filter(|k| row(k).is_none()).collect();
    let note = json!({
        "from": data.join("ui-settings.json").display().to_string(),
        "found": raw.is_some(),
        "at": chrono::Utc::now().to_rfc3339(),
        "written": outcome.written,
        "theme": outcome.theme,
        "drafts": outcome.drafts,
        "no_home": left,
        "unknown": unknown,
    });
    std::fs::create_dir_all(app).map_err(|e| Refused::Broken(e.to_string()))?;
    std::fs::write(&marker, serde_json::to_vec_pretty(&note).unwrap_or_default()).map_err(|e| Refused::Broken(e.to_string()))?;
    Ok((outcome, Json::Array(drafts)))
}

/// gpui's theme pick into the engine's `theme-choice.json`, when that holds none.
fn carry_theme(data: &Path, theirs: &Json) -> bool {
    let chosen = zeron_theme::catalog::chosen(data);
    if chosen.light.is_some() || chosen.dark.is_some() {
        return false;
    }
    let pick = |scheme: &str, default: &str| theirs[scheme].as_str().filter(|id| *id != default && !id.is_empty()).map(String::from);
    let (light, dark) = (pick("light", "zeron-light"), pick("dark", "zeron-dark"));
    if light.is_none() && dark.is_none() {
        return false;
    }
    let mut asked = Map::new();
    if let Some(id) = light {
        asked.insert("light".into(), json!(id));
    }
    if let Some(id) = dark {
        asked.insert("dark".into(), json!(id));
    }
    zeron_theme::catalog::choose(data, &Json::Object(asked)).is_ok()
}

/// The other interface's unsent drafts (`composer-drafts.json`), each as this
/// app's composer keeps one: `{ key, text, cargo }`. Staged files are copied
/// into `kept` (the other app removes its own as they go); a linked file, an
/// image or an Appshot whose file is gone is left out. A draft for a chat not
/// yet made (a draft session) goes to the new chat's (""), the newest one.
pub fn drafts(data: &Path, kept: &Path) -> Vec<Json> {
    let Some(file) = std::fs::read(data.join("composer-drafts.json")).ok().and_then(|b| serde_json::from_slice::<Json>(&b).ok()) else {
        return Vec::new();
    };
    let draft_sessions: Vec<(&str, &str)> = file["sessions"]
        .as_array()
        .map(|all| all.iter().filter_map(|s| Some((s["id"].as_str()?, s["createdAt"].as_str().unwrap_or("")))).collect())
        .unwrap_or_default();
    let newest_draft = draft_sessions.iter().max_by_key(|(_, at)| *at).map(|(id, _)| *id);
    let mut out = Vec::new();
    let Some(all) = file["drafts"].as_object() else { return out };
    let mut keys: Vec<&String> = all.keys().collect();
    keys.sort();
    let mut made = 0;
    for key in keys {
        let entry = &all[key];
        let is_draft_session = draft_sessions.iter().any(|(id, _)| id == key);
        let key = if key.is_empty() || Some(key.as_str()) == newest_draft {
            ""
        } else if is_draft_session {
            continue;
        } else {
            key.as_str()
        };
        if key.is_empty() && out.iter().any(|d: &Json| d["key"] == "") {
            continue;
        }
        let mut cargo = Vec::new();
        for a in entry["attachments"].as_array().into_iter().flatten() {
            made += 1;
            if let Some(item) = attachment(a, kept, made) {
                cargo.push(item);
            }
        }
        for shot in entry["appshots"].as_array().into_iter().flatten() {
            made += 1;
            let Some(path) = keep_file(&shot["screenshot"], kept) else { continue };
            let app = shot["appName"].as_str().unwrap_or("App");
            let title = shot["windowTitle"].as_str().filter(|t| !t.is_empty());
            cargo.push(json!({
                "id": format!("migrated{made}"),
                "kind": "appshot",
                "name": title.unwrap_or(app),
                "meta": format!("{app} · Appshot"),
                "src": path, "path": path,
                "appshot": {
                    "app": app, "bundle": shot["bundleIdentifier"], "title": title,
                    "text": shot["accessibilityContent"], "truncated": shot["accessibilityTruncated"].as_bool() == Some(true),
                },
            }));
        }
        let text = entry["prompt"].as_str().unwrap_or("");
        if text.trim().is_empty() && cargo.is_empty() {
            continue;
        }
        for (i, item) in cargo.iter_mut().enumerate() {
            if item["number"].is_null() {
                item["number"] = json!(i + 1);
            }
        }
        out.push(json!({ "key": key, "text": text, "cargo": cargo }));
    }
    out
}

fn attachment(a: &Json, kept: &Path, made: usize) -> Option<Json> {
    let name = a["name"].as_str().unwrap_or("file");
    let linked = a["linked"].as_bool() == Some(true);
    let path = if linked {
        let path = a["path"].as_str()?;
        Path::new(path).is_file().then(|| path.to_string())?
    } else {
        keep_file(a, kept)?
    };
    let picture = a["isFile"].as_bool() != Some(true) && is_picture(&path);
    let mut item = json!({
        "id": format!("migrated{made}"),
        "kind": if picture { "image" } else { "file" },
        "name": name,
        "path": path,
    });
    if picture {
        item["src"] = json!(path);
    }
    if linked {
        item["linked"] = json!(true);
        item["on_host"] = json!(true);
        item["label"] = json!(name);
    }
    if let Some(n) = a["mention"].as_u64() {
        item["number"] = json!(n);
    }
    Some(item)
}

fn is_picture(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".heic", ".bmp", ".tiff"].iter().any(|ext| lower.ends_with(ext))
}

/// A staged file, copied into `kept` under a name of its own; none when it's gone.
fn keep_file(a: &Json, kept: &Path) -> Option<String> {
    let from = PathBuf::from(a["path"].as_str()?);
    if !from.is_file() {
        return None;
    }
    std::fs::create_dir_all(kept).ok()?;
    let name = from.file_name()?.to_string_lossy().into_owned();
    let mut to = kept.join(&name);
    let mut n = 1;
    while to.exists() {
        n += 1;
        to = kept.join(format!("{n}-{name}"));
    }
    std::fs::copy(&from, &to).ok()?;
    Some(to.display().to_string())
}

/// At the main window's opening: the run, if this build migrates (or a debug
/// build is asked to), said in the log. The drafts to seed.
pub fn at_launch(app_id: &str) -> Json {
    let asked = crate::identity::CURRENT.migrates || crate::dev::switch("ZERON_ZGX_MIGRATE").is_some_and(|v| v != "0");
    let (Some(data), Some(app)) = (crate::engine::data_dir(), zgx_script::app_data_dir(app_id)) else {
        return Json::Null;
    };
    if !asked {
        return Json::Null;
    }
    let result = migrate(&data, &app);
    if let Some(line) = said(&data, &result) {
        eprintln!("zeron-zgx: migration: {line}");
    }
    result.map(|(_, drafts)| drafts).unwrap_or(Json::Null)
}

/// The log's line for a run (none once it has run).
fn said(data: &Path, result: &Result<(Outcome, Json), Refused>) -> Option<String> {
    let file = data.join("ui-settings.json");
    match result {
        Ok((outcome, _)) => Some(format!(
            "carried {} settings{} and {} drafts from {}",
            outcome.written.len(),
            if outcome.theme { " and the themes" } else { "" },
            outcome.drafts,
            file.display()
        )),
        Err(Refused::Done) => None,
        Err(Refused::Broken(why)) => Some(format!("{} can't be read ({why}): nothing carried, and it's tried again next launch", file.display())),
    }
}

/// The drafts into the composer's own, before its scripts first read them
/// (screens/chat/shared.luau: `chat.composer.drafts` by chat, the new chat's
/// as the composer's own text and cargo).
pub fn seed_drafts(host: &zgx_script::ScriptHost, drafts: Json) {
    let Some(all) = drafts.as_array().filter(|a| !a.is_empty()) else { return };
    let mut kept = Map::new();
    for d in all {
        let key = d["key"].as_str().unwrap_or("");
        if key.is_empty() {
            host.shared("chat.composer.text", crate::plugins::json_value(&d["text"]));
            host.shared("chat.composer.cargo", crate::plugins::json_value(&d["cargo"]));
        } else {
            kept.insert(key.into(), json!({ "text": d["text"], "cargo": d["cargo"], "quotes": {} }));
        }
    }
    if !kept.is_empty() {
        host.shared("chat.composer.drafts", crate::plugins::json_value(&Json::Object(kept)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/migration");

    /// A data dir holding fixture `name` as its `ui-settings.json` (`$DATA` its own path), and an app folder.
    fn dirs(name: &str) -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let (data, app) = (dir.path().join("data"), dir.path().join("app"));
        std::fs::create_dir_all(data.join("composer-drafts")).unwrap();
        let text = std::fs::read_to_string(Path::new(FIXTURES).join(name)).unwrap();
        std::fs::write(data.join("ui-settings.json"), text.replace("$DATA", &data.display().to_string())).unwrap();
        image::RgbaImage::new(4, 3).save(data.join("art.png")).unwrap();
        (dir, data, app)
    }

    fn state(app: &Path, key: &str) -> Option<Json> {
        let bytes = std::fs::read(app.join("script-state/app.luau").join(file_of(key))).ok()?;
        Some(serde_json::from_slice(&bytes).unwrap())
    }

    fn keys_of(name: &str) -> Vec<String> {
        let text = std::fs::read_to_string(Path::new(FIXTURES).join(name)).unwrap();
        serde_json::from_str::<Json>(&text).unwrap().as_object().unwrap().keys().cloned().collect()
    }

    #[test]
    fn every_key_the_other_interface_writes_has_a_row() {
        let written = keys_of("v0.2.132.json");
        let rows: Vec<&str> = TABLE.iter().map(|(k, _)| *k).collect();
        for key in &written {
            assert!(rows.contains(&key.as_str()), "{key} has no row");
        }
        // What it writes no more: read in older files.
        for key in rows {
            assert!(written.iter().any(|k| k == key) || ["accentColor", "filesEditorFontSize"].contains(&key), "{key} is in no file of today's");
        }
    }

    /// Each home is a key a script reads, and its default here is that script's.
    #[test]
    fn each_home_is_read_by_a_script_at_the_default_kept_here() {
        let ui = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
        let mut homes: Vec<&str> = TABLE.iter().filter_map(|(_, h)| if let Home::Persist(keys) = h { Some(*keys) } else { None }).flatten().copied().collect();
        homes.extend(DEFAULTS.iter().map(|(k, _)| *k));
        for key in homes {
            let (module, name) = key.split_once("::").unwrap();
            let source = std::fs::read_to_string(ui.join(module)).unwrap_or_else(|_| panic!("{module} for {key}"));
            let call = format!("ui.persist(\"{name}\", ");
            let at = source.find(&call).unwrap_or_else(|| panic!("{module} reads no {name}"));
            let Some((_, default)) = DEFAULTS.iter().find(|(k, _)| *k == key) else { panic!("{key} has no default here") };
            let theirs = source[at + call.len()..].split([')', ' ', ',']).next().unwrap();
            if matches!(theirs.chars().next(), Some('"' | 't' | 'f' | '0'..='9')) {
                assert_eq!(theirs, *default, "{key}'s default");
            }
        }
    }

    #[test]
    fn today_s_file_lands_whole() {
        let (_dir, data, app) = dirs("v0.2.132.json");
        // Each key with a home is read into it (written unless it's this app's default too).
        let settings = normalized(serde_json::from_slice(&std::fs::read(data.join("ui-settings.json")).unwrap()).unwrap());
        let mut wanted: Vec<(String, Json)> = carried_settings::carried(&settings);
        wanted.extend(more(&settings));
        wanted.extend(carried_settings::carried_picture(&settings, &data.join("kept")));
        let (outcome, _) = migrate(&data, &app).unwrap();
        for (key, home) in TABLE {
            if let Home::Persist(homes) = home {
                if ["accentColor", "filesEditorFontSize"].contains(key) {
                    continue;
                }
                for home in *homes {
                    let (_, value) = wanted.iter().find(|(name, _)| *name == file_of(home)).unwrap_or_else(|| panic!("{key} → {home} isn't read"));
                    let default = default_of(&file_of(home)).is_some_and(|d| same(&d, value));
                    assert_eq!(state(&app, home).is_some(), !default, "{key} → {home}");
                }
            }
        }
        assert_eq!(outcome.written.len(), outcome.written.iter().collect::<std::collections::BTreeSet<_>>().len());
        let s = |k: &str| state(&app, k).unwrap();
        assert_eq!(s("screens/settings/prefs.luau::settings_send_with"), json!("mod_enter"));
        assert_eq!(s("screens/shell/sidebar.luau::shell_sidebar_organize"), json!(["project"]));
        assert_eq!(s("screens/settings/prefs.luau::settings_conversation_width"), json!(912.0));
        assert_eq!(s("ds/look.luau::accent"), json!("pink"));
        assert_eq!(s("ds/look.luau::interface_font"), json!("IBM Plex Sans"));
        assert_eq!(s("ds/look.luau::code_font"), json!("JetBrains Mono"));
        assert_eq!(s("ds/look.luau::text_size"), json!(15.0));
        assert_eq!(s("screens/settings/prefs.luau::settings_code_font_size"), json!(14.0));
        assert_eq!(s("ds/material.luau::surface"), json!("opaque"));
        assert_eq!(s("ds/material.luau::frost_tint"), json!(0.91));
        assert_eq!(s("ds/math.luau::math_face"), json!("stix"));
        assert_eq!(s("ds/math.luau::math_size"), json!(120.0));
        assert_eq!(s("ds/html_card.luau::html_open_app"), json!("zeron"));
        assert_eq!(s("screens/tasks/init.luau::tasks_view"), json!("board"));
        assert_eq!(s("screens/onboarding/state.luau::appearance"), json!("dark"));
        assert_eq!(s("screens/shell/split.luau::shell_session_tabs"), json!(["chat-a", "chat-b"]));
        assert_eq!(s("screens/shell/page.luau::shell_files_width"), json!(610.0));
        assert_eq!(s("screens/settings/prefs.luau::settings_skill_completion"), json!({ "codex": { "dollar": false, "separate": true } }));
        assert_eq!(
            s("screens/shell/keys.luau::shell_keymap"),
            json!({ "shell.toggle_sidebar": "cmd+shift+b", "shell.toggle_terminal": "cmd+`", "shell.session_9": "ctrl+9" })
        );
        assert_eq!(s("screens/chat/appshots.luau::appshots_shortcut"), json!("cmd+shift+2"));
        // Voice and the device preview (U28's pages).
        assert_eq!(s("ds/dictation.luau::dictation_key"), json!("cmd+shift+d"));
        assert_eq!(s("ds/dictation.luau::dictation_enabled"), json!(true));
        assert_eq!(s("ds/dictation.luau::dictation_input"), json!("BuiltInMicrophoneDevice"));
        assert_eq!(s("screens/settings/voice.luau::codex_voice"), json!("juniper"));
        assert_eq!(s("screens/settings/device_hub.luau::device_auto_float"), json!(false));
        // The theme, in the engine's own file both themes pages read.
        assert!(outcome.theme);
        let chosen = zeron_theme::catalog::chosen(&data);
        assert_eq!((chosen.light, chosen.dark.as_deref()), (None, Some("github-dark")));
        // Pins nothing reads any more aren't written.
        assert!(state(&app, "screens/shell/add_project.luau::shell_add_project_pins").is_none());
        // What has no home is named in the marker, with why.
        let marker: Json = serde_json::from_slice(&std::fs::read(app.join(MARKER)).unwrap()).unwrap();
        assert_eq!(marker["no_home"]["splitRatio"], json!("a split's ratios are each tab's own here"));
        assert_eq!(marker["unknown"], json!([]));
    }

    #[test]
    fn older_files_are_read_as_the_shipping_app_reads_them() {
        let (_dir, data, app) = dirs("v0.2.100.json");
        migrate(&data, &app).unwrap();
        let s = |k: &str| state(&app, k);
        // `accentColor` "violet" was Zeron's; the editor's size became Code's; the Appshot chime followed the chimes.
        assert_eq!(s("ds/look.luau::accent"), Some(json!("zeron")));
        assert_eq!(s("screens/settings/prefs.luau::settings_code_font_size"), Some(json!(15.0)));
        assert_eq!(s("screens/chat/appshots.luau::appshots_sound"), Some(json!(false)));
        assert_eq!(s("ds/look.luau::interface_font"), Some(json!("Inter")));
        assert_eq!(s("ds/look.luau::text_size"), Some(json!(12.0)));
        assert_eq!(s("screens/shell/sidebar.luau::shell_sidebar_inbox"), Some(json!(true)));
        // Clamped as the conversation's width is here.
        assert_eq!(s("screens/settings/prefs.luau::settings_conversation_width"), Some(json!(1200.0)));
        // "big" isn't a size: its default stays.
        assert_eq!(s("ds/math.luau::math_size"), None);
        // The slash-menu flag of old: every agent's skills in the `/` menu.
        let completion = s("screens/settings/prefs.luau::settings_skill_completion").unwrap();
        assert_eq!(completion["codex"], json!({ "dollar": true, "separate": false }));
        assert_eq!(completion.as_object().unwrap().len(), COMPLETION_AGENTS.len());
        // A map from before Save File: Cmd+S and Cmd+B were its defaults, Cmd+T a rebind.
        assert_eq!(s("screens/shell/keys.luau::shell_keymap"), Some(json!({ "shell.new_chat": "cmd+t" })));
        let marker: Json = serde_json::from_slice(&std::fs::read(app.join(MARKER)).unwrap()).unwrap();
        assert_eq!(marker["no_home"]["pullRequestDestination"], json!("pull requests are the GitHub plugin's, which keeps its own"));

        let (_dir, data, app) = dirs("v0.2.120.json");
        migrate(&data, &app).unwrap();
        let s = |k: &str| state(&app, k);
        // The theme's own accent beside an old name: the old name, as crates/ui settings.rs `migrated` reads it ("teal" was cyan).
        assert_eq!(s("ds/look.luau::accent"), Some(json!("cyan")));
        assert_eq!(s("ds/material.luau::surface"), Some(json!("theme")));
        assert_eq!(s("ds/material.luau::frost_tint"), Some(json!(0.5)));
        assert_eq!(s("ds/material.luau::frost_backdrop_brightness"), Some(json!(1.0)));
        assert_eq!(s("ds/look.luau::code_font"), Some(json!("Geist")));
        // Geist is this app's own interface font, and 100% its own math size: its defaults, left so.
        assert_eq!(s("ds/look.luau::interface_font"), None);
        assert_eq!(s("ds/math.luau::math_face"), None, "a face this app doesn't know");
        assert_eq!(s("ds/math.luau::math_size"), None);
        assert_eq!(s("ds/html_card.luau::html_open_app"), None, "another app to open with isn't one of this app's two");
        assert!(!zeron_theme::catalog::chosen(&data).dark.is_some(), "the default themes aren't a choice");
    }

    #[test]
    fn a_value_of_yours_stays_and_a_default_doesn_t() {
        let (_dir, data, app) = dirs("v0.2.132.json");
        let state_dir = app.join("script-state/app.luau");
        std::fs::create_dir_all(&state_dir).unwrap();
        let put = |k: &str, v: &str| std::fs::write(state_dir.join(file_of(k)), v).unwrap();
        // Yours: kept. This app's default: the other's value replaces it.
        put("screens/shell/sidebar.luau::shell_sidebar_sort", "\"updated\"");
        put("ds/look.luau::accent", "\"green\"");
        put("screens/shell/sidebar.luau::shell_sidebar_compact", "false");
        put("screens/settings/prefs.luau::settings_conversation_width", "660");
        // One of the frame's own: the frame is left whole.
        put("screens/shell/appearance.luau::shell_frame_shade", "0.7");
        migrate(&data, &app).unwrap();
        let s = |k: &str| state(&app, k).unwrap();
        assert_eq!(s("screens/shell/sidebar.luau::shell_sidebar_sort"), json!("created"), "this app's default gives way");
        assert_eq!(s("ds/look.luau::accent"), json!("green"));
        assert_eq!(s("screens/shell/sidebar.luau::shell_sidebar_compact"), json!(true));
        assert_eq!(s("screens/settings/prefs.luau::settings_conversation_width"), json!(912.0));
        assert_eq!(s("screens/shell/appearance.luau::shell_frame_shade"), json!(0.7));
        assert!(state(&app, "screens/shell/appearance.luau::shell_frame_style").is_none());
    }

    #[test]
    fn it_runs_once() {
        let (_dir, data, app) = dirs("v0.2.132.json");
        assert!(migrate(&data, &app).is_ok());
        assert!(app.join(MARKER).is_file());
        std::fs::write(data.join("ui-settings.json"), r#"{ "sidebarCompact": false, "accent": { "preset": "blue" } }"#).unwrap();
        std::fs::remove_file(app.join("script-state/app.luau").join(file_of("ds/look.luau::accent"))).unwrap();
        assert_eq!(migrate(&data, &app).unwrap_err(), Refused::Done);
        assert_eq!(state(&app, "screens/shell/sidebar.luau::shell_sidebar_compact"), Some(json!(true)));
        assert_eq!(state(&app, "ds/look.luau::accent"), None);
        assert_eq!(said(&data, &Err(Refused::Done)), None, "nothing said once it has run");
    }

    #[test]
    fn a_broken_file_changes_nothing_and_says_so() {
        let (_dir, data, app) = dirs("broken.json");
        let result = migrate(&data, &app);
        assert!(matches!(result, Err(Refused::Broken(_))));
        assert!(!app.exists(), "no setting, no marker: a mended file is carried next launch");
        let line = said(&data, &result).unwrap();
        assert!(line.contains("ui-settings.json can't be read") && line.contains("nothing carried"), "{line}");
        // Mended, it's carried.
        std::fs::write(data.join("ui-settings.json"), r#"{ "sidebarCompact": true }"#).unwrap();
        migrate(&data, &app).unwrap();
        assert_eq!(state(&app, "screens/shell/sidebar.luau::shell_sidebar_compact"), Some(json!(true)));
    }

    #[test]
    fn an_empty_file_or_none_carries_nothing_and_is_done() {
        let (_dir, data, app) = dirs("empty.json");
        let (outcome, drafts) = migrate(&data, &app).unwrap();
        assert_eq!((outcome.written.len(), outcome.theme, drafts), (0, false, json!([])));
        assert!(app.join(MARKER).is_file());
        let (_dir, data, app) = dirs("empty.json");
        std::fs::remove_file(data.join("ui-settings.json")).unwrap();
        assert!(migrate(&data, &app).unwrap().0.written.is_empty());
    }

    #[test]
    fn drafts_come_across_with_the_files_still_there() {
        let (_dir, data, app) = dirs("empty.json");
        let text = std::fs::read_to_string(Path::new(FIXTURES).join("composer-drafts.json")).unwrap();
        std::fs::write(data.join("composer-drafts.json"), text.replace("$DATA", &data.display().to_string())).unwrap();
        image::RgbaImage::new(2, 2).save(data.join("composer-drafts/shot.png")).unwrap();
        image::RgbaImage::new(2, 2).save(data.join("composer-drafts/safari.png")).unwrap();
        let (outcome, drafts) = migrate(&data, &app).unwrap();
        assert_eq!(outcome.drafts, 3);
        let by = |key: &str| drafts.as_array().unwrap().iter().find(|d| d["key"] == key).cloned();
        let chat = by("chat-a").unwrap();
        assert_eq!(chat["text"], json!("Look at [Image 1] and the log"));
        let cargo = chat["cargo"].as_array().unwrap();
        // The picture and the Appshot; the one gone and the linked log (not there) are left out.
        assert_eq!(cargo.len(), 2);
        assert_eq!((cargo[0]["kind"].as_str(), cargo[0]["number"].as_u64()), (Some("image"), Some(1)));
        let copy = Path::new(cargo[0]["path"].as_str().unwrap());
        assert!(copy.starts_with(app.join("migrated-drafts")) && copy.is_file(), "a copy of its own");
        assert_eq!(cargo[1]["kind"], json!("appshot"));
        assert_eq!(cargo[1]["appshot"]["text"], json!("Zeron — the page"));
        assert_eq!(by("side-1").unwrap()["text"], json!("a side chat's words"));
        // The newest chat not yet made is the new chat's; an older one, and an empty one, aren't kept.
        assert_eq!(by("").unwrap()["text"], json!("the newest new chat"));
        assert!(by("draft-old").is_none() && by("chat-empty").is_none());
        // A linked file that's there stays linked, by its own path.
        std::fs::write(data.join("build.log"), "ok").unwrap();
        let again = super::drafts(&data, &app.join("again"));
        let linked = again[0]["cargo"].as_array().unwrap().iter().find(|c| c["linked"] == json!(true)).cloned().unwrap();
        assert_eq!(linked["path"], json!(data.join("build.log").display().to_string()));
    }

    #[test]
    fn combinations_read_as_this_app_writes_them() {
        assert_eq!(combo("mod-shift-b").as_deref(), Some("cmd+shift+b"));
        assert_eq!(combo("ctrl-alt-space").as_deref(), Some("ctrl+alt+space"));
        assert_eq!(combo("mod--").as_deref(), Some("cmd+-"));
        assert_eq!(combo("").as_deref(), Some(""));
        assert_eq!(combo("hyper-k"), None);
    }
}
