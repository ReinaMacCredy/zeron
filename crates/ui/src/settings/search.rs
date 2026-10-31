//! Landing on a settings row: each row carries its [`SettingId`] (or a
//! shortcut's [`ShortcutId`]) as a [`SearchAnchor`], and a pick from the
//! Settings search ([`crate::shell::SettingsSection::matching_rows`]) asks
//! [`reveal`] to glide that row into view.

mod reveal;

pub use reveal::{PageReveal, SearchAnchor, clear_reveal, page_reveal, pending, reveal};
#[cfg(test)]
pub(crate) use reveal::{lit, take_anchored};

use super::ShortcutId;
use crate::shell::SettingsSection;

/// A settings row a search pick can land on: the key its [`SearchAnchor`]
/// carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SettingId {
    SendMessagesWith,
    FollowUpBehavior,
    ThinkingLevelShortcut,
    CompactMode,
    CompactModelPicker,
    EffortSlider,
    EscapeStopsAgent,
    SessionSwitcher,
    SessionSwitcherLimit,
    CtrlTabCyclesTabs,
    PullRequestDestination,
    ThreadNaming,
    ShowPredictions,
    WorktreeLocation,
    CommitCredit,
    ColorScheme,
    LightTheme,
    DarkTheme,
    AccentColor,
    WallpaperColors,
    Glass,
    InsetCard,
    SidebarHoverReveal,
    NewThreadBackground,
    WallpaperFolder,
    BackgroundEffect,
    ReduceMotion,
    PauseAnimations,
    SmoothScrolling,
    ThemeLibrary,
    InterfaceFont,
    TerminalFont,
    CodeFont,
    ConversationWidth,
    RenderMath,
    MathFont,
    MathSize,
    DesktopNotifications,
    AgentUpdates,
    BackgroundOnly,
    SessionSounds,
    TaskCompletedSound,
    InputRequiredSound,
    ErrorSound,
    CodexVoiceDevice,
    CodexVoiceStyle,
    Dictation,
    Microphone,
    SpeechModel,
    SkillDollar,
    SkillSlash,
    CloudUsage,
    CloudGitHub,
    Credits,
    Plan,
    ExtraCredits,
    Autosave,
    AutosaveDelay,
    WordWrap,
    ShowAllFiles,
    AppshotsOverview,
    CaptureAppshots,
    AppshotSound,
    AppshotDestination,
    InputMonitoring,
    WindowCapture,
    ApplicationText,
    AppshotPermissions,
    QuickCapture,
}

impl SettingId {
    /// The row a hit lands on while this one is hidden: a sub-setting shows
    /// only once its parent is on (or, for a background effect, set).
    pub fn parent(self) -> Option<SettingId> {
        match self {
            SettingId::BackgroundEffect => Some(SettingId::NewThreadBackground),
            SettingId::Microphone | SettingId::SpeechModel => Some(SettingId::Dictation),
            SettingId::AutosaveDelay => Some(SettingId::Autosave),
            SettingId::AppshotsOverview => Some(SettingId::CaptureAppshots),
            SettingId::InputMonitoring => Some(SettingId::WindowCapture),
            _ => None,
        }
    }

    pub fn in_advanced_look(self) -> bool {
        matches!(
            self,
            SettingId::WallpaperColors
                | SettingId::Glass
                | SettingId::InsetCard
                | SettingId::SidebarHoverReveal
                | SettingId::NewThreadBackground
                | SettingId::WallpaperFolder
                | SettingId::BackgroundEffect
                | SettingId::PauseAnimations
                | SettingId::SmoothScrolling
                | SettingId::ThemeLibrary
                | SettingId::RenderMath
                | SettingId::MathFont
                | SettingId::MathSize
        )
    }

    /// A row each provider shows in its expanded details on Providers.
    pub fn per_provider(self) -> bool {
        matches!(self, SettingId::SkillDollar | SettingId::SkillSlash)
    }
}

/// Where a hit lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// A settings row.
    Setting(SettingId),
    /// A shortcut's row: on Shortcuts, or on the feature page that owns it.
    Shortcut(ShortcutId),
    /// A page as a whole, opened at its top.
    Page(SettingsSection),
}

impl Target {
    /// Where a hit lands while this row is hidden. A provider's own rows show
    /// only once it is expanded: until then, the provider list, to pick one.
    pub(crate) fn fallback(self) -> Option<Target> {
        match self {
            Target::Setting(id) if id.per_provider() => {
                Some(Target::Page(SettingsSection::Harnesses))
            }
            Target::Setting(id) => id.parent().map(Target::Setting),
            Target::Shortcut(_) | Target::Page(_) => None,
        }
    }
}

impl Target {
    pub fn in_advanced_look(self) -> bool {
        matches!(self, Target::Setting(id) if id.in_advanced_look())
    }
}

impl From<SettingId> for Target {
    fn from(id: SettingId) -> Self {
        Target::Setting(id)
    }
}

impl From<ShortcutId> for Target {
    fn from(id: ShortcutId) -> Self {
        Target::Shortcut(id)
    }
}
