//! The sidebar as you had it in the app this one replaces: the first time a
//! beta opens, what `ui-settings.json` says of the sidebar becomes its own
//! `ui.persist` values. Only what it has no value for yet, so what you've
//! changed here stays, and a value once carried is its own from then on.
//!
//! The look comes the same way: the card, frame, chat art and its picture, and
//! the Live state column's fold and the model picker's layout, each of the frame and the art as a set: one you've touched here isn't
//! paired with the other app's rest. New project's pinned folders come with it. Pinned chats and sections
//! aren't here: the engine keeps them, for either app.
//! Computer use's click sounds and its guide's Got it come too, and so do
//! Chat's, the session switcher's, Files', the chimes', Appshots', the motion and the
//! conversation's width.

use std::path::Path;

use serde_json::{Value as Json, json};

const SIDEBAR: &str = "screens%2Fshell%2Fsidebar.luau%3A%3Ashell_sidebar_";
const VIEW: &str = "screens%2Fshell%2Fview.luau%3A%3Ashell_";
const PICKER: &str = "screens%2Fshell%2Fadd_project.luau%3A%3Ashell_add_project_";
const LOOK: &str = "screens%2Fshell%2Fappearance.luau%3A%3Ashell_";
const SIDECAR_COLLAPSED: &str = "screens%2Fsidecar.luau%3A%3Asidecar_collapsed.json";
const INSET: &str = "ds%2Flook.luau%3A%3Ainset_card.json";
const THEME_COLORS: &str = "ds%2Flook.luau%3A%3Awallpaper_theme_colors.json";
const MODEL_PICKER_COMPACT: &str = "ds%2Fmodel_picker.luau%3A%3Amodel_picker_compact.json";
const COMPUTER: &str = "screens%2Fsettings%2Fcomputer.luau%3A%3A";
const PREFS: &str = "screens%2Fsettings%2Fprefs.luau%3A%3A";
const SWITCHER: &str = "screens%2Fshell%2Fswitcher.luau%3A%3A";
const APPSHOTS: &str = "screens%2Fchat%2Fappshots.luau%3A%3A";
const MOTION: &str = "ds%2Fmotion.luau%3A%3A";
const SKILLS_OFF: &str = "screens%2Fchat%2Finvocations.luau%3A%3Askills_disabled.json";

fn look(name: &str) -> String {
    format!("{LOOK}{name}.json")
}

fn frame_set() -> Vec<String> {
    let mut set = vec![INSET.to_string()];
    set.extend(["frame_style", "frame_shade", "frame_blur", "frame_sidebar", "frame_blend"].map(look));
    set
}

fn art_set() -> Vec<String> {
    let mut set = vec![THEME_COLORS.to_string()];
    set.extend(
        [
            "chat_background",
            "chat_background_placement",
            "chat_background_opacity",
            "chat_background_blur",
            "chat_background_scrim",
            "chat_background_tab_shade",
            "wallpaper",
            "wallpaper_image",
            "wallpaper_effect",
            "wallpaper_color",
        ]
        .map(look),
    );
    set
}

/// `settings` (`ui-settings.json`) as files of `ui.persist`, each a name and
/// its value; only what it says and this app has.
pub(crate) fn carried(settings: &Json) -> Vec<(String, Json)> {
    let mut out = Vec::new();
    let sidebar = |name: &str| format!("{SIDEBAR}{name}.json");
    for (from, to) in [
        ("sidebarCompact", "compact"),
        ("sidebarShowProjectIcon", "show_project_icon"),
        ("sidebarShowHarness", "show_harness"),
        ("sidebarShowBranch", "show_branch"),
        ("sidebarShowPullRequest", "show_pr"),
        ("sidebarShowProjectFilter", "show_project_filter"),
        ("sidebarShowArchived", "show_archived"),
        ("sidebarShowProjectLabel", "show_location"),
        ("sidebarWorkingShelf", "show_working"),
        ("sidebarActiveFirst", "active_first"),
    ] {
        if let Some(on) = settings[from].as_bool() {
            out.push((sidebar(to), json!(on)));
        }
    }
    if let Some(width) = settings["sidebarWidth"].as_f64() {
        out.push((sidebar("width"), json!(width)));
    }
    match settings["sidebarSort"].as_str() {
        Some("lastUpdated") => out.push((sidebar("sort"), json!("updated"))),
        Some("created") => out.push((sidebar("sort"), json!("created"))),
        _ => {}
    }
    match settings["sidebarOrganization"].as_str() {
        Some("byProject") => out.push((sidebar("organize"), json!(["project"]))),
        Some("byDevice") => out.push((sidebar("organize"), json!(["device"]))),
        Some("inOneList") => out.push((sidebar("organize"), json!([]))),
        _ => {}
    }
    if let Some(organization) = settings["sidebarOrganization"].as_str() {
        out.push((sidebar("inbox"), json!(organization == "inbox")));
    }
    if let Some(space) = settings["spaceFilter"].as_str() {
        out.push((sidebar("project"), json!(space)));
    }
    if let Some(on) = settings["sidebarHoverEnabled"].as_bool() {
        out.push((format!("{VIEW}sidebar_hover.json"), json!(on)));
    }
    if let Some(shut) = settings["sidebarCollapsed"].as_bool() {
        out.push((format!("{VIEW}collapsed.json"), json!(shut)));
    }
    if let Some(shut) = settings["sidecarCollapsed"].as_bool() {
        out.push((SIDECAR_COLLAPSED.to_string(), json!(shut)));
    }
    if let Some(on) = settings["insetShell"].as_bool() {
        out.push((INSET.to_string(), json!(on)));
    }
    if let Some(on) = settings["wallpaperThemeColors"].as_bool() {
        out.push((THEME_COLORS.to_string(), json!(on)));
    }
    if let Some(on) = settings["compactModelPicker"].as_bool() {
        out.push((MODEL_PICKER_COMPACT.to_string(), json!(on)));
    }
    for (group, from, to) in [
        ("shellFrame", "style", "frame_style"),
        ("shellFrame", "shade", "frame_shade"),
        ("shellFrame", "blur", "frame_blur"),
        ("shellFrame", "sidebar", "frame_sidebar"),
        ("shellFrame", "blend", "frame_blend"),
        ("chatBackground", "enabled", "chat_background"),
        ("chatBackground", "placement", "chat_background_placement"),
        ("chatBackground", "opacity", "chat_background_opacity"),
        ("chatBackground", "blur", "chat_background_blur"),
        ("chatBackground", "scrim", "chat_background_scrim"),
        ("chatBackground", "tabShade", "chat_background_tab_shade"),
    ] {
        if !settings[group][from].is_null() {
            out.push((look(to), settings[group][from].clone()));
        }
    }
    if let Some(effect) = settings["newThreadBackgroundEffect"].as_str() {
        out.push((look("wallpaper_effect"), json!(effect)));
    }
    if let Some(color) = settings["wallpaperColor"].as_str() {
        out.push((look("wallpaper_color"), json!(color)));
    }
    if let Some(sounds) = settings["computerClickSounds"].as_str().filter(|s| matches!(*s, "foreground" | "foreground-and-background" | "off")) {
        out.push((format!("{COMPUTER}click_sounds.json"), json!(sounds)));
    }
    if let Some(seen) = settings["computerGettingStartedAcknowledged"].as_bool() {
        out.push((format!("{COMPUTER}guide_acknowledged.json"), json!(seen)));
    }
    if let Some(pins) = settings["projectFolderPins"].as_object().filter(|pins| !pins.is_empty()) {
        out.push((format!("{PICKER}pins.json"), Json::Object(pins.clone())));
    }
    out.extend(carried_prefs(settings));
    out
}

/// Settings' own pages (Chat, General's switcher, Files, Notifications'
/// chimes, Appshots, Appearance's motion and the conversation's width): each only as a value this
/// app takes.
fn carried_prefs(settings: &Json) -> Vec<(String, Json)> {
    let mut out = Vec::new();
    let pref = |name: &str| format!("{PREFS}{name}.json");
    for (from, to) in [
        ("escapeStopsActiveAgent", "settings_escape_stops"),
        ("transcriptCompactMode", "settings_compact_transcript"),
        ("filesAutosaveEnabled", "settings_files_autosave"),
        ("filesWordWrap", "settings_files_word_wrap"),
        ("filesShowAll", "settings_files_show_all"),
        ("soundEnabled", "settings_sounds"),
        ("soundCompletionEnabled", "settings_sound_done"),
        ("soundInputEnabled", "settings_sound_request"),
        ("soundAttentionEnabled", "settings_sound_attention"),
    ] {
        if let Some(on) = settings[from].as_bool() {
            out.push((pref(to), json!(on)));
        }
    }
    match settings["composerSendBehavior"].as_str() {
        Some("enter") => out.push((pref("settings_send_with"), json!("enter"))),
        Some("mod-enter") => out.push((pref("settings_send_with"), json!("mod_enter"))),
        _ => {}
    }
    if let Some(follow) = settings["followUpBehavior"].as_str().filter(|f| matches!(*f, "queue" | "steer")) {
        out.push((pref("settings_follow_up"), json!(follow)));
    }
    if let Some(zeron) = settings["openWebLinksInZeron"].as_bool() {
        out.push((pref("settings_open_links"), json!(if zeron { "zeron" } else { "external" })));
    }
    if let Some(ms) = settings["filesAutosaveDelayMs"].as_u64().filter(|ms| (100..=60_000).contains(ms)) {
        out.push((pref("settings_files_autosave_delay"), json!(ms)));
    }
    if let Some(width) = settings["transcriptWidth"].as_f64().filter(|w| w.is_finite()) {
        out.push((pref("settings_conversation_width"), json!(width.clamp(560.0, 1200.0))));
    }
    let window = match settings["switcherWindow"].as_str() {
        Some("all") => Some("all"),
        Some("hours1") => Some("1h"),
        Some("hours2") => Some("2h"),
        Some("hours4") => Some("4h"),
        Some("hours8") => Some("8h"),
        Some("hours12") => Some("12h"),
        Some("hours24") => Some("24h"),
        Some("days3") => Some("3d"),
        Some("days7") => Some("7d"),
        _ => None,
    };
    if let Some(window) = window {
        out.push((format!("{SWITCHER}shell_switcher_window.json"), json!(window)));
    }
    if let Some(limit) = settings["switcherLimit"].as_u64() {
        out.push((format!("{SWITCHER}shell_switcher_limit.json"), json!(limit)));
    }
    if let Some(on) = settings["switcherCyclesTabs"].as_bool() {
        out.push((format!("{SWITCHER}shell_switcher_cycles_tabs.json"), json!(on)));
    }
    for (from, to) in [("appshotsEnabled", "appshots_enabled"), ("appshotSoundEnabled", "appshots_sound")] {
        if let Some(on) = settings[from].as_bool() {
            out.push((format!("{APPSHOTS}{to}.json"), json!(on)));
        }
    }
    let destination = match settings["appshotDestination"].as_str() {
        Some("automatic") => Some("automatic"),
        Some("last-session") => Some("last"),
        Some("new-session") => Some("new"),
        _ => None,
    };
    if let Some(destination) = destination {
        out.push((format!("{APPSHOTS}appshots_destination.json"), json!(destination)));
    }
    let (reduce, follow) = match settings["reduceMotion"].as_str() {
        Some("system") => (Some(false), Some(true)),
        Some("on") => (Some(true), Some(false)),
        Some("off") => (Some(false), Some(false)),
        _ => (None, None),
    };
    if let (Some(reduce), Some(follow)) = (reduce, follow) {
        out.push((format!("{MOTION}motion_reduce.json"), json!(reduce)));
        out.push((format!("{MOTION}motion_follow_system.json"), json!(follow)));
    }
    if let Some(on) = settings["pauseAnimationsInBackground"].as_bool() {
        out.push((format!("{MOTION}motion_pause_background.json"), json!(on)));
    }
    if let Some(paths) = settings["disabledSkills"].as_array() {
        let off: serde_json::Map<String, Json> = paths.iter().filter_map(|p| p.as_str()).map(|p| (p.to_string(), json!(true))).collect();
        if !off.is_empty() {
            out.push((SKILLS_OFF.to_string(), Json::Object(off)));
        }
    }
    out
}

/// A copy in this app's own folder: the other app removes its file when it replaces it.
pub(crate) fn carried_picture(settings: &Json, pictures: &Path) -> Vec<(String, Json)> {
    let background = &settings["newThreadComposerBackground"];
    let Some((path, w, h)) = background["path"].as_str().and_then(|picture| crate::settings::keep_wallpaper_at_its_size(picture, pictures)) else {
        return Vec::new();
    };
    let place = &background["adjustment"];
    vec![
        (
            look("wallpaper_image"),
            json!({
                "path": path,
                "name": background["name"],
                "w": w,
                "h": h,
                "x": place["focalX"].as_f64().unwrap_or(0.5),
                "y": place["focalY"].as_f64().unwrap_or(0.5),
                "zoom": place["zoom"].as_f64().unwrap_or(1.0),
            }),
        ),
        (look("wallpaper"), json!("image")),
    ]
}

/// Writes into `state` (the app window's `ui.persist` folder) what `settings`
/// says and `state` has no value for. A setting that can't be written is left
/// at its default.
fn carry_into(state: &Path, pictures: &Path, settings: &Json) {
    carry_with(state, pictures, settings, Vec::new(), &|name| state.join(name).exists(), &|_, _| true);
}

/// `carry_into`, with `more` beside what `carried` says, `held` saying which
/// of `state`'s files are a value of this app's own, and `worth` which values
/// are worth writing: the names written.
pub(crate) fn carry_with(
    state: &Path,
    pictures: &Path,
    settings: &Json,
    more: Vec<(String, Json)>,
    held: &dyn Fn(&str) -> bool,
    worth: &dyn Fn(&str, &Json) -> bool,
) -> Vec<String> {
    let touched = |set: &[String]| set.iter().any(|name| held(name));
    let (frame, art) = (frame_set(), art_set());
    let (frame_touched, art_touched) = (touched(&frame), touched(&art));
    let mut wanted = carried(settings);
    wanted.extend(more);
    if !art_touched {
        wanted.extend(carried_picture(settings, pictures));
    }
    wanted.retain(|(name, _)| !(frame_touched && frame.contains(name)) && !(art_touched && art.contains(name)));
    let mut written = Vec::new();
    if wanted.is_empty() || std::fs::create_dir_all(state).is_err() {
        return written;
    }
    for (name, value) in wanted {
        if !held(&name) && worth(&name, &value) && !written.contains(&name) && std::fs::write(state.join(&name), value.to_string()).is_ok() {
            written.push(name);
        }
    }
    written
}

/// Before the main window's scripts first read them.
pub fn carry(ui_settings: Option<&Path>, state: Option<&Path>) {
    let (Some(from), Some(state)) = (ui_settings, state) else { return };
    let Some(settings) = std::fs::read(from).ok().and_then(|bytes| serde_json::from_slice::<Json>(&bytes).ok()) else {
        return;
    };
    let pictures = from.parent().unwrap_or(Path::new("")).join(crate::settings::WALLPAPER_DIR);
    carry_into(state, &pictures, &settings);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(state: &Path, name: &str) -> Json {
        serde_json::from_slice(&std::fs::read(state.join(format!("{SIDEBAR}{name}.json"))).unwrap()).unwrap()
    }

    #[test]
    fn the_sidebar_comes_across_as_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let settings = json!({
            "sidebarOrganization": "byDevice", "sidebarSort": "created", "sidebarCompact": false,
            "sidebarShowHarness": false, "sidebarShowPullRequest": false, "sidebarWidth": 293.5,
            "sidebarCollapsed": true,
            "spaceFilter": "space-1",
            "sidebarHoverEnabled": false, "sidebarShowProjectLabel": false, "sidebarWorkingShelf": true, "sidebarActiveFirst": true,
            "compactModelPicker": false,
        });
        carry_into(dir.path(), &dir.path().join("kept"), &settings);
        assert_eq!(read(dir.path(), "organize"), json!(["device"]));
        assert_eq!(read(dir.path(), "sort"), json!("created"));
        assert_eq!(read(dir.path(), "compact"), json!(false));
        assert_eq!(read(dir.path(), "show_harness"), json!(false));
        assert_eq!(read(dir.path(), "show_pr"), json!(false));
        assert_eq!(read(dir.path(), "width"), json!(293.5));
        assert!(dir.path().join(format!("{VIEW}collapsed.json")).exists());
        assert_eq!(read(dir.path(), "inbox"), json!(false));
        assert_eq!(read(dir.path(), "show_location"), json!(false));
        assert_eq!(read(dir.path(), "show_working"), json!(true));
        assert_eq!(read(dir.path(), "active_first"), json!(true));
        assert_eq!(read(dir.path(), "project"), json!("space-1"));
        assert!(dir.path().join(format!("{VIEW}sidebar_hover.json")).exists());
        assert_eq!(std::fs::read_to_string(dir.path().join(MODEL_PICKER_COMPACT)).unwrap(), "false");
        // What it doesn't say stays at this app's default.
        assert!(!dir.path().join(format!("{SIDEBAR}show_branch.json")).exists());
    }

    #[test]
    fn computer_use_s_click_sounds_and_guide_come_across() {
        let dir = tempfile::tempdir().unwrap();
        carry_into(dir.path(), &dir.path().join("kept"), &json!({ "computerClickSounds": "foreground-and-background", "computerGettingStartedAcknowledged": true }));
        let read = |name: &str| std::fs::read_to_string(dir.path().join(format!("{COMPUTER}{name}.json"))).unwrap();
        assert_eq!((read("click_sounds").as_str(), read("guide_acknowledged").as_str()), ("\"foreground-and-background\"", "true"));
        let odd = tempfile::tempdir().unwrap();
        carry_into(odd.path(), &odd.path().join("kept"), &json!({ "computerClickSounds": "loud" }));
        assert!(!odd.path().join(format!("{COMPUTER}click_sounds.json")).exists(), "a word this app doesn't know stays its default");
    }

    #[test]
    fn settings_pages_come_across_as_this_app_takes_them() {
        let dir = tempfile::tempdir().unwrap();
        carry_into(
            dir.path(),
            &dir.path().join("kept"),
            &json!({
                "composerSendBehavior": "mod-enter", "followUpBehavior": "steer", "escapeStopsActiveAgent": true,
                "transcriptCompactMode": true, "openWebLinksInZeron": false,
                "switcherWindow": "hours12", "switcherLimit": 8, "switcherCyclesTabs": true,
                "filesAutosaveEnabled": true, "filesAutosaveDelayMs": 1500, "filesWordWrap": true, "filesShowAll": false,
                "soundEnabled": true, "soundCompletionEnabled": false, "soundInputEnabled": true, "soundAttentionEnabled": false,
                "appshotsEnabled": true, "appshotSoundEnabled": false, "appshotDestination": "last-session",
                "transcriptWidth": 736.0, "reduceMotion": "on", "pauseAnimationsInBackground": true,
                "disabledSkills": ["/u/.claude/skills/pdf/SKILL.md"],
            }),
        );
        let file = |name: String| serde_json::from_slice::<Json>(&std::fs::read(dir.path().join(name)).unwrap()).unwrap();
        let pref = |name: &str| file(format!("{PREFS}{name}.json"));
        assert_eq!(pref("settings_send_with"), json!("mod_enter"));
        assert_eq!(pref("settings_follow_up"), json!("steer"));
        assert_eq!(pref("settings_escape_stops"), json!(true));
        assert_eq!(pref("settings_compact_transcript"), json!(true));
        assert_eq!(pref("settings_open_links"), json!("external"));
        assert_eq!(pref("settings_files_autosave"), json!(true));
        assert_eq!(pref("settings_files_autosave_delay"), json!(1500));
        assert_eq!(pref("settings_files_word_wrap"), json!(true));
        assert_eq!(pref("settings_files_show_all"), json!(false));
        assert_eq!((pref("settings_sounds"), pref("settings_sound_done"), pref("settings_sound_request"), pref("settings_sound_attention")), (json!(true), json!(false), json!(true), json!(false)));
        assert_eq!(pref("settings_conversation_width"), json!(736.0));
        assert_eq!(file(format!("{SWITCHER}shell_switcher_window.json")), json!("12h"));
        assert_eq!(file(format!("{SWITCHER}shell_switcher_limit.json")), json!(8));
        assert_eq!(file(format!("{SWITCHER}shell_switcher_cycles_tabs.json")), json!(true));
        assert_eq!(file(format!("{APPSHOTS}appshots_enabled.json")), json!(true));
        assert_eq!(file(format!("{APPSHOTS}appshots_sound.json")), json!(false));
        assert_eq!(file(format!("{APPSHOTS}appshots_destination.json")), json!("last"));
        assert_eq!((file(format!("{MOTION}motion_reduce.json")), file(format!("{MOTION}motion_follow_system.json"))), (json!(true), json!(false)));
        assert_eq!(file(format!("{MOTION}motion_pause_background.json")), json!(true));
        assert_eq!(file(SKILLS_OFF.to_string()), json!({ "/u/.claude/skills/pdf/SKILL.md": true }));
        // A word or a number this app doesn't take stays its default.
        let odd = tempfile::tempdir().unwrap();
        carry_into(odd.path(), &odd.path().join("kept"), &json!({ "composerSendBehavior": "shift-enter", "switcherWindow": "weeks2", "appshotDestination": "elsewhere", "filesAutosaveDelayMs": 0, "transcriptWidth": 99999.0 }));
        for name in [format!("{PREFS}settings_send_with.json"), format!("{SWITCHER}shell_switcher_window.json"), format!("{APPSHOTS}appshots_destination.json"), format!("{PREFS}settings_files_autosave_delay.json")] {
            assert!(!odd.path().join(&name).exists(), "{name}");
        }
        let wide = serde_json::from_slice::<Json>(&std::fs::read(odd.path().join(format!("{PREFS}settings_conversation_width.json"))).unwrap()).unwrap();
        assert_eq!(wide, json!(1200.0));
    }

    #[test]
    fn new_projects_pinned_folders_come_across() {
        let dir = tempfile::tempdir().unwrap();
        let pins = json!({ "device-1": ["/code", "/work/zephyr"] });
        carry_into(dir.path(), &dir.path().join("kept"), &json!({ "projectFolderPins": pins }));
        let carried = dir.path().join(format!("{PICKER}pins.json"));
        assert_eq!(serde_json::from_slice::<Json>(&std::fs::read(carried).unwrap()).unwrap(), pins);
        let none = tempfile::tempdir().unwrap();
        carry_into(none.path(), &none.path().join("kept"), &json!({ "projectFolderPins": {} }));
        assert!(!none.path().join(format!("{PICKER}pins.json")).exists());
    }

    #[test]
    fn the_frame_and_the_art_come_across() {
        let dir = tempfile::tempdir().unwrap();
        let picture = dir.path().join("art.png");
        image::RgbaImage::new(4, 3).save(&picture).unwrap();
        let state = dir.path().join("state");
        let kept = dir.path().join("kept");
        carry_into(
            &state,
            &kept,
            &json!({
                "insetShell": false, "sidecarCollapsed": true, "wallpaperThemeColors": false, "wallpaperColor": "#5948a8",
                "shellFrame": { "style": "glass", "shade": 0.38 },
                "chatBackground": { "enabled": true, "tabShade": 0.55 },
                "newThreadBackgroundEffect": "none",
                "newThreadComposerBackground": { "path": picture, "name": "art.png", "adjustment": { "focalX": 0.25, "focalY": 0.5, "zoom": 2.0 } },
            }),
        );
        let file = |name: &str| serde_json::from_slice::<Json>(&std::fs::read(state.join(name)).unwrap()).unwrap();
        assert_eq!(file("ds%2Flook.luau%3A%3Ainset_card.json"), json!(false));
        assert_eq!(file(SIDECAR_COLLAPSED), json!(true));
        let look = |name: &str| file(&format!("{LOOK}{name}.json"));
        assert_eq!(look("frame_style"), json!("glass"));
        assert_eq!(look("frame_shade"), json!(0.38));
        assert_eq!(look("chat_background"), json!(true));
        assert_eq!(look("chat_background_tab_shade"), json!(0.55));
        assert_eq!(look("wallpaper_effect"), json!("none"));
        assert_eq!(look("wallpaper_color"), json!("#5948a8"));
        assert_eq!(look("wallpaper"), json!("image"));
        let image = look("wallpaper_image");
        assert_eq!((image["w"].as_u64(), image["h"].as_u64(), image["x"].as_f64(), image["zoom"].as_f64()), (Some(4), Some(3), Some(0.25), Some(2.0)));
        std::fs::remove_file(&picture).unwrap();
        let copy = Path::new(image["path"].as_str().unwrap());
        assert_eq!(copy.parent(), Some(kept.as_path()));
        assert!(copy.is_file(), "the other app removing its file leaves this one's");
        // A picture that has gone leaves the wallpaper as it was.
        let gone = dir.path().join("gone");
        carry_into(&gone, &kept, &json!({ "newThreadComposerBackground": { "path": "/nowhere.png", "name": "x" } }));
        assert!(!gone.join(format!("{LOOK}wallpaper.json")).exists());
    }

    #[test]
    fn what_you_changed_here_stays() {
        let dir = tempfile::tempdir().unwrap();
        let mine = dir.path().join(format!("{SIDEBAR}sort.json"));
        std::fs::write(&mine, "\"updated\"").unwrap();
        carry_into(dir.path(), &dir.path().join("kept"), &json!({ "sidebarSort": "created", "sidebarCompact": false }));
        assert_eq!(read(dir.path(), "sort"), json!("updated"));
        assert_eq!(read(dir.path(), "compact"), json!(false));
    }

    #[test]
    fn a_set_with_anything_of_yours_is_left_whole() {
        let dir = tempfile::tempdir().unwrap();
        let picture = dir.path().join("art.png");
        image::RgbaImage::new(4, 3).save(&picture).unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(state.join(look("wallpaper_image")), "{}").unwrap();
        std::fs::write(state.join(look("frame_style")), "\"glass\"").unwrap();
        let kept = dir.path().join("kept");
        carry_into(
            &state,
            &kept,
            &json!({
                "insetShell": false, "sidebarCompact": false, "newThreadBackgroundEffect": "none", "wallpaperColor": "#5948a8",
                "newThreadComposerBackground": { "path": picture, "name": "art.png" },
            }),
        );
        for name in [look("wallpaper_effect"), look("wallpaper_color"), look("wallpaper"), INSET.to_string()] {
            assert!(!state.join(&name).exists(), "{name}");
        }
        assert!(!kept.exists(), "no copy for a set left alone");
        assert_eq!(read(&state, "compact"), json!(false));
    }

    #[test]
    fn no_file_or_a_broken_one_carries_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        carry(Some(&dir.path().join("missing.json")), Some(&state));
        assert!(!state.exists());
        let broken = dir.path().join("broken.json");
        std::fs::write(&broken, "{").unwrap();
        carry(Some(&broken), Some(&state));
        assert!(!state.exists());
    }
}
