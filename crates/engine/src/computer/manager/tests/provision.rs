use std::sync::atomic::AtomicUsize;

use zeron_proto::computer::{COMPUTER_PROVISION_SUMMARY_MAX_LENGTH, ComputerHealthStatus};

use super::*;

fn provisioning(transcript: String) -> (Arc<Scripted>, Arc<AtomicUsize>) {
    let backend = Scripted::default_fake();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    backend.hooks().provision = Some(Arc::new(move |_, ()| {
        counted.fetch_add(1, Ordering::SeqCst);
        let transcript = transcript.clone();
        async move { Ok(transcript) }.boxed()
    }));
    (backend, calls)
}

#[tokio::test]
async fn engages_the_backend_first_so_setup_reads_the_live_desktop() {
    let (backend, calls) = provisioning("Computer permissions are ready.".into());
    let Built {
        manager, backend, ..
    } = manager(backend);
    manager.provision().await.unwrap();
    assert!(!backend.calls_for("availability").is_empty());
    assert!(backend.calls_for("probeAvailability").is_empty());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    manager.dispose().await;
}

#[tokio::test]
async fn refuses_without_provisioning_when_the_backend_has_nothing_to_install() {
    let Built { manager, .. } = manager(Scripted::default_fake());
    let error = manager.provision().await.unwrap_err();
    assert!(error.to_string().contains("nothing to install"), "{error}");
    assert_eq!(manager.get_status().await.provisionable, Some(false));
    manager.dispose().await;
}

#[tokio::test]
async fn clamps_the_transcript_and_answers_with_the_post_setup_status() {
    let (backend, _) = provisioning("L".repeat(100_000));
    let Built { manager, .. } = manager(backend);
    let result = manager.provision().await.unwrap();
    assert_eq!(
        result.summary.chars().count(),
        COMPUTER_PROVISION_SUMMARY_MAX_LENGTH
    );
    assert_eq!(result.status.computer_id, "desktop");
    assert_eq!(
        result.status.availability,
        ComputerAvailability::Available {
            backend: Some("fake".into())
        }
    );
    assert_eq!(result.status.health.status, ComputerHealthStatus::Connected);
    assert!(result.status.capabilities.input);
    assert_eq!(result.status.provisionable, Some(true));
    manager.dispose().await;
}
