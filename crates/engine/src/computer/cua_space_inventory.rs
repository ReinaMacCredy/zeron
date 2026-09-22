use std::collections::HashSet;

use serde_json::{Map, Value};
use zeron_proto::computer::{ComputerSpaceErrorCode, ComputerSpaceInventory};

use super::ui_tree_targeting::ComputerTargetError;

const NATIVE_FIELDS: [(&str, &str); 5] = [
    ("space_id", "id"),
    ("space_uuid", "uuid"),
    ("display_id", "displayId"),
    ("kind", "kind"),
    ("current", "current"),
];

fn decode(value: &Map<String, Value>) -> Option<ComputerSpaceInventory> {
    if value.get("source").and_then(Value::as_str) != Some("macos-managed-spaces") {
        return None;
    }
    let spaces = value
        .get("spaces")?
        .as_array()?
        .iter()
        .map(|entry| {
            let entry = entry.as_object()?;
            Some(Value::Object(
                NATIVE_FIELDS
                    .iter()
                    .filter_map(|(native, contract)| {
                        Some(((*contract).to_owned(), entry.get(*native)?.clone()))
                    })
                    .collect(),
            ))
        })
        .collect::<Option<Vec<Value>>>()?;
    let mut contract = Map::new();
    contract.insert("source".into(), value["source"].clone());
    if let Some(complete) = value.get("complete") {
        contract.insert("complete".into(), complete.clone());
    }
    contract.insert("spaces".into(), Value::Array(spaces));
    let inventory: ComputerSpaceInventory = serde_json::from_value(Value::Object(contract)).ok()?;
    let mut ids = HashSet::new();
    let mut uuids = HashSet::new();
    for space in &inventory.spaces {
        if !ids.insert(space.id) {
            return None;
        }
        if let Some(uuid) = &space.uuid
            && !uuids.insert(uuid.as_str())
        {
            return None;
        }
    }
    Some(inventory)
}

pub fn cua_space_inventory(
    value: &Map<String, Value>,
) -> Result<ComputerSpaceInventory, ComputerTargetError> {
    decode(value).ok_or_else(|| {
        ComputerTargetError::space(
            ComputerSpaceErrorCode::ComputerSpacesUnavailable,
            "The native driver did not provide a valid managed-display Space inventory. Use exact existing windows in place; do not infer empty Spaces or Space ownership from window membership.",
        )
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use zeron_proto::computer::{ComputerSpace, ComputerSpaceKind};

    use super::*;
    use crate::computer::ui_tree_targeting::ComputerTargetErrorCode;

    fn space() -> Value {
        json!({
            "space_id": 7,
            "space_uuid": "space-uuid",
            "display_id": "display-a",
            "kind": "desktop",
            "current": false,
        })
    }

    fn inventory(spaces: Vec<Value>) -> Map<String, Value> {
        json!({ "source": "macos-managed-spaces", "complete": true, "spaces": spaces })
            .as_object()
            .unwrap()
            .clone()
    }

    fn with(mut base: Value, patch: Value) -> Value {
        for (key, value) in patch.as_object().unwrap() {
            base[key] = value.clone();
        }
        base
    }

    #[test]
    fn decodes_native_managed_inventory_including_unknown_current_identity() {
        assert_eq!(
            cua_space_inventory(&inventory(vec![space()]))
                .unwrap()
                .spaces[0],
            ComputerSpace {
                id: 7,
                uuid: Some("space-uuid".into()),
                display_id: "display-a".into(),
                kind: ComputerSpaceKind::Desktop,
                current: Some(false),
            }
        );
        let mut partial = inventory(vec![with(
            space(),
            json!({ "current": null, "space_uuid": null }),
        )]);
        partial.insert("complete".into(), json!(false));
        assert_eq!(
            cua_space_inventory(&partial).unwrap().spaces[0].current,
            None
        );
    }

    #[test]
    fn rejects_guessed_malformed_and_ambiguous_inventory() {
        for value in [
            json!({ "windows": [{ "space_ids": [7] }] })
                .as_object()
                .unwrap()
                .clone(),
            inventory(vec![with(space(), json!({ "space_id": 1.5 }))]),
            inventory(vec![with(
                space(),
                json!({ "space_id": 9_007_199_254_740_992_u64 }),
            )]),
            inventory(vec![with(space(), json!({ "display_id": "" }))]),
            inventory(vec![
                space(),
                with(space(), json!({ "space_uuid": "other" })),
            ]),
            inventory(vec![space(), with(space(), json!({ "space_id": 8 }))]),
        ] {
            let error = cua_space_inventory(&value).unwrap_err();
            assert_eq!(
                error.code,
                ComputerTargetErrorCode::Space(ComputerSpaceErrorCode::ComputerSpacesUnavailable),
                "{value:?}"
            );
        }
    }

    #[test]
    fn rejects_an_inventory_whose_driver_omitted_a_space_identity_field() {
        let mut missing = space();
        missing.as_object_mut().unwrap().remove("space_uuid");
        assert!(cua_space_inventory(&inventory(vec![missing])).is_err());
    }
}
