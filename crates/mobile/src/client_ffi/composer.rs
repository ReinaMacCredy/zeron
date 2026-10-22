use super::PermissionMode;

#[derive(Debug, Clone, uniffi::Record)]
pub struct PermissionModeInfo {
    pub mode: PermissionMode,
    pub label: String,
    pub short_label: String,
    pub description: String,
}

#[uniffi::export]
pub fn permission_modes() -> Vec<PermissionModeInfo> {
    zeron_proto::PermissionMode::ALL
        .into_iter()
        .map(|mode| PermissionModeInfo {
            mode: mode.into(),
            label: mode.label().into(),
            short_label: mode.short_label().into(),
            description: mode.description().into(),
        })
        .collect()
}

#[uniffi::export]
pub fn follow_up_notice(queueing: bool, steer: bool, can_steer: bool) -> Option<String> {
    zeron_proto::steer_queues_notice(queueing, steer, can_steer).map(str::to_owned)
}
