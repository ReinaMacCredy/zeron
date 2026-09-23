use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;
use zeron_proto::computer::ComputerAvailability;
use zeron_rpc::ComputerHost;

use super::approval_gate::ComputerApprovalGate;
use super::backend::{ComputerBackend, clamp_computer_message};
use super::control_state::ComputerControlState;
use super::cua_backend::{CuaComputerBackend, CuaComputerBackendOptions};
use super::manager::{
    ComputerManager, ComputerManagerEnv, ComputerManagerOptions, ComputerManagerSeams,
    DefaultComputerManagerSeams,
};
use super::tools::ComputerTools;
use super::unavailable_backend::{UnavailableComputerBackend, UnavailableComputerBackendOptions};

const NO_HOST_MESSAGE: &str = "No Zeron desktop host is connected to this engine. Open Zeron on this Mac so the agent can use the computer.";
const DISABLED_MESSAGE: &str = "Computer support is disabled by the service configuration.";

pub struct ComputerServiceOptions {
    pub backend: Option<Arc<dyn ComputerBackend>>,
    pub host: Option<Arc<dyn ComputerHost>>,
    pub cua: CuaComputerBackendOptions,
    pub supported: Option<bool>,
    pub platform: Option<String>,
    pub data_dir: Option<PathBuf>,
    pub control_state: Arc<ComputerControlState>,
    pub approval_gate: ComputerApprovalGate,
    pub seams: Arc<dyn ComputerManagerSeams>,
    pub env: ComputerManagerEnv,
}

impl Default for ComputerServiceOptions {
    fn default() -> Self {
        Self {
            backend: None,
            host: None,
            cua: CuaComputerBackendOptions::default(),
            supported: None,
            platform: None,
            data_dir: None,
            control_state: Arc::default(),
            approval_gate: ComputerApprovalGate::default(),
            seams: Arc::new(DefaultComputerManagerSeams),
            env: ComputerManagerEnv::from_env(),
        }
    }
}

pub struct ComputerService {
    pub supported: bool,
    pub availability: ComputerAvailability,
    pub manager: Arc<ComputerManager>,
    pub tools: Arc<ComputerTools>,
}

fn unavailable_backend(platform: &str) -> UnavailableComputerBackend {
    let (message, availability) = match platform {
        "macos" => (
            NO_HOST_MESSAGE.to_owned(),
            ComputerAvailability::BackendUnavailable {
                message: NO_HOST_MESSAGE.to_owned(),
            },
        ),
        "linux" => (
            format!("No computer backend is configured for this server running on {platform}."),
            ComputerAvailability::BackendUnavailable {
                message: "No computer backend is available on this server.".to_owned(),
            },
        ),
        _ => (
            format!("No computer backend is configured for this server running on {platform}."),
            ComputerAvailability::UnsupportedPlatform {
                platform: platform.to_owned(),
            },
        ),
    };
    UnavailableComputerBackend::new(
        &message,
        UnavailableComputerBackendOptions {
            availability: Some(availability),
            ..UnavailableComputerBackendOptions::default()
        },
    )
}

impl ComputerService {
    pub async fn boot(options: ComputerServiceOptions) -> Self {
        let platform = options
            .platform
            .unwrap_or_else(|| std::env::consts::OS.to_owned());
        let (backend, routable): (Arc<dyn ComputerBackend>, bool) =
            match (options.backend, options.host) {
                (Some(backend), _) => (backend, true),
                (None, Some(host)) => (Arc::new(CuaComputerBackend::new(host, options.cua)), true),
                (None, None) => (Arc::new(unavailable_backend(&platform)), false),
            };
        if options.data_dir.is_none() {
            options.seams.warn(
                "computer data directory unavailable; the audit log and scroll gearing stay in memory",
                json!({}),
            );
        }
        let mut manager_options = ComputerManagerOptions::new(backend.clone());
        manager_options.data_dir = options.data_dir;
        manager_options.control_state = options.control_state;
        manager_options.approval_gate = options.approval_gate;
        manager_options.seams = options.seams;
        manager_options.env = options.env;
        let manager = ComputerManager::new(manager_options);
        let availability = match options.supported {
            None => backend.probe_availability().await.unwrap_or_else(|error| {
                ComputerAvailability::BackendUnavailable {
                    message: clamp_computer_message(
                        &error.to_string(),
                        "The computer backend failed.",
                    ),
                }
            }),
            Some(true) => ComputerAvailability::Available {
                backend: Some("test-override".to_owned()),
            },
            Some(false) => ComputerAvailability::BackendUnavailable {
                message: DISABLED_MESSAGE.to_owned(),
            },
        };
        Self {
            supported: options.supported.unwrap_or(routable),
            availability,
            tools: ComputerTools::new(manager.clone(), Vec::new()),
            manager,
        }
    }

    pub async fn dispose(&self) {
        self.manager.dispose().await;
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use zeron_proto::computer::{ComputerHostCall, CuaReply};

    use super::*;
    use crate::computer::fake_backend::{FakeComputerBackend, FakeComputerBackendOptions};

    fn fake() -> Arc<FakeComputerBackend> {
        Arc::new(FakeComputerBackend::new(
            FakeComputerBackendOptions::default(),
        ))
    }

    fn methods(backend: &FakeComputerBackend) -> Vec<String> {
        backend
            .calls()
            .into_iter()
            .map(|call| call.method)
            .collect()
    }

    fn fake_available() -> ComputerAvailability {
        ComputerAvailability::Available {
            backend: Some("fake".to_owned()),
        }
    }

    fn with_backend(backend: Arc<FakeComputerBackend>) -> ComputerServiceOptions {
        ComputerServiceOptions {
            backend: Some(backend),
            env: ComputerManagerEnv::default(),
            ..ComputerServiceOptions::default()
        }
    }

    struct UnreachableHost;

    #[async_trait]
    impl ComputerHost for UnreachableHost {
        async fn request(&self, _call: ComputerHostCall) -> CuaReply {
            CuaReply {
                ok: false,
                error: Some("The host endpoint is unreachable.".to_owned()),
                ..CuaReply::default()
            }
        }

        fn cancel(&self, _correlation_id: &str) {}
    }

    #[tokio::test]
    async fn boots_and_seeds_a_thread_without_ever_asking_the_backend_for_the_desktop() {
        let backend = fake();
        let service = ComputerService::boot(with_backend(backend.clone())).await;
        assert!(service.supported);
        assert_eq!(service.availability, fake_available());
        assert_eq!(methods(&backend), vec!["probeAvailability"]);
        let seeded = service.manager.get_thread_state("thread-boot").await;
        assert_eq!(seeded.availability, fake_available());
        assert!(seeded.windows.is_empty());
        assert_eq!(
            methods(&backend),
            vec!["probeAvailability", "probeAvailability"]
        );
        service.dispose().await;
    }

    #[tokio::test]
    async fn stays_supported_when_the_boot_probe_merely_reports_the_backend_unavailable() {
        let backend = fake();
        backend.set_availability(ComputerAvailability::BackendUnavailable {
            message: "The backend is not available yet.".to_owned(),
        });
        let service = ComputerService::boot(with_backend(backend)).await;
        assert!(service.supported);
        assert!(matches!(
            service.availability,
            ComputerAvailability::BackendUnavailable { .. }
        ));
        service.dispose().await;
    }

    #[tokio::test]
    async fn keeps_the_configured_override_ahead_of_both_reads() {
        let backend = fake();
        let service = ComputerService::boot(ComputerServiceOptions {
            supported: Some(false),
            ..with_backend(backend.clone())
        })
        .await;
        assert!(!service.supported);
        assert!(matches!(
            service.availability,
            ComputerAvailability::BackendUnavailable { .. }
        ));
        assert!(backend.calls().is_empty());
        service.dispose().await;
    }

    #[tokio::test]
    async fn reports_an_unsupported_platform_instead_of_a_fake_desktop_on_windows() {
        let service = ComputerService::boot(ComputerServiceOptions {
            platform: Some("windows".to_owned()),
            env: ComputerManagerEnv::default(),
            ..ComputerServiceOptions::default()
        })
        .await;
        let unsupported = ComputerAvailability::UnsupportedPlatform {
            platform: "windows".to_owned(),
        };
        assert!(!service.supported);
        assert_eq!(service.availability, unsupported);
        let state = service.manager.get_thread_state("thread-windows").await;
        assert_eq!(state.availability, unsupported);
        service.dispose().await;
    }

    #[tokio::test]
    async fn refuses_a_mac_engine_with_no_desktop_host_until_one_registers() {
        let service = ComputerService::boot(ComputerServiceOptions {
            platform: Some("macos".to_owned()),
            env: ComputerManagerEnv::default(),
            ..ComputerServiceOptions::default()
        })
        .await;
        assert!(!service.supported);
        assert_eq!(
            service.availability,
            ComputerAvailability::BackendUnavailable {
                message: NO_HOST_MESSAGE.to_owned()
            }
        );
        let error = service
            .manager
            .click(
                Some("thread"),
                zeron_proto::computer::ComputerTarget {
                    x: Some(1.0),
                    y: Some(1.0),
                    ..Default::default()
                },
                &[],
                Default::default(),
            )
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("No Zeron desktop host"),
            "{error}"
        );
        service.dispose().await;
    }

    #[tokio::test]
    async fn routes_a_real_backend_on_windows_when_a_host_is_registered() {
        let service = ComputerService::boot(ComputerServiceOptions {
            platform: Some("windows".to_owned()),
            host: Some(Arc::new(UnreachableHost)),
            env: ComputerManagerEnv::default(),
            ..ComputerServiceOptions::default()
        })
        .await;
        assert!(service.supported);
        assert!(!matches!(
            service.availability,
            ComputerAvailability::UnsupportedPlatform { .. }
        ));
        service.dispose().await;
    }
}
