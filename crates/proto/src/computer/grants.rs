use serde::{Deserialize, Serialize};

use super::contract::{ComputerBuildSignature, ComputerPermission};

pub const COMPUTER_PERMISSIONS: [ComputerPermission; 3] = [
    ComputerPermission::Accessibility,
    ComputerPermission::ScreenRecording,
    ComputerPermission::InputMonitoring,
];

pub const COMPUTER_BLOCKING_PERMISSIONS: [ComputerPermission; 2] = [
    ComputerPermission::Accessibility,
    ComputerPermission::InputMonitoring,
];

impl ComputerPermission {
    pub fn label(self) -> &'static str {
        match self {
            Self::Accessibility => "Accessibility",
            Self::ScreenRecording => "Screen Recording",
            Self::InputMonitoring => "Input Monitoring",
        }
    }

    pub fn tcc_service_name(self) -> &'static str {
        match self {
            Self::Accessibility => "Accessibility",
            Self::ScreenRecording => "ScreenCapture",
            Self::InputMonitoring => "ListenEvent",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComputerGrantState {
    Granted,
    Denied,
    NotDetermined,
    Restricted,
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerGrantSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessibility_permission: Option<ComputerGrantState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_recording_permission: Option<ComputerGrantState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_monitoring_permission: Option<ComputerGrantState>,
}

pub fn missing_computer_permissions(snapshot: &ComputerGrantSnapshot) -> Vec<ComputerPermission> {
    COMPUTER_PERMISSIONS
        .into_iter()
        .filter(|permission| {
            let state = match permission {
                ComputerPermission::Accessibility => snapshot.accessibility_permission,
                ComputerPermission::ScreenRecording => snapshot.screen_recording_permission,
                ComputerPermission::InputMonitoring => snapshot.input_monitoring_permission,
            };
            state != Some(ComputerGrantState::Granted)
        })
        .collect()
}

pub fn computer_grants_block_control(permissions: &[ComputerPermission]) -> bool {
    COMPUTER_BLOCKING_PERMISSIONS
        .iter()
        .any(|permission| permissions.contains(permission))
}

pub fn sort_computer_permissions(permissions: &[ComputerPermission]) -> Vec<ComputerPermission> {
    COMPUTER_PERMISSIONS
        .into_iter()
        .filter(|permission| permissions.contains(permission))
        .collect()
}

pub fn list_computer_permissions(permissions: &[ComputerPermission]) -> String {
    let labels: Vec<&str> = sort_computer_permissions(permissions)
        .into_iter()
        .map(ComputerPermission::label)
        .collect();
    match labels.as_slice() {
        [] => String::new(),
        [only] => (*only).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

pub fn computer_stale_grant_advice(
    permissions: &[ComputerPermission],
    build_signature: ComputerBuildSignature,
    bundle_id: Option<&str>,
) -> Option<String> {
    if build_signature != ComputerBuildSignature::Adhoc {
        return None;
    }
    let sorted = sort_computer_permissions(permissions);
    if sorted.is_empty() {
        return None;
    }
    let base = "This is a locally built copy of Zeron, so macOS may already list it with the switch on from \
        an earlier build. Remove this app from the permission list, add the current build again, \
        then fully quit and reopen it.";
    let Some(bundle_id) = bundle_id.map(str::trim).filter(|id| !id.is_empty()) else {
        return Some(base.to_string());
    };
    let commands = sorted
        .iter()
        .map(|permission| {
            format!(
                "tccutil reset {} {bundle_id}",
                permission.tcc_service_name()
            )
        })
        .collect::<Vec<_>>()
        .join(", then ");
    Some(format!(
        "{base} To clear only this app's old grant, run `{commands}` in Terminal and grant access again."
    ))
}

pub fn computer_permission_setup_message(
    permissions: &[ComputerPermission],
    build_signature: ComputerBuildSignature,
    bundle_id: Option<&str>,
) -> String {
    let labels = list_computer_permissions(permissions);
    let base = if labels.is_empty() {
        "Zeron needs a macOS privacy permission to control this Mac. Grant it in System Settings › Privacy & Security, then try again.".to_string()
    } else {
        format!(
            "Zeron needs {labels} to control this Mac. Turn Zeron on in System Settings › Privacy & Security › {labels}, then try again."
        )
    };
    match computer_stale_grant_advice(permissions, build_signature, bundle_id) {
        Some(advice) => format!("{base} {advice}"),
        None => base,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ComputerPermission::{Accessibility, InputMonitoring, ScreenRecording};

    #[test]
    fn requires_positive_grant_evidence_for_every_computer_permission() {
        assert_eq!(
            missing_computer_permissions(&ComputerGrantSnapshot {
                accessibility_permission: None,
                screen_recording_permission: Some(ComputerGrantState::Denied),
                input_monitoring_permission: Some(ComputerGrantState::Unknown),
            }),
            vec![Accessibility, ScreenRecording, InputMonitoring]
        );
        assert_eq!(
            missing_computer_permissions(&ComputerGrantSnapshot {
                accessibility_permission: Some(ComputerGrantState::Granted),
                screen_recording_permission: Some(ComputerGrantState::Granted),
                input_monitoring_permission: Some(ComputerGrantState::Denied),
            }),
            vec![InputMonitoring]
        );
        assert_eq!(
            missing_computer_permissions(&ComputerGrantSnapshot {
                accessibility_permission: Some(ComputerGrantState::Granted),
                screen_recording_permission: Some(ComputerGrantState::Granted),
                input_monitoring_permission: Some(ComputerGrantState::Granted),
            }),
            vec![]
        );
    }

    #[test]
    fn names_grants_in_one_fixed_order_whatever_order_they_arrive_in() {
        assert_eq!(
            sort_computer_permissions(&[ScreenRecording, Accessibility]),
            vec![Accessibility, ScreenRecording]
        );
        assert_eq!(
            list_computer_permissions(&[ScreenRecording, Accessibility]),
            "Accessibility and Screen Recording"
        );
        assert_eq!(
            list_computer_permissions(&[ScreenRecording]),
            "Screen Recording"
        );
        assert_eq!(list_computer_permissions(&[]), "");
        assert_eq!(
            list_computer_permissions(&[InputMonitoring, ScreenRecording, Accessibility]),
            "Accessibility, Screen Recording and Input Monitoring"
        );
    }

    #[test]
    fn explains_a_stale_grant_on_an_adhoc_build_naming_the_right_tccutil_service() {
        let advice = computer_stale_grant_advice(
            &[Accessibility, ScreenRecording],
            ComputerBuildSignature::Adhoc,
            Some("sh.zeron.app.dev"),
        )
        .unwrap();
        assert!(advice.contains("add the current build again"));
        assert!(!advice.contains("Zeron has cleared"));
        assert!(advice.contains("tccutil reset Accessibility sh.zeron.app.dev"));
        assert!(advice.contains("tccutil reset ScreenCapture sh.zeron.app.dev"));
    }

    #[test]
    fn names_the_responsible_app_rather_than_assuming_the_released_one() {
        let advice = computer_stale_grant_advice(
            &[Accessibility],
            ComputerBuildSignature::Adhoc,
            Some("com.example.zeron.canary"),
        )
        .unwrap();
        assert!(advice.contains("tccutil reset Accessibility com.example.zeron.canary"));
        assert!(!advice.contains("sh.zeron.app"));
    }

    #[test]
    fn withholds_the_tccutil_sentence_when_no_responsible_bundle_id_is_known() {
        for bundle_id in [None, Some(""), Some("   ")] {
            let advice = computer_stale_grant_advice(
                &[Accessibility],
                ComputerBuildSignature::Adhoc,
                bundle_id,
            )
            .unwrap();
            assert!(advice.contains("add the current build again"));
            assert!(!advice.contains("tccutil"));
            assert!(!advice.contains("If none appears"));
        }
        assert!(
            !computer_permission_setup_message(
                &[Accessibility],
                ComputerBuildSignature::Adhoc,
                None
            )
            .contains("tccutil")
        );
    }

    #[test]
    fn says_nothing_about_stale_grants_on_a_signed_build() {
        assert_eq!(
            computer_stale_grant_advice(
                &[Accessibility],
                ComputerBuildSignature::Signed,
                Some("com.example.app")
            ),
            None
        );
        let message = computer_permission_setup_message(
            &[Accessibility],
            ComputerBuildSignature::Signed,
            Some("com.example.app"),
        );
        assert!(message.contains("Accessibility"));
        assert!(message.contains("System Settings"));
        assert!(!message.contains("tccutil"));
    }

    #[test]
    fn puts_the_stale_grant_explanation_into_the_adhoc_setup_message() {
        let message = computer_permission_setup_message(
            &[Accessibility],
            ComputerBuildSignature::Adhoc,
            Some("sh.zeron.app"),
        );
        assert!(message.contains("System Settings"));
        assert!(message.contains("tccutil reset Accessibility sh.zeron.app"));
    }

    #[test]
    fn separates_the_grant_that_stops_everything_from_the_one_that_only_blinds() {
        assert!(computer_grants_block_control(&[Accessibility]));
        assert!(computer_grants_block_control(&[
            Accessibility,
            ScreenRecording
        ]));
        assert!(!computer_grants_block_control(&[ScreenRecording]));
        assert!(computer_grants_block_control(&[InputMonitoring]));
        assert!(!computer_grants_block_control(&[]));
    }
}
