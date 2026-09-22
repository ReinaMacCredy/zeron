use zeron_proto::computer::{
    ComputerAvailability, ComputerBuildSignature, ComputerPermission,
    computer_grants_block_control, list_computer_permissions,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerSetupSignal {
    pub missing: Vec<ComputerPermission>,
    pub blocking: bool,
    pub build_signature: Option<ComputerBuildSignature>,
    pub bundle_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ComputerSetupInput<'a> {
    pub failed_for_setup: bool,
    pub availability: Option<&'a ComputerAvailability>,
    pub missing: &'a [ComputerPermission],
    pub build_signature: Option<ComputerBuildSignature>,
    pub bundle_id: Option<&'a str>,
}

pub fn responsible_desktop_bundle_id(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

pub fn computer_setup_signal(input: ComputerSetupInput<'_>) -> Option<ComputerSetupSignal> {
    let bundle_id = responsible_desktop_bundle_id(input.bundle_id);
    if let Some(ComputerAvailability::PermissionRequired {
        missing,
        build_signature,
        ..
    }) = input.availability
    {
        return Some(ComputerSetupSignal {
            missing: missing.clone(),
            blocking: true,
            build_signature: Some(*build_signature),
            bundle_id,
        });
    }
    if input.failed_for_setup {
        return Some(ComputerSetupSignal {
            missing: input.missing.to_vec(),
            blocking: true,
            build_signature: input.build_signature,
            bundle_id,
        });
    }
    (!input.missing.is_empty()).then(|| ComputerSetupSignal {
        missing: input.missing.to_vec(),
        blocking: computer_grants_block_control(input.missing),
        build_signature: input.build_signature,
        bundle_id,
    })
}

pub fn computer_setup_tool_note(signal: &ComputerSetupSignal) -> String {
    let labels = list_computer_permissions(&signal.missing);
    let needed = if labels.is_empty() {
        "a macOS privacy permission"
    } else {
        labels.as_str()
    };
    let asked =
        format!("Zeron needs {needed} and has shown the user a setup card with a guided flow.");
    if signal.blocking {
        return format!(
            "{asked} Nothing on the desktop can be driven without it. Stop desktop automation, say in one sentence that you are waiting for the user to grant it, and do not retry or work around it until the user says it is granted."
        );
    }
    format!(
        "{asked} This one does not block desktop control — only the pictures: screenshots and the computer pane will fail while it is missing, and the window list, the accessibility state and every input still work. Do not stop. Carry on with those, say once that you cannot see the screen until the user grants it, and target controls by label rather than by coordinates."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUNDLE: &str = "sh.zeron.app.dev";

    #[test]
    fn reads_the_desktop_hosts_identifier_and_treats_blank_as_unknown() {
        assert_eq!(
            responsible_desktop_bundle_id(Some(" com.x.zeron ")).as_deref(),
            Some("com.x.zeron")
        );
        assert_eq!(responsible_desktop_bundle_id(Some("   ")), None);
        assert_eq!(responsible_desktop_bundle_id(None), None);
    }

    #[test]
    fn carries_the_responsible_app_onto_every_signal_it_raises() {
        let availability = ComputerAvailability::PermissionRequired {
            missing: vec![ComputerPermission::Accessibility],
            message: "needs Accessibility".to_owned(),
            build_signature: ComputerBuildSignature::Adhoc,
            bundle_id: None,
        };
        assert_eq!(
            computer_setup_signal(ComputerSetupInput {
                availability: Some(&availability),
                bundle_id: Some(BUNDLE),
                ..ComputerSetupInput::default()
            }),
            Some(ComputerSetupSignal {
                missing: vec![ComputerPermission::Accessibility],
                blocking: true,
                build_signature: Some(ComputerBuildSignature::Adhoc),
                bundle_id: Some(BUNDLE.to_owned()),
            })
        );
        assert_eq!(
            computer_setup_signal(ComputerSetupInput {
                failed_for_setup: true,
                bundle_id: Some(BUNDLE),
                ..ComputerSetupInput::default()
            }),
            Some(ComputerSetupSignal {
                missing: vec![],
                blocking: true,
                build_signature: None,
                bundle_id: Some(BUNDLE.to_owned()),
            })
        );
        assert_eq!(
            computer_setup_signal(ComputerSetupInput {
                missing: &[ComputerPermission::ScreenRecording],
                bundle_id: Some(BUNDLE),
                ..ComputerSetupInput::default()
            }),
            Some(ComputerSetupSignal {
                missing: vec![ComputerPermission::ScreenRecording],
                blocking: false,
                build_signature: None,
                bundle_id: Some(BUNDLE.to_owned()),
            })
        );
    }

    #[test]
    fn omits_the_app_entirely_when_no_desktop_host_is_responsible() {
        assert_eq!(
            computer_setup_signal(ComputerSetupInput {
                failed_for_setup: true,
                missing: &[ComputerPermission::Accessibility],
                ..ComputerSetupInput::default()
            }),
            Some(ComputerSetupSignal {
                missing: vec![ComputerPermission::Accessibility],
                blocking: true,
                build_signature: None,
                bundle_id: None,
            })
        );
    }

    #[test]
    fn still_reports_nothing_to_do_when_no_grant_is_missing_and_nothing_failed() {
        assert_eq!(
            computer_setup_signal(ComputerSetupInput {
                bundle_id: Some(BUNDLE),
                ..ComputerSetupInput::default()
            }),
            None
        );
        assert_eq!(
            computer_setup_signal(ComputerSetupInput {
                failed_for_setup: false,
                bundle_id: Some(BUNDLE),
                ..ComputerSetupInput::default()
            }),
            None
        );
    }

    #[test]
    fn tells_the_model_to_stop_only_for_a_blocking_grant() {
        let blocking = ComputerSetupSignal {
            missing: vec![
                ComputerPermission::ScreenRecording,
                ComputerPermission::Accessibility,
            ],
            blocking: true,
            build_signature: None,
            bundle_id: None,
        };
        let note = computer_setup_tool_note(&blocking);
        assert!(note.starts_with("Zeron needs Accessibility and Screen Recording and has shown"));
        assert!(note.contains("Stop desktop automation"));
        let degraded = ComputerSetupSignal {
            missing: vec![ComputerPermission::ScreenRecording],
            blocking: false,
            ..blocking.clone()
        };
        assert!(computer_setup_tool_note(&degraded).contains("Do not stop."));
        let unnamed = ComputerSetupSignal {
            missing: vec![],
            ..blocking
        };
        assert!(
            computer_setup_tool_note(&unnamed)
                .starts_with("Zeron needs a macOS privacy permission")
        );
    }
}
