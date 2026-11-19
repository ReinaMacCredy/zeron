#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UiSettings {
    pub dictation_enabled: bool,
    /// Dictation microphone as a `zeron_voice::InputDevice` id; `None`
    /// follows the system default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dictation_input: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_geometry: Option<WindowGeometry>,
    /// Submit using Enter or the platform modifier plus Enter.
    pub composer_send_behavior: ComposerSendBehavior,
    /// Queue or steer a message submitted while the agent is busy.
    pub follow_up_behavior: FollowUpBehavior,
    /// Modifier for the thinking-level step shortcuts (modifier + `-`/`+`).
    pub reasoning_shortcut_modifier: ReasoningShortcutModifier,
    /// Recency limit on the sessions the Ctrl+Tab switcher offers.
    pub switcher_window: SwitcherWindow,
    /// Most cards the Ctrl+Tab switcher shows; 0 for no cap.
    pub switcher_limit: u32,
    /// Ctrl+Tab steps through the tab strip in place of the switcher.
    pub switcher_cycles_tabs: bool,
    /// Legacy global opt-in; per-harness preferences take precedence.
    pub skills_in_slash_menu: bool,
    pub skill_completion_by_harness:
        std::collections::HashMap<zeron_proto::HarnessId, SkillCompletionSettings>,
    /// UI-side only: the harnesses' own skill configs are untouched.
    #[serde(default, skip_serializing_if = "std::collections::BTreeSet::is_empty")]
    pub disabled_skills: std::collections::BTreeSet<String>,
    /// Open model selection with an effort slider and a separate model list.
    pub compact_model_picker: bool,
    pub compact_effort_slider: EffortSliderStyle,
    pub computer_click_sounds: ComputerClickSounds,
    pub sidebar_width: f32,
    pub sidebar_collapsed: bool,
    /// Reveal the collapsed sidebar when hovering over the left window edge.
    pub sidebar_hover_enabled: bool,
    pub inset_shell: bool,
    /// How active sessions are partitioned in the sidebar.
    pub sidebar_organization: SidebarOrganization,
    /// Timestamp used to order active sessions (newest first).
    pub sidebar_sort: SidebarSort,
    pub sidebar_active_first: bool,
    pub sidebar_working_shelf: bool,
    /// Optional harness branding and repository metadata shown below each
    /// session title.
    pub sidebar_show_project_label: bool,
    pub sidebar_compact: bool,
    pub sidebar_show_project_icon: bool,
    pub sidebar_show_harness: bool,
    pub sidebar_show_branch: bool,
    pub sidebar_show_pull_request: bool,
    /// The project filter row at the top of the sidebar.
    pub sidebar_show_project_filter: bool,
    /// The Archived accordion below the active session list.
    pub sidebar_show_archived: bool,
    pub sidebar_collapsed_leads: Vec<String>,
    #[serde(default = "whats_new_seen_before_tracking")]
    pub whats_new_seen_version: Option<String>,
    /// The last selected space — restored on boot when the row still exists;
    /// also the new-tab default when the sidebar filter is "All".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_space_id: Option<String>,
    /// Last successfully launched Action per project in this viewport.
    #[serde(skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub last_project_action_by_space_id: std::collections::HashMap<String, String>,
    /// Open session tabs in visual order (drag-reorder edits in place).
    /// Device-local: a tab is a local viewport onto the synced session list —
    /// closing one never archives the session. Ids of archived/deleted chats
    /// are pruned against the doc ([`Shell::sync_open_tabs`]). `None` = file
    /// written by a pre-tabs build; seeded once from the last space's sessions.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_tabs: Option<Vec<String>>,
    /// Set once the stored tabs were pruned to what the old activity strip
    /// showed; tabs no longer hide when idle, so the ids it kept hidden
    /// would otherwise all surface at once.
    pub open_tabs_stable: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub collapsed_tab_groups: Vec<String>,
    /// Sidebar session filter: a space id, or `None` for "All spaces".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub space_filter: Option<String>,
    /// Custom sidebar organization, isolated between account profiles on this device.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub sidebar_sections_by_profile: HashMap<String, Vec<SidebarSection>>,
    /// Device-local pins for local profiles; synced profiles use registry pins.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub sidebar_pinned_session_ids_by_profile: HashMap<String, Vec<String>>,
    /// Master switch for session notification chimes. `ZERON_DISABLE_SOUND`
    /// overrides every per-event preference below.
    pub sound_enabled: bool,
    /// Chime when an agent run completes successfully.
    pub sound_completion_enabled: bool,
    /// Chime when an agent is waiting for user input.
    pub sound_input_enabled: bool,
    /// Chime when a run fails or the durable connection state degrades.
    pub sound_attention_enabled: bool,
    /// Desktop banner notifications on the same transitions.
    /// `ZERON_DISABLE_NOTIFICATIONS` overrides.
    pub notifications_enabled: bool,
    /// Suppress the banner while a Zeron window is focused (the chime covers
    /// the foreground case).
    pub notifications_background_only: bool,
    /// Desktop banners for newly discovered agent CLI releases. In-app chips
    /// remain enabled independently of this preference.
    pub agent_update_notifications: bool,
    pub auto_update: bool,
    pub right_pane_width: f32,
    /// Split view: the LEFT pane's share of the conversation column.
    pub split_ratio: f32,
    pub terminal_height: f32,
    /// Customizable shortcut combos (feature-inventory §1.4).
    pub keymap: KeymapConfig,
    /// macOS viewer-side Appshot capture. Device-local because the shortcut
    /// and TCC permissions belong to this desktop.
    #[cfg_attr(not(any(target_os = "macos", target_os = "linux")), serde(skip))]
    pub appshots_enabled: bool,
    #[cfg_attr(not(any(target_os = "macos", target_os = "linux")), serde(skip))]
    pub appshot_sound_enabled: bool,
    #[cfg_attr(not(any(target_os = "macos", target_os = "linux")), serde(skip))]
    pub appshot_destination: crate::appshots::AppshotDestination,
    pub computer_getting_started_acknowledged: bool,
    pub computer_effort_hint_dismissed: bool,
    /// Whether bare Escape stops the active agent after contextual consumers
    /// decline it. Device-local and opt-in.
    pub escape_stops_active_agent: bool,
    /// Read by the engine's keep-awake task as `keepAwakeWhileWorking`.
    pub keep_awake_while_working: bool,
    /// The Settings section last viewed. ⌘, / Ctrl+,, the footer gear and the
    /// palette reopen it; links naming a section replace it. Files without
    /// it, or with a name this build does not know, open on General.
    pub settings_section: crate::shell::SettingsSection,
    pub tasks_view: TasksView,
    pub tree_view: TreeView,
    /// Light/dark preference. Defaults to following the OS.
    pub appearance: crate::appearance::AppearanceMode,
    /// First-run setup was finished or skipped. A file written before this
    /// field existed reads as done (existing users never see it); only a
    /// fresh install, with no settings file yet, starts it.
    pub onboarding_completed: bool,
    /// Optional columns shown in every Git History pane.
    pub git_history_columns: GitHistoryColumns,
    /// User-adjusted widths for the resizable Git History columns.
    pub git_history_column_widths: GitHistoryColumnWidths,
    /// User-selected order for the optional Git History columns.
    pub git_history_column_order: GitHistoryColumnOrder,
    /// How authors are represented in Git History rows.
    pub git_history_author_display: GitHistoryAuthorDisplay,
    /// Width of the Git page's commit panel, dragged from its left edge.
    pub git_commit_panel_width: f32,
    /// Interface and conversational-prose family. Device-local by design.
    pub ui_font_family: crate::typography::UiFontFamily,
    /// Base size for interface and conversational prose.
    pub ui_font_size: crate::typography::UiFontSize,
    /// Terminal family and absolute pixel size. Only fixed-width families
    /// qualify, including compatible Nerd Fonts.
    pub terminal_font_family: crate::typography::UiFontFamily,
    pub terminal_font_size: f32,
    /// Family and absolute pixel size for code, diffs, and file editors.
    pub code_font_family: crate::typography::UiFontFamily,
    pub code_font_size: f32,
    /// Independently selected light and dark theme variants.
    pub theme_selection: zeron_theme::ThemeSelection,
    /// Changes pane: side-by-side diffs instead of the unified stack.
    pub diff_split: bool,
    /// Changes pane: wrap long source lines instead of scrolling horizontally.
    pub diff_wrap: bool,
    /// Agent-sent Markdown fences: wrap long lines to the chat width instead
    /// of exposing their horizontal scroll plane.
    pub code_fences_fit_content: bool,
    /// Typeset LaTeX math in Markdown. Off shows its source as code.
    pub render_math: bool,
    /// The bundled face typeset math is set in.
    pub math_font: MathFont,
    /// Typeset math size, as a percent of the text around it. Read
    /// tolerantly ([`deserialize_math_size`]).
    #[serde(deserialize_with = "deserialize_math_size")]
    pub math_size: u16,
    /// Maximum message and docked composer surface width in logical pixels.
    /// The centered new-chat composer keeps its own width.
    pub transcript_width: f32,
    /// Open a normal web-link activation in the session Browser. Explicit
    /// context-menu actions remain available regardless of this preference.
    /// Off, pull request links leave Zeron too; on, they open where
    /// [`Self::pull_request_destination`] says.
    pub open_web_links_in_zeron: bool,
    pub html_open_app: crate::html_open::HtmlOpenApp,
    /// Compact transcript: a turn's working steps (thinking, tool calls, and
    /// the narration between them) fold into one collapsed accordion, so only
    /// the reply text stays visible.
    pub transcript_compact_mode: bool,
    /// Destination shared by PR badges, the pull-request board, and pull
    /// request links in messages while [`Self::open_web_links_in_zeron`] is on.
    /// Stored as `pullRequestOpenIn`; [`Self::save`] also writes the legacy
    /// `pullRequestDestination` for older builds sharing this file.
    #[serde(rename = "pullRequestOpenIn")]
    pub pull_request_destination: PullRequestDestination,
    /// Last explicitly selected PR scope, restored when no project is selected.
    pub last_pull_request_repository: Option<String>,
    pub last_pull_request_device: Option<String>,
    pub pull_requests_tab: crate::pull_requests::PageTab,
    /// Device-local bookmarks, keyed by provider/repository/PR, never GitHub mutations.
    pub pull_request_stars: Vec<String>,
    /// Folders pinned in the new-project picker, keyed by device id.
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub project_folder_pins: HashMap<String, Vec<String>>,
    /// Save edited workspace files automatically after the configured delay.
    pub files_autosave_enabled: bool,
    /// Idle time before an edited workspace file is saved automatically.
    pub files_autosave_delay_ms: u64,
    /// Wrap long lines in workspace file editors and previews.
    pub files_word_wrap: bool,
    /// Include hidden and ignored entries in workspace file trees.
    pub files_show_all: bool,
    /// The user collapsed the sidecar beside the transcript to its icon rail.
    pub sidecar_collapsed: bool,
    /// Interactive identity overlay; imported themes default to their own accent.
    pub accent: zeron_theme::AccentSelection,
    /// Glass policy, independent from the selected appearance, theme, and accent.
    pub surface: zeron_theme::SurfacePreference,
    pub frost: FrostTuning,
    /// Optional device-local artwork behind the blank new-thread composer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_thread_composer_background: Option<NewThreadComposerBackground>,
    /// Device-local folder used by the random wallpaper shortcut.
    pub wallpaper_folder: Option<PathBuf>,
    pub wallpaper_source: Option<PathBuf>,
    /// Most recently displayed sources first; bounded by the shuffle cooldown.
    pub wallpaper_history: Vec<PathBuf>,
    pub wallpaper_theme_colors: bool,
    pub wallpaper_color: Option<zeron_theme::Color>,
    /// Non-destructive treatment composited inside the artwork's fade mask.
    pub new_thread_background_effect: NewThreadBackgroundEffect,
    /// How the same artwork shows behind an open conversation.
    pub chat_background: ChatBackground,
    pub terminal_background: TerminalBackground,
    pub shell_frame: ShellFrame,
    /// Snap animations to rest. Defaults to following the OS.
    pub reduce_motion: crate::motion::ReduceMotion,
    /// Also snap animations while the main window is not focused.
    pub pause_animations_in_background: bool,
    pub commit_message_instruction: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_message_model: Option<CommitModelChoice>,
    pub pr_chat_history: PrChatHistory,
    pub pr_chat_idle_expiry: PrChatIdleExpiry,
    pub pr_chat_sessions: std::collections::BTreeMap<String, Vec<PrChatSession>>,
    pub git_agent_busy_policy: GitAgentBusyPolicy,
    pub device_auto_float: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_float_frame: Option<crate::device_panel::float::DeviceFloatFrame>,
    pub home_widgets: HomeWidgets,
    /// Mouse-wheel notches glide instead of jumping (`smooth_scroll`).
    /// Trackpads already scroll smoothly; reduced motion overrides.
    pub smooth_scrolling: bool,
    /// Stable native Codex voice id only; devices and microphone state are never persisted.
    #[serde(default)]
    pub codex_voice: Option<String>,
    /// Local preference: never transfers an active call or credentials.
    #[serde(default)]
    pub codex_voice_device: Option<String>,
    /// Pre-theme settings used `accentColor`. Read it once, migrate to
    /// [`Self::accent`], and never write it again.
    #[serde(default, rename = "accentColor", skip_serializing)]
    legacy_accent_color: Option<crate::theme::AccentColor>,
}
