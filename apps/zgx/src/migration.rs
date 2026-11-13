//! The other interface's (crates/ui's) settings and drafts, carried into
//! this one once, the first time it opens as the Zeron app (src/identity.rs
//! `migrates`), or with `ZERON_ZGX_MIGRATE=1` on a debug build.
//!
//! Every `ui-settings.json` key that interface writes is in [`TABLE`]: where
//! it goes here, or why it has no home. A value this app holds already stays,
//! unless it's this app's default. Older shapes of the file are read too
//! ([`normalized`]). A file that isn't JSON changes nothing and says so; with
//! no marker kept, a launch after it's mended carries it then. Drafts
//! (`composer-drafts.json`) go into the composer's drafts, each with the
//! attachments whose files are still there, and are kept in [`PENDING`] until
//! each is sent or cleared: put back at every launch till then.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value as Json, json};
use zgx_script::Value;

use crate::carried_settings;

/// In this app's own folder: what was carried, and when. There, it never runs again.
pub const MARKER: &str = "migrated-from-zeron.json";

/// In this app's own folder: the carried drafts not yet sent or cleared.
pub const PENDING: &str = "migrated-drafts.json";

/// Whose defaults and spellings the other interface's keymap has (crates/ui `default_combo_on`).
const MAC: bool = cfg!(target_os = "macos");

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
    ("dictationEnabled", Home::Persist(&["screens/settings/prefs.luau::settings_dictation"])),
    ("dictationInput", Home::Persist(&["screens/settings/prefs.luau::settings_dictation_input"])),
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
    ("whatsNewSeenVersion", Home::InPlace("What's New reads and writes it there (src/whats_new.rs)")),
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
    ("autoUpdate", Home::None("not followed: a found release installs by itself only with ZERON_AUTO_UPDATE, till the feed tells this app's releases from the other's")),
    ("rightPaneWidth", Home::Persist(&["screens/shell/page.luau::shell_files_width", "screens/shell/page.luau::shell_panel_width_chosen"])),
    ("splitRatio", Home::None("a split's ratios are each tab's own here")),
    ("terminalHeight", Home::None("terminals are panel tabs here, no drawer")),
    ("keymap", Home::Persist(&["screens/shell/keys.luau::shell_keymap"])),
    ("appshotsEnabled", Home::InPlace("Settings → Appshots reads and writes it there (src/appshots Prefs)")),
    ("appshotSoundEnabled", Home::InPlace("Settings → Appshots reads and writes it there (src/appshots Prefs)")),
    ("appshotDestination", Home::InPlace("Settings → Appshots reads and writes it there (src/appshots Prefs)")),
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
    ("deviceFloatFrame", Home::Persist(&["screens/device/float.luau::device_float_frame"])),
    ("homeWidgets", Home::None("no widget grid here (U27c)")),
    ("smoothScrolling", Home::Persist(&["ds/motion.luau::smooth_scrolling"])),
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
    ("screens/shell/sidebar.luau::shell_sidebar_width", "272"),
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
    ("ds/motion.luau::smooth_scrolling", "true"),
    ("screens/device/float.luau::device_float_frame", "null"),
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
    ("screens/chat/invocations.luau::skills_disabled", "{}"),
    ("screens/tasks/init.luau::tasks_view", "\"list\""),
    ("screens/onboarding/state.luau::appearance", "\"system\""),
    ("screens/shell/route.luau::shell_project", "null"),
    ("screens/shell/page.luau::shell_files_width", "660"),
    ("screens/shell/page.luau::shell_panel_width_chosen", "false"),
    ("screens/shell/split.luau::shell_session_tabs", "[]"),
    ("screens/shell/keys.luau::shell_keymap", "{}"),
    ("screens/settings/prefs.luau::settings_dictation", "false"),
    ("screens/settings/prefs.luau::settings_dictation_input", "\"system\""),
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
        put("screens/settings/prefs.luau::settings_code_font_size", json!(at_or_above(px, &CODE_SIZES)));
    }
    for (from, to) in [
        ("diffSplit", "ds/git.luau::ds.git.diff_split"),
        ("diffWrap", "ds/git.luau::ds.git.diff_wrap"),
        ("codeFencesFitContent", "ds/markdown.luau::markdown_code_fit"),
        ("renderMath", "ds/math.luau::math_render"),
        ("smoothScrolling", "ds/motion.luau::smooth_scrolling"),
    ] {
        if let Some(on) = settings[from].as_bool() {
            put(to, json!(on));
        }
    }
    let frame = &settings["deviceFloatFrame"];
    let side = |key: &str| frame[key].as_f64().filter(|n| n.is_finite());
    if let (Some(x), Some(y), Some(width), Some(height)) = (side("x"), side("y"), side("width"), side("height")) {
        put("screens/device/float.luau::device_float_frame", json!({ "x": x, "y": y, "width": width, "height": height }));
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
    let mut keys = keymap(&settings["keymap"], MAC);
    // (Hold to dictate is the shell's `shell.dictate` here.)
    if let Some(hold) = settings["keymap"]["toggleDictation"].as_str().filter(|c| *c != "mod-d").and_then(combo) {
        keys.bound.insert("shell.dictate".into(), json!(hold));
    }
    if !keys.bound.is_empty() {
        put("screens/shell/keys.luau::shell_keymap", Json::Object(keys.bound));
    }
    for (from, to) in [("dictationEnabled", "screens/settings/prefs.luau::settings_dictation"), ("deviceAutoFloat", "screens/settings/device_hub.luau::device_auto_float")] {
        if let Some(on) = settings[from].as_bool() {
            put(to, json!(on));
        }
    }
    for (from, to) in [
        ("dictationInput", "screens/settings/prefs.luau::settings_dictation_input"),
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

/// This app's Code sizes.
const CODE_SIZES: [f64; 6] = [11.0, 12.0, 13.0, 14.0, 15.0, 16.0];

/// The first of `choices` (ascending) at or above `value`, else the largest:
/// code never comes out smaller than it was (gpui's default 12.5 is 13 here).
fn at_or_above(value: f64, choices: &[f64]) -> f64 {
    choices.iter().copied().find(|c| *c >= value - 1e-9).or_else(|| choices.last().copied()).unwrap_or(value)
}

/// What the other interface's file holds that has a home here but can't land
/// in it, by `key` (`keymap.<id>` for a shortcut): the value, and why.
fn left_out(settings: &Json, mac: bool) -> Map<String, Json> {
    let mut left = keymap(&settings["keymap"], mac).left;
    let mut say = |key: &str, value: &Json, why: &str| {
        left.insert(key.into(), json!({ "value": value, "why": why }));
    };
    for (key, own) in [("uiFontFamily", "geist"), ("codeFontFamily", "geistMono")] {
        if let Some(word) = settings[key].as_str().filter(|w| font(w, own).is_none()) {
            say(key, &json!(word), "a font this app doesn't offer: its own stays");
        }
    }
    if let Some(face) = settings["mathFont"].as_str().filter(|f| !matches!(*f, "fira" | "stixTwo" | "computerModern" | "libertinus" | "pagella" | "euler")) {
        say("mathFont", &json!(face), "a math face this app doesn't offer: its own stays");
    }
    if let Some(kind) = settings["htmlOpenApp"]["kind"].as_str().filter(|k| !matches!(*k, "systemDefault" | "zeron")) {
        say("htmlOpenApp", &settings["htmlOpenApp"], &format!("{kind}: this app opens HTML in Zeron or the system's app only"));
    }
    left
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

/// Its defaults on the Mac (`mac`) or elsewhere, in either platform's
/// spelling, and older ones (crates/ui settings.rs `default_combo_on`,
/// `default_quick_capture`).
fn default_combo(field: &str, mac: bool) -> &'static [&'static str] {
    match field {
        "quickCapture" if mac => &["option-option", "mod-alt-n", "mod-shift-space", "alt-space"],
        "quickCapture" => &["none", "mod-alt-n", "mod-shift-space", "alt-space"],
        "captureAppshot" if mac => &["command-command"],
        "captureAppshot" => &["mod-alt-space", ""],
        "saveFile" => &["mod-s"],
        "toggleTray" => &["mod-i"],
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
        "randomWallpaper" => &["mod-u"],
        "browserReload" => &["mod-shift-r"],
        _ => &[],
    }
}

const JUMP_DEFAULTS: [&str; 9] = ["mod-1", "mod-2", "mod-3", "mod-4", "mod-5", "mod-6", "mod-7", "mod-8", "mod-9"];

/// The macOS double-tap chords the other interface's Appshot and Quick
/// capture menus offer (crates/ui appshots `ChordModifier`), and "none".
const CHORDS: [&str; 3] = ["command-command", "option-option", "shift-shift"];
const NO_HOTKEY: &str = "none";

/// A combination as this app writes it (`cmd+shift+b`), from the shipping
/// app's (`mod-shift-b`); none for one this app can't bind (a modifier
/// alone, as a double-tap chord is, or "none").
pub fn combo(theirs: &str) -> Option<String> {
    if theirs.is_empty() {
        return Some(String::new());
    }
    let (mods, key) = match theirs.strip_suffix("--") {
        Some(rest) => (rest, "-"),
        None => theirs.rsplit_once('-').unwrap_or(("", theirs)),
    };
    if key.is_empty() || matches!(key, "mod" | "cmd" | "command" | "super" | "ctrl" | "control" | "alt" | "option" | "shift" | "fn" | NO_HOTKEY) {
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

/// What the shipping app's keymap rebinds: by this app's command, and what
/// can't be bound here (by `keymap.<id>`, with why).
#[derive(Debug, Default, PartialEq)]
struct Keys {
    bound: Map<String, Json>,
    left: Map<String, Json>,
}

const UNBINDABLE: &str = "a double-tap chord (or a modifier alone), which this app's shortcuts can't hold: its own stays";

/// The keymap as the Mac's (`mac`) or another platform's other interface wrote it.
fn keymap(map: &Json, mac: bool) -> Keys {
    let mut keys = Keys::default();
    let Some(theirs_all) = map.as_object() else { return keys };
    let mut leave = |field: &str, theirs: &str, why: &str| {
        keys.left.insert(format!("keymap.{field}"), json!({ "value": theirs, "why": why }));
    };
    // A map from before Save File held Cmd+S and Cmd+B as Toggle sidebar's and Toggle changes' defaults.
    let older = !theirs_all.contains_key("saveFile");
    for (field, command) in SHORTCUTS {
        let Some(theirs) = theirs_all.get(*field).and_then(Json::as_str) else { continue };
        let old_default = older && matches!((*field, theirs), ("toggleSidebar", "mod-s") | ("toggleChanges", "mod-b"));
        // (A Quick capture chord on the Mac: Settings → Appshots reads it in place.)
        let chord_in_place = mac && *field == "quickCapture" && CHORDS.contains(&theirs);
        if default_combo(field, mac).contains(&theirs) || old_default || chord_in_place {
            continue;
        }
        // (A shortcut cleared there: none here either.)
        let ours = if theirs == NO_HOTKEY { Some(String::new()) } else { combo(theirs) };
        match ours {
            Some(ours) => {
                keys.bound.insert((*command).into(), json!(ours));
            }
            None => leave(field, theirs, UNBINDABLE),
        }
    }
    if let Some(jumps) = theirs_all.get("jumpSession").and_then(Json::as_array) {
        for (i, theirs) in jumps.iter().filter_map(Json::as_str).enumerate().take(9) {
            if theirs == JUMP_DEFAULTS[i] {
                continue;
            }
            match combo(theirs) {
                Some(ours) => {
                    keys.bound.insert(format!("shell.session_{}", i + 1), json!(ours));
                }
                None => leave(&format!("jumpSession.{}", i + 1), theirs, UNBINDABLE),
            }
        }
    }
    for field in ["saveFile", "toggleTray"] {
        if let Some(theirs) = theirs_all.get(field).and_then(Json::as_str).filter(|c| !default_combo(field, mac).contains(c)) {
            leave(field, theirs, "no such command here to bind");
        }
    }
    if let Some(theirs) = theirs_all.get("toggleDictation").and_then(Json::as_str).filter(|c| combo(c).is_none()) {
        leave("toggleDictation", theirs, UNBINDABLE);
    }
    keys
}

/// What the run found and did, as the marker keeps it.
#[derive(Debug, Default, PartialEq)]
pub struct Outcome {
    pub written: Vec<String>,
    pub theme: bool,
    pub drafts: usize,
    /// The drafts not carried, by the other interface's key, with why.
    pub drafts_left: Map<String, Json>,
}

/// Why the run didn't finish.
#[derive(Debug, PartialEq)]
pub enum Refused {
    /// It ran before (the marker is there).
    Done,
    /// The other interface's `ui-settings.json` can't be read: nothing carried.
    Broken(String),
    /// Carried, but the marker couldn't be written: it runs again next launch.
    Marker(String),
    Partial(String),
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
    let unreadable = std::cell::RefCell::new(Vec::<String>::new());
    let held = |file: &str| match std::fs::read(state.join(file)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => {
            unreadable.borrow_mut().push(file.to_owned());
            true
        }
        Ok(bytes) => match serde_json::from_slice::<Json>(&bytes) {
            Err(_) => false,
            Ok(mine) => default_of(file).is_none_or(|d| !same(&d, &mine)),
        },
    };
    let more = more(&settings);
    // (carried_settings still writes the pins the beta read once; nothing reads them now.)
    let mut carried = settings.clone();
    if let Some(map) = carried.as_object_mut() {
        map.remove("projectFolderPins");
    }
    let pictures = data.join(crate::settings::WALLPAPER_DIR);
    // (A value that is this app's default too is no choice to keep: left unwritten, so a later default follows.)
    let attempted = std::cell::RefCell::new(Vec::<String>::new());
    let worth = |file: &str, value: &Json| {
        let worth = default_of(file).is_none_or(|d| !same(&d, value));
        if worth {
            attempted.borrow_mut().push(file.to_owned());
        }
        worth
    };
    let mut trouble = Vec::new();
    if let Err(e) = std::fs::create_dir_all(&state) {
        trouble.push(format!("{} can't be made ({e})", state.display()));
    }
    let written = carried_settings::carry_with(&state, &pictures, &carried, more, &held, &worth);
    let mut failed: Vec<String> = attempted.into_inner().into_iter().chain(unreadable.into_inner()).filter(|name| !written.contains(name)).collect();
    failed.sort();
    failed.dedup();
    if !failed.is_empty() {
        let more = failed.len().saturating_sub(5);
        let named = failed[..failed.len() - more].join(", ");
        trouble.push(match more {
            0 => format!("{} settings couldn't be read or written ({named})", failed.len()),
            more => format!("{} settings couldn't be read or written ({named} and {more} more)", failed.len()),
        });
    }
    let theme = carry_theme(data, &settings["themeSelection"]);
    let (drafts, drafts_left) = drafts(data, &app.join("migrated-drafts")).unwrap_or_else(|why| {
        trouble.push(why);
        (Vec::new(), Map::new())
    });
    let outcome = Outcome { written, theme, drafts: drafts.len(), drafts_left };
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
        "drafts_left": outcome.drafts_left,
        "no_home": left,
        "not_carried": left_out(&settings, MAC),
        "unknown": unknown,
    });
    let failed = |e: std::io::Error| Refused::Marker(e.to_string());
    std::fs::create_dir_all(app).map_err(failed)?;
    keep_pending(app, &drafts).map_err(failed)?;
    if !trouble.is_empty() {
        return Err(Refused::Partial(trouble.join("; ")));
    }
    std::fs::write(&marker, serde_json::to_vec_pretty(&note).unwrap_or_default()).map_err(failed)?;
    Ok((outcome, Json::Array(drafts)))
}

/// The carried drafts not yet sent or cleared, as [`PENDING`] keeps them (none: the file goes).
fn keep_pending(app: &Path, drafts: &[Json]) -> std::io::Result<()> {
    let file = app.join(PENDING);
    if drafts.is_empty() {
        return match std::fs::remove_file(&file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    std::fs::write(file, serde_json::to_vec_pretty(drafts).unwrap_or_default())
}

/// [`PENDING`]'s drafts, at a launch after the one that carried them.
pub fn pending(app: &Path) -> Vec<Json> {
    std::fs::read(app.join(PENDING))
        .ok()
        .and_then(|b| serde_json::from_slice::<Json>(&b).ok())
        .and_then(|d| d.as_array().cloned())
        .unwrap_or_default()
}

/// [`PENDING`] less the drafts `held` no longer holds (sent or cleared); whether any went.
fn prune(app: &Path, drafts: &mut Vec<Json>, held: impl Fn(&str) -> bool) -> bool {
    let before = drafts.len();
    drafts.retain(|d| held(d["key"].as_str().unwrap_or("")));
    if drafts.len() == before {
        return false;
    }
    if let Err(e) = keep_pending(app, drafts) {
        eprintln!("zeron-zgx: migration: {} couldn't be updated: {e}", app.join(PENDING).display());
    }
    true
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
pub fn drafts(data: &Path, kept: &Path) -> Result<(Vec<Json>, Map<String, Json>), String> {
    let mut left = Map::new();
    let file = match std::fs::read(data.join("composer-drafts.json")) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), left)),
        Err(e) => return Err(format!("composer-drafts.json can't be read ({e})")),
        Ok(bytes) => match serde_json::from_slice::<Json>(&bytes) {
            Ok(file) => file,
            Err(e) => {
                left.insert("composer-drafts.json".into(), json!(format!("can't be read ({e}): none carried")));
                return Ok((Vec::new(), left));
            }
        },
    };
    let draft_sessions: Vec<(&str, &str)> = file["sessions"]
        .as_array()
        .map(|all| all.iter().filter_map(|s| Some((s["id"].as_str()?, s["createdAt"].as_str().unwrap_or("")))).collect())
        .unwrap_or_default();
    let newest_draft = draft_sessions.iter().max_by_key(|(_, at)| *at).map(|(id, _)| *id);
    let mut out = Vec::new();
    let Some(all) = file["drafts"].as_object() else { return Ok((out, left)) };
    let mut keys: Vec<&String> = all.keys().collect();
    keys.sort();
    let mut made = 0;
    for theirs in keys {
        let entry = &all[theirs];
        let worth = !entry["prompt"].as_str().unwrap_or("").trim().is_empty()
            || entry["attachments"].as_array().is_some_and(|a| !a.is_empty())
            || entry["appshots"].as_array().is_some_and(|a| !a.is_empty());
        let is_draft_session = draft_sessions.iter().any(|(id, _)| id == theirs);
        let key = if theirs.is_empty() || Some(theirs.as_str()) == newest_draft {
            ""
        } else if is_draft_session {
            if worth {
                left.insert(theirs.clone(), json!("an older new chat's draft: only the newest new chat's comes over"));
            }
            continue;
        } else {
            theirs.as_str()
        };
        if key.is_empty() && out.iter().any(|d: &Json| d["key"] == "") {
            if worth {
                left.insert(theirs.clone(), json!("another new chat's draft holds the new chat's place"));
            }
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
            if worth {
                left.insert(theirs.clone(), json!("only files, and they're gone"));
            }
            continue;
        }
        // (Numbered past its mentions, so no two chips share a number.)
        let mut next = cargo.iter().filter_map(|item| item["number"].as_u64()).max().unwrap_or(0);
        for item in cargo.iter_mut() {
            if item["number"].is_null() {
                next += 1;
                item["number"] = json!(next);
            }
        }
        out.push(json!({ "key": key, "text": text, "cargo": cargo }));
    }
    Ok((out, left))
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

/// A picture the composer draws (screens/chat/shared.luau `is_image`: zgx decodes no GIF or HEIC yet).
fn is_picture(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    [".png", ".jpg", ".jpeg", ".webp"].iter().any(|ext| lower.ends_with(ext))
}

/// A staged file, copied into `kept` under a name of its own (the copy made
/// before, by a run that didn't finish, is that one); none when it's gone.
fn keep_file(a: &Json, kept: &Path) -> Option<String> {
    let from = PathBuf::from(a["path"].as_str()?);
    if !from.is_file() {
        return None;
    }
    std::fs::create_dir_all(kept).ok()?;
    let name = from.file_name()?.to_string_lossy().into_owned();
    let bytes = std::fs::read(&from).ok()?;
    let mut to = kept.join(&name);
    let mut n = 1;
    while to.exists() {
        if std::fs::read(&to).is_ok_and(|there| there == bytes) {
            return Some(to.display().to_string());
        }
        n += 1;
        to = kept.join(format!("{n}-{name}"));
    }
    std::fs::write(&to, &bytes).ok()?;
    Some(to.display().to_string())
}

/// Whether this launch carries the other interface's files in: a released
/// Zeron app does; a debug build only when `ZERON_ZGX_MIGRATE=1` asks (`switch`).
fn asked(identity: crate::identity::Identity, switch: Option<&std::ffi::OsStr>) -> bool {
    identity.migrates || switch.is_some_and(|v| v == "1")
}

/// At the main window's opening: the run, if this build migrates (or a debug
/// build is asked to), said in the log. The drafts to seed: those carried
/// now, or at a later launch those still pending.
pub fn at_launch(app_id: &str) -> (Json, Option<Json>) {
    let Some(app) = zgx_script::app_data_dir(app_id) else { return (Json::Null, None) };
    if asked(crate::identity::CURRENT, crate::dev::switch("ZERON_ZGX_MIGRATE").as_deref())
        && let Some(data) = crate::engine::data_dir()
    {
        let result = migrate(&data, &app);
        if let Some(line) = said(&data, &app, &result) {
            eprintln!("zeron-zgx: migration: {line}");
        }
        if let Ok((outcome, drafts)) = result {
            return (drafts, carried(&outcome));
        }
    }
    (Json::Array(pending(&app)), None)
}

fn carried(outcome: &Outcome) -> Option<Json> {
    let settings = outcome.written.len() + usize::from(outcome.theme);
    (settings + outcome.drafts > 0).then(|| json!({ "settings": settings, "drafts": outcome.drafts }))
}

/// The log's line for a run (none once it has run).
fn said(data: &Path, app: &Path, result: &Result<(Outcome, Json), Refused>) -> Option<String> {
    let file = data.join("ui-settings.json");
    match result {
        Ok((outcome, _)) => Some(format!(
            "carried {} settings{} and {} drafts from {}{}",
            outcome.written.len(),
            if outcome.theme { " and the themes" } else { "" },
            outcome.drafts,
            file.display(),
            match outcome.drafts_left.len() {
                0 => String::new(),
                n => format!(" ({n} drafts left out, named in {})", app.join(MARKER).display()),
            }
        )),
        Err(Refused::Done) => None,
        Err(Refused::Broken(why)) => Some(format!("{} can't be read ({why}): nothing carried, and it's tried again next launch", file.display())),
        Err(Refused::Marker(why)) => Some(format!(
            "carried from {}, but {} couldn't be written ({why}): it runs again next launch, keeping what it carried",
            file.display(),
            app.join(MARKER).display()
        )),
        Err(Refused::Partial(why)) => Some(format!("carried from {}, but {why}: it runs again next launch, keeping what it carried", file.display())),
    }
}

/// Whether a draft's text and cargo are nothing (as screens/chat/shared.luau keeps none for it).
fn nothing(text: Option<&Value>, cargo: Option<&Value>) -> bool {
    let empty = |v: Option<&Value>| match v {
        None | Some(Value::Nil) => true,
        Some(Value::Str(s)) => s.is_empty(),
        Some(Value::List(items)) => items.is_empty(),
        Some(Value::Map(fields)) => fields.is_empty(),
        Some(other) => matches!(other.to_json().as_str(), "" | "[]" | "{}" | "null"),
    };
    empty(text) && empty(cargo)
}

/// Whether the composer holds a draft under `key`: the one shown (`here`'s,
/// as `text` and `cargo`), or one put away in `kept` (screens/chat/shared.luau).
fn held_in(key: &str, here: &Value, text: &Value, cargo: &Value, kept: &Value) -> bool {
    if matches!(here, Value::Str(shown) if &**shown == key) || (key.is_empty() && matches!(here, Value::Nil)) {
        return !nothing(Some(text), Some(cargo));
    }
    kept.field(key).is_some_and(|d| !nothing(d.field("text"), d.field("cargo")))
}

thread_local! {
    /// What keeps [`PENDING`] following the composer: the main window's.
    static FOLLOWING: std::cell::RefCell<Option<zgx::reactive::Effect>> = const { std::cell::RefCell::new(None) };
}

/// The drafts into the composer's own, before its scripts first read them
/// (screens/chat/shared.luau: `chat.composer.drafts` by chat, the new chat's
/// as the composer's own text and cargo), where it holds none for that chat.
/// Then each leaves [`PENDING`] once it's sent or cleared.
pub fn seed_drafts(cx: &mut zgx_desktop::WindowContext, host: &zgx_script::ScriptHost, app_id: &str, drafts: Json) {
    let (Some(app), Some(all)) = (zgx_script::app_data_dir(app_id), drafts.as_array().filter(|a| !a.is_empty())) else { return };
    let here = host.shared("chat.composer.here", "");
    let text = host.shared("chat.composer.text", "");
    let cargo = host.shared("chat.composer.cargo", crate::plugins::json_value(&json!([])));
    let kept = host.shared("chat.composer.drafts", crate::plugins::json_value(&json!({})));
    let holds = |key: &str| held_in(key, &here.get(), &text.get(), &cargo.get(), &kept.get());
    let mut put = crate::plugins::value_json(&kept.get()).as_object().cloned().unwrap_or_default();
    for d in all {
        let key = d["key"].as_str().unwrap_or("");
        if holds(key) {
            continue;
        }
        if key.is_empty() && here.with_untracked(|h| matches!(h, Value::Str(s) if s.is_empty())) {
            text.set(crate::plugins::json_value(&d["text"]));
            cargo.set(crate::plugins::json_value(&d["cargo"]));
        } else {
            put.insert(key.into(), json!({ "text": d["text"], "cargo": d["cargo"], "quotes": {} }));
        }
    }
    kept.set(crate::plugins::json_value(&Json::Object(put)));
    let mut waiting: Vec<Json> = all.clone();
    let effect = cx.ui.runtime.effect(move || {
        if waiting.is_empty() {
            return;
        }
        let (h, t, c, k) = (here.get(), text.get(), cargo.get(), kept.get());
        prune(&app, &mut waiting, |key| held_in(key, &h, &t, &c, &k));
    });
    FOLLOWING.with(|f| *f.borrow_mut() = Some(effect));
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

    /// `UiSettings`'s keys as crates/ui/src/settings.rs declares them (each
    /// field camelCased, or its `rename`), read from the source now.
    fn ui_settings_keys() -> Vec<String> {
        let source = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../crates/ui/src/settings.rs")).unwrap();
        let body = source.split("pub struct UiSettings {").nth(1).expect("the struct").split("\n}").next().unwrap();
        let mut keys = Vec::new();
        let mut renamed = None;
        for line in body.lines().map(str::trim) {
            if line.starts_with("#[") {
                if let Some(at) = line.find("rename = \"") {
                    renamed = line[at + 10..].split('"').next().map(String::from);
                }
                continue;
            }
            let line = line.strip_prefix("pub ").unwrap_or(line);
            let Some((field, rest)) = line.split_once(':') else { continue };
            if rest.starts_with(':') || field.is_empty() || !field.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') {
                continue;
            }
            let camel: String = field
                .split('_')
                .enumerate()
                .map(|(i, part)| if i == 0 { part.to_string() } else { part[..1].to_uppercase() + &part[1..] })
                .collect();
            keys.push(renamed.take().unwrap_or(camel));
        }
        keys
    }

    #[test]
    fn every_key_the_other_interface_writes_has_a_row() {
        let theirs = ui_settings_keys();
        assert!(theirs.len() > 100 && theirs.contains(&"accentColor".to_string()) && theirs.contains(&"pullRequestOpenIn".to_string()), "{theirs:?}");
        let rows: Vec<&str> = TABLE.iter().map(|(k, _)| *k).collect();
        for key in &theirs {
            assert!(rows.contains(&key.as_str()), "{key} has no row");
        }
        // What it writes no more: read in older files (crates/ui settings.rs `load`).
        for key in &rows {
            assert!(theirs.iter().any(|k| k == key) || ["filesEditorFontSize", "pullRequestDestination"].contains(key), "{key} is no key of UiSettings");
        }
        // Today's fixture is such a file.
        for key in keys_of("v0.2.132.json") {
            assert!(rows.contains(&key.as_str()), "{key} has no row");
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
            // (A name, as fit.SIDEBAR_DEFAULT, is no literal to compare.)
            if matches!(theirs.chars().next(), Some('"' | '0'..='9')) || matches!(theirs, "true" | "false") {
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
        assert_eq!(s("ds/motion.luau::smooth_scrolling"), json!(false));
        assert_eq!(s("screens/device/float.luau::device_float_frame"), json!({ "x": 10.0, "y": 10.0, "width": 400.0, "height": 300.0 }));
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
            json!({ "shell.toggle_sidebar": "cmd+shift+b", "shell.toggle_terminal": "cmd+`", "shell.session_9": "ctrl+9", "shell.dictate": "cmd+shift+d" })
        );
        // The other interface's own defaults (the Mac's: a double tap of Option, of both Commands) aren't rebinds.
        assert!(s("screens/shell/keys.luau::shell_keymap").get("shell.quick_capture").is_none());
        // Voice and the device preview (U28's pages).
        assert_eq!(s("screens/shell/keys.luau::shell_keymap")["shell.dictate"], json!("cmd+shift+d"));
        assert_eq!(s("screens/settings/prefs.luau::settings_dictation"), json!(true));
        assert_eq!(s("screens/settings/prefs.luau::settings_dictation_input"), json!("BuiltInMicrophoneDevice"));
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
        // Off the Mac those chords are no default: named, not bound.
        assert_eq!(marker["not_carried"]["keymap.quickCapture"].is_null(), cfg!(target_os = "macos"));
    }

    #[test]
    fn older_files_are_read_as_the_shipping_app_reads_them() {
        let (_dir, data, app) = dirs("v0.2.100.json");
        migrate(&data, &app).unwrap();
        let s = |k: &str| state(&app, k);
        // `accentColor` "violet" was Zeron's; the editor's size became Code's.
        assert_eq!(s("ds/look.luau::accent"), Some(json!("zeron")));
        assert_eq!(s("screens/settings/prefs.luau::settings_code_font_size"), Some(json!(15.0)));
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
    fn the_launch_s_pill_is_told_only_what_was_carried() {
        assert_eq!(carried(&Outcome::default()), None);
        let theme = Outcome { theme: true, ..Outcome::default() };
        assert_eq!(carried(&theme), Some(json!({ "settings": 1, "drafts": 0 })));
        let both = Outcome { written: vec!["a".into(), "b".into()], drafts: 3, ..Outcome::default() };
        assert_eq!(carried(&both), Some(json!({ "settings": 2, "drafts": 3 })));
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
        assert_eq!(said(&data, &app, &Err(Refused::Done)), None, "nothing said once it has run");
    }

    #[test]
    fn a_broken_file_changes_nothing_and_says_so() {
        let (_dir, data, app) = dirs("broken.json");
        let result = migrate(&data, &app);
        assert!(matches!(result, Err(Refused::Broken(_))));
        assert!(!app.exists(), "no setting, no marker: a mended file is carried next launch");
        let line = said(&data, &app, &result).unwrap();
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
        // The older new chat's draft is named where it's left out.
        assert_eq!(outcome.drafts_left.keys().collect::<Vec<_>>(), ["draft-old"]);
        let marker: Json = serde_json::from_slice(&std::fs::read(app.join(MARKER)).unwrap()).unwrap();
        assert!(marker["drafts_left"]["draft-old"].as_str().unwrap().contains("only the newest"));
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
        // Numbered past the picture's mention, not over it.
        assert_eq!(cargo[1]["number"], json!(2));
        // A picture only where the composer draws one: a GIF or HEIC is a file.
        assert!(is_picture("a.PNG") && is_picture("b.webp") && !is_picture("c.gif") && !is_picture("d.heic"));
        assert_eq!(by("side-1").unwrap()["text"], json!("a side chat's words"));
        // The newest chat not yet made is the new chat's; an older one, and an empty one, aren't kept.
        assert_eq!(by("").unwrap()["text"], json!("the newest new chat"));
        assert!(by("draft-old").is_none() && by("chat-empty").is_none());
        // A linked file that's there stays linked, by its own path.
        std::fs::write(data.join("build.log"), "ok").unwrap();
        let (again, _) = super::drafts(&data, &app.join("again")).unwrap();
        let linked = again[0]["cargo"].as_array().unwrap().iter().find(|c| c["linked"] == json!(true)).cloned().unwrap();
        assert_eq!(linked["path"], json!(data.join("build.log").display().to_string()));
    }

    /// A chip with no mention of its own is numbered past the draft's mentions, never onto one.
    #[test]
    fn chips_are_numbered_past_their_mentions() {
        let (_dir, data, app) = dirs("empty.json");
        for name in ["a.png", "b.png"] {
            image::RgbaImage::new(2, 2).save(data.join("composer-drafts").join(name)).unwrap();
        }
        let path = |name: &str| data.join("composer-drafts").join(name).display().to_string();
        let file = json!({ "version": 1, "drafts": { "chat-a": { "prompt": "see [Image 1]", "attachments": [
            { "id": "x", "name": "a.png", "path": path("a.png") },
            { "id": "y", "name": "Image 1.png", "path": path("b.png"), "mention": 1 },
        ], "appshots": [] } } });
        std::fs::write(data.join("composer-drafts.json"), file.to_string()).unwrap();
        let (drafts, _) = super::drafts(&data, &app.join("kept")).unwrap();
        let numbers: Vec<u64> = drafts[0]["cargo"].as_array().unwrap().iter().map(|c| c["number"].as_u64().unwrap()).collect();
        assert_eq!(numbers, [2, 1]);
    }

    /// A draft carried stays carried till it's sent or cleared: kept on disk,
    /// put back at each launch after, never twice.
    #[test]
    fn a_carried_draft_comes_back_each_launch_till_it_s_sent_or_cleared() {
        let (_dir, data, app) = dirs("empty.json");
        let text = std::fs::read_to_string(Path::new(FIXTURES).join("composer-drafts.json")).unwrap();
        std::fs::write(data.join("composer-drafts.json"), text.replace("$DATA", &data.display().to_string())).unwrap();
        image::RgbaImage::new(2, 2).save(data.join("composer-drafts/shot.png")).unwrap();
        let (_, carried) = migrate(&data, &app).unwrap();
        let keys = |all: &[Json]| all.iter().map(|d| d["key"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        assert_eq!(keys(&pending(&app)), keys(carried.as_array().unwrap()));
        // The next launch: not carried again (the marker), but still there to seed.
        assert_eq!(migrate(&data, &app).unwrap_err(), Refused::Done);
        let mut waiting = pending(&app);
        assert_eq!(keys(&waiting), ["chat-a", "", "side-1"]);
        // As the composer holds them: the new chat's shown (sent: emptied), chat-a's put away, side-1's cleared.
        let here = Value::from("");
        let kept = crate::plugins::json_value(&json!({ "chat-a": { "text": "Look at [Image 1] and the log", "cargo": [], "quotes": {} } }));
        let none = crate::plugins::json_value(&json!([]));
        assert!(held_in("chat-a", &here, &Value::from(""), &none, &kept));
        assert!(!held_in("", &here, &Value::from(""), &none, &kept));
        assert!(held_in("", &here, &Value::from("still typing"), &none, &kept));
        assert!(!held_in("side-1", &here, &Value::from(""), &none, &kept));
        assert!(prune(&app, &mut waiting, |key| held_in(key, &here, &Value::from(""), &none, &kept)));
        assert_eq!(keys(&pending(&app)), ["chat-a"]);
        // Nothing more gone: the file isn't written again.
        assert!(!prune(&app, &mut waiting, |_| true));
        // The last one sent: the file goes.
        assert!(prune(&app, &mut waiting, |_| false));
        assert!(!app.join(PENDING).exists() && pending(&app).is_empty());
    }

    /// The marker can't be written: said as that (not as a file that can't be
    /// read), and the run again next launch makes no second copy of a draft's file.
    #[test]
    fn a_marker_that_can_t_be_written_says_so_and_copies_nothing_twice() {
        let (dir, data, app) = dirs("v0.2.132.json");
        let text = std::fs::read_to_string(Path::new(FIXTURES).join("composer-drafts.json")).unwrap();
        std::fs::write(data.join("composer-drafts.json"), text.replace("$DATA", &data.display().to_string())).unwrap();
        image::RgbaImage::new(2, 2).save(data.join("composer-drafts/shot.png")).unwrap();
        std::fs::create_dir_all(&app).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.path().join("nowhere/marker.json"), app.join(MARKER)).unwrap();
        let result = migrate(&data, &app);
        assert!(matches!(result, Err(Refused::Marker(_))), "{result:?}");
        let line = said(&data, &app, &result).unwrap();
        assert!(line.contains("couldn't be written") && !line.contains("can't be read"), "{line}");
        let copies = || std::fs::read_dir(app.join("migrated-drafts")).unwrap().count();
        let first = copies();
        assert!(first > 0);
        std::fs::remove_file(app.join(MARKER)).unwrap();
        migrate(&data, &app).unwrap();
        assert_eq!(copies(), first, "the same copies, none made again");
        assert!(app.join(MARKER).is_file());
    }

    #[cfg(unix)]
    #[test]
    fn a_setting_or_drafts_file_that_fails_leaves_no_marker_and_runs_again() {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = |path: &Path, mode: u32| std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
        let (_dir, data, app) = dirs("v0.2.132.json");
        let state_dir = app.join("script-state/app.luau");
        std::fs::create_dir_all(&state_dir).unwrap();
        mode(&state_dir, 0o555);
        let result = migrate(&data, &app);
        mode(&state_dir, 0o755);
        assert!(matches!(&result, Err(Refused::Partial(why)) if why.contains("settings couldn't be read or written")), "{result:?}");
        assert!(!app.join(MARKER).exists());
        assert!(said(&data, &app, &result).unwrap().contains("runs again next launch"));
        migrate(&data, &app).unwrap();
        assert_eq!(state(&app, "ds/look.luau::accent"), Some(json!("pink")));

        // A setting of yours that can't be read is neither overwritten nor taken as done.
        let (_dir, data, app) = dirs("v0.2.132.json");
        std::fs::create_dir_all(app.join("script-state/app.luau")).unwrap();
        let accent = app.join("script-state/app.luau").join(file_of("ds/look.luau::accent"));
        std::fs::write(&accent, "\"red\"").unwrap();
        mode(&accent, 0o000);
        let result = migrate(&data, &app);
        mode(&accent, 0o644);
        assert!(matches!(&result, Err(Refused::Partial(_))), "{result:?}");
        assert_eq!(state(&app, "ds/look.luau::accent"), Some(json!("red")));
        assert!(!app.join(MARKER).exists());

        // The drafts file there but unreadable: not taken as none.
        let (_dir, data, app) = dirs("empty.json");
        let text = std::fs::read_to_string(Path::new(FIXTURES).join("composer-drafts.json")).unwrap();
        let drafts_file = data.join("composer-drafts.json");
        std::fs::write(&drafts_file, text.replace("$DATA", &data.display().to_string())).unwrap();
        mode(&drafts_file, 0o000);
        let result = migrate(&data, &app);
        mode(&drafts_file, 0o644);
        assert!(matches!(&result, Err(Refused::Partial(why)) if why.contains("composer-drafts.json")), "{result:?}");
        assert!(!app.join(MARKER).exists());
        assert_eq!(migrate(&data, &app).unwrap().0.drafts, 3);
    }

    /// A debug build of the Zeron app carries nothing unless asked
    /// (`ZERON_ZGX_MIGRATE=1`); a released one does by itself.
    #[test]
    fn a_debug_zeron_app_migrates_only_when_asked() {
        use crate::identity::{Build, of};
        let one = std::ffi::OsStr::new("1");
        assert!(!asked(of(Build::Cutover, false), None));
        assert!(asked(of(Build::Cutover, false), Some(one)));
        assert!(!asked(of(Build::Cutover, false), Some(std::ffi::OsStr::new("0"))));
        assert!(asked(of(Build::Cutover, true), None));
        assert!(!asked(of(Build::Development, false), None));
        assert!(!asked(of(Build::Beta, true), None));
    }

    /// gpui's default Code size (12.5) is this app's 13, not 12: never smaller than it was.
    #[test]
    fn a_code_size_comes_out_no_smaller() {
        let size = |px: f64| more(&json!({ "codeFontSize": px })).into_iter().find(|(f, _)| f.contains("settings_code_font_size")).map(|(_, v)| v);
        assert_eq!(size(12.5), Some(json!(13.0)));
        assert_eq!(size(12.0), Some(json!(12.0)));
        assert_eq!(size(14.0), Some(json!(14.0)));
        assert_eq!(size(8.0), Some(json!(11.0)));
        assert_eq!(size(20.0), Some(json!(16.0)));
    }

    /// The other interface's keymap as each platform writes it: its defaults
    /// stay defaults, a chord this app can't bind is named, never bound.
    #[test]
    fn the_other_interface_s_defaults_on_each_platform_stay_defaults() {
        let keys = |map: Json, mac: bool| keymap(&map, mac);
        // The Mac's defaults (crates/ui `default_quick_capture`, `default_combo_on(true)`).
        assert_eq!(keys(json!({ "quickCapture": "option-option", "captureAppshot": "command-command", "saveFile": "mod-s" }), true), Keys::default());
        // Elsewhere's.
        assert_eq!(keys(json!({ "quickCapture": "none", "captureAppshot": "mod-alt-space", "saveFile": "mod-s" }), false), Keys::default());
        // The Mac's other chords: named, not bound.
        let mac = keys(json!({ "quickCapture": "shift-shift", "captureAppshot": "option-option", "saveFile": "mod-s" }), true);
        // (Settings → Appshots reads both chords in place.)
        assert!(mac.bound.is_empty() && mac.left.is_empty(), "{mac:?}");
        // Quick capture cleared on the Mac: cleared here ("" is none); Appshots can't be: named.
        let cleared = keys(json!({ "quickCapture": "none", "captureAppshot": "none", "saveFile": "mod-s" }), true);
        assert_eq!(cleared.bound, json!({ "shell.quick_capture": "" }).as_object().cloned().unwrap());
        // A rebind of a command this app hasn't: named.
        let other = keys(json!({ "saveFile": "mod-shift-s", "toggleTray": "mod-shift-i" }), true);
        assert!(other.left.contains_key("keymap.saveFile") && other.left.contains_key("keymap.toggleTray"));
        // A font, a math face and an app to open HTML with that this app hasn't: named.
        let left = left_out(&json!({ "uiFontFamily": "system", "mathFont": "garamond", "htmlOpenApp": { "kind": "app", "path": "/x" } }), true);
        assert!(left.contains_key("uiFontFamily") && left.contains_key("mathFont") && left.contains_key("htmlOpenApp"), "{left:?}");
    }

    #[test]
    fn combinations_read_as_this_app_writes_them() {
        assert_eq!(combo("mod-shift-b").as_deref(), Some("cmd+shift+b"));
        assert_eq!(combo("ctrl-alt-space").as_deref(), Some("ctrl+alt+space"));
        assert_eq!(combo("mod--").as_deref(), Some("cmd+-"));
        assert_eq!(combo("").as_deref(), Some(""));
        assert_eq!(combo("hyper-k"), None);
        // Modifiers alone (a double-tap chord) or "none": nothing a key can press.
        assert_eq!(combo("option-option"), None);
        assert_eq!(combo("shift-shift"), None);
        assert_eq!(combo("command-command"), None);
        assert_eq!(combo("none"), None);
    }
}
