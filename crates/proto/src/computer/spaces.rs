use serde::{Deserialize, Deserializer, Serialize};

use super::bounds::{self, MAX_SAFE_INTEGER};

pub const COMPUTER_SPACES_MAX_LENGTH: usize = 256;

fn space_identity<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let value = bounds::text::<D, 256>(d)?;
    if value.is_empty() {
        return Err(serde::de::Error::custom(
            "expected a non-empty Space identity",
        ));
    }
    Ok(value)
}

fn opt_space_identity<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let value = bounds::opt_text::<D, 256>(d)?;
    if value.as_deref() == Some("") {
        return Err(serde::de::Error::custom(
            "expected a non-empty Space identity",
        ));
    }
    Ok(value)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComputerSpaceKind {
    Desktop,
    Fullscreen,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerSpace {
    #[serde(deserialize_with = "bounds::int::<_, u64, 1, MAX_SAFE_INTEGER>")]
    pub id: u64,
    #[serde(deserialize_with = "opt_space_identity")]
    pub uuid: Option<String>,
    #[serde(deserialize_with = "space_identity")]
    pub display_id: String,
    pub kind: ComputerSpaceKind,
    pub current: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComputerSpaceSource {
    #[serde(rename = "macos-managed-spaces")]
    MacosManagedSpaces,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputerSpaceInventory {
    #[serde(deserialize_with = "bounds::items::<_, _, 0, COMPUTER_SPACES_MAX_LENGTH>")]
    pub spaces: Vec<ComputerSpace>,
    pub complete: bool,
    pub source: ComputerSpaceSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerSpaceErrorCode {
    ComputerSpacesUnavailable,
    ComputerSpaceNotFound,
    ComputerSpaceCurrent,
    ComputerSpaceIdentityUnproven,
    ComputerSpaceNotDesignated,
    ComputerSpaceReserved,
    ComputerSpaceReservationRequired,
    ComputerSpaceReservationChanged,
    ComputerSpaceReservationLimit,
    ComputerSpaceWindowNotSelected,
    ComputerSpaceTargetOutsideReservation,
    ComputerSpaceTargetMembershipUnproven,
    ComputerSpaceOperationUnsupported,
}

impl ComputerSpaceErrorCode {
    pub const ALL: [Self; 13] = [
        Self::ComputerSpacesUnavailable,
        Self::ComputerSpaceNotFound,
        Self::ComputerSpaceCurrent,
        Self::ComputerSpaceIdentityUnproven,
        Self::ComputerSpaceNotDesignated,
        Self::ComputerSpaceReserved,
        Self::ComputerSpaceReservationRequired,
        Self::ComputerSpaceReservationChanged,
        Self::ComputerSpaceReservationLimit,
        Self::ComputerSpaceWindowNotSelected,
        Self::ComputerSpaceTargetOutsideReservation,
        Self::ComputerSpaceTargetMembershipUnproven,
        Self::ComputerSpaceOperationUnsupported,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerSpaceReservation {
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub space_id: u64,
    pub space_uuid: String,
    pub display_id: String,
    pub selected_window_id: Option<String>,
    pub selected_pid: Option<u32>,
    pub invalid_reason: Option<ComputerSpaceErrorCode>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn inventory() -> Value {
        json!({
            "spaces": [
                { "id": 3, "uuid": "A-B", "displayId": "main", "kind": "desktop", "current": true },
                { "id": 9, "uuid": null, "displayId": "side", "kind": "fullscreen", "current": null },
            ],
            "complete": false,
            "source": "macos-managed-spaces",
        })
    }

    #[test]
    fn inventory_round_trips_with_null_identities_on_the_wire() {
        let decoded: ComputerSpaceInventory = serde_json::from_value(inventory()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), inventory());
    }

    #[test]
    fn inventory_rejects_bad_ids_identities_and_sources() {
        for (pointer, value) in [
            ("/spaces/0/id", json!(0)),
            ("/spaces/0/id", json!(1.5)),
            ("/spaces/0/id", json!(MAX_SAFE_INTEGER + 1)),
            ("/spaces/0/uuid", json!("")),
            ("/spaces/0/displayId", json!("x".repeat(257))),
            ("/spaces/0/kind", json!("stage")),
            ("/source", json!("yabai")),
        ] {
            let mut bad = inventory();
            *bad.pointer_mut(pointer).unwrap() = value.clone();
            assert!(
                serde_json::from_value::<ComputerSpaceInventory>(bad).is_err(),
                "{pointer} = {value}"
            );
        }
    }

    #[test]
    fn error_codes_serialize_as_their_wire_names() {
        assert_eq!(
            serde_json::to_value(ComputerSpaceErrorCode::ComputerSpacesUnavailable).unwrap(),
            json!("computer_spaces_unavailable")
        );
        assert_eq!(
            serde_json::to_value(ComputerSpaceErrorCode::ComputerSpaceTargetMembershipUnproven)
                .unwrap(),
            json!("computer_space_target_membership_unproven")
        );
    }
}
