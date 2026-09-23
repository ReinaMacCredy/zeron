use std::sync::Arc;

use super::*;
use crate::computer::cua_backend::{CUA_STILL_FRAME_INTERVAL_MS, cua_still_interval_ms};
use crate::computer::still_frame_publisher::MIN_STILL_INTERVAL_MS;

async fn attached_interval(f: &Fixture) -> u64 {
    f.backend.attach_stream(Arc::new(|_| {})).await.unwrap();
    f.backend.still_interval_ms()
}

#[tokio::test]
async fn arms_the_still_publisher_at_the_compiled_1000_ms_cadence_by_default() {
    assert_eq!(cua_still_interval_ms(None, None), 1_000);
    let f = fixture();
    if std::env::var_os(crate::computer::call_context::ZERON_CUA_PREVIEW_STILL_MS).is_none() {
        assert_eq!(attached_interval(&f).await, CUA_STILL_FRAME_INTERVAL_MS);
    }
    f.backend.dispose().await;
}

#[tokio::test]
async fn zeron_cua_preview_still_ms_overrides_the_still_cadence() {
    assert_eq!(cua_still_interval_ms(None, Some("4000")), 4_000);
}

#[tokio::test]
async fn an_unparsable_zeron_cua_preview_still_ms_falls_back_to_the_default() {
    assert_eq!(cua_still_interval_ms(None, Some("fast")), 1_000);
}

#[tokio::test]
async fn the_constructor_still_interval_wins_and_stays_above_the_publisher_floor() {
    assert_eq!(cua_still_interval_ms(Some(750), Some("4000")), 750);
    let f = fixture_with(FixtureOptions {
        still_interval_ms: Some(750),
        ..Default::default()
    });
    assert_eq!(attached_interval(&f).await, 750);
    f.backend.dispose().await;
    let floored = fixture_with(FixtureOptions {
        still_interval_ms: Some(5),
        ..Default::default()
    });
    assert_eq!(attached_interval(&floored).await, MIN_STILL_INTERVAL_MS);
    floored.backend.dispose().await;
}
