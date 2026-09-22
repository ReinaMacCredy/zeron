//! Read-only managed-Space inventory. This module never changes the active
//! Space or creates/moves windows. Unsupported or unstable identity stays
//! unknown; a window-membership list is not a complete Space inventory.

use core_foundation::{
    array::CFArray,
    base::{CFType, TCFType},
    dictionary::CFDictionary,
    number::CFNumber,
    string::CFString,
};
use serde::Serialize;
use std::{collections::HashSet, ffi::c_void, sync::OnceLock};

use crate::input::skylight::{as_fn, find_sym};

type ConnectionFn = unsafe extern "C" fn() -> u32;
type CopyDisplaysFn = unsafe extern "C" fn(u32) -> *const c_void;
type CurrentSpaceFn = unsafe extern "C" fn(u32, *const c_void) -> u64;
const MAX_SPACES: usize = 256;
const MAX_SAFE_ID: i64 = 9_007_199_254_740_991;

#[derive(Debug, Serialize)]
pub struct ManagedSpace {
    pub space_id: u64,
    pub space_uuid: Option<String>,
    pub display_id: String,
    pub kind: &'static str,
    pub current: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ManagedSpaceInventory {
    pub source: &'static str,
    pub complete: bool,
    pub spaces: Vec<ManagedSpace>,
}

fn field(dictionary: &CFDictionary, name: &str) -> Option<CFType> {
    let key = CFString::new(name);
    let value = dictionary.find(key.as_CFTypeRef())?;
    if (*value).is_null() {
        return None;
    }
    // The dictionary retains its values; the get-rule wrapper holds its own
    // reference while the field is examined with checked CF type downcasts.
    Some(unsafe { CFType::wrap_under_get_rule(*value) })
}

fn string_field(dictionary: &CFDictionary, name: &str) -> Option<String> {
    let value = field(dictionary, name)?.downcast::<CFString>()?.to_string();
    (!value.is_empty() && value.len() <= 256).then_some(value)
}

fn number_field(dictionary: &CFDictionary, name: &str) -> Option<i64> {
    field(dictionary, name)?.downcast::<CFNumber>()?.to_i64()
}

fn space_id(dictionary: &CFDictionary) -> Option<u64> {
    let value = number_field(dictionary, "ManagedSpaceID")?;
    (value > 0 && value <= MAX_SAFE_ID).then_some(value as u64)
}

fn item(pointer: *const c_void) -> Result<CFType, &'static str> {
    if pointer.is_null() {
        return Err("Managed Space inventory contains an invalid object.");
    }
    Ok(unsafe { CFType::wrap_under_get_rule(pointer) })
}

fn parse_inventory(
    root: &CFType,
    mut read_current: impl FnMut(&str) -> Option<u64>,
) -> Result<ManagedSpaceInventory, &'static str> {
    let displays = root
        .downcast::<CFArray>()
        .ok_or("Managed display inventory is not an array.")?;
    if displays.is_empty() || displays.len() > 32 {
        return Err("Managed display inventory is absent or exceeds the supported bound.");
    }
    let mut result = ManagedSpaceInventory {
        source: "macos-managed-spaces",
        complete: true,
        spaces: Vec::new(),
    };
    let mut ids = HashSet::new();
    let mut display_ids = HashSet::new();
    let mut uuids = HashSet::new();
    for pointer in displays.iter() {
        let display = item(*pointer)?
            .downcast::<CFDictionary>()
            .ok_or("Invalid managed display record.")?;
        let display_id = string_field(&display, "Display Identifier")
            .ok_or("Managed display identity is unavailable.")?;
        if !display_ids.insert(display_id.clone()) {
            return Err("Managed display identity is ambiguous.");
        }
        let snapshot_current = field(&display, "Current Space")
            .and_then(|v| v.downcast::<CFDictionary>())
            .and_then(|v| space_id(&v));
        let observed_current = read_current(&display_id);
        let current = snapshot_current.filter(|id| Some(*id) == observed_current);
        result.complete &= current.is_some();
        let spaces = field(&display, "Spaces")
            .and_then(|v| v.downcast::<CFArray>())
            .ok_or("Managed Space list is unavailable.")?;
        if spaces.is_empty() || result.spaces.len() + spaces.len() as usize > MAX_SPACES {
            return Err("Managed Space inventory is empty or exceeds the supported bound.");
        }
        let mut current_present = false;
        for pointer in spaces.iter() {
            let space = item(*pointer)?
                .downcast::<CFDictionary>()
                .ok_or("Invalid managed Space record.")?;
            let id = space_id(&space).ok_or("Managed Space numeric identity is unavailable.")?;
            if !ids.insert(id) {
                return Err("Managed Space numeric identity is ambiguous.");
            }
            let uuid = string_field(&space, "uuid");
            if uuid
                .as_ref()
                .is_some_and(|value| !uuids.insert(value.clone()))
            {
                return Err("Managed Space UUID is ambiguous.");
            }
            current_present |= current == Some(id);
            result.spaces.push(ManagedSpace {
                space_id: id,
                space_uuid: uuid,
                display_id: display_id.clone(),
                kind: match number_field(&space, "type") {
                    Some(0) => "desktop",
                    Some(4) => "fullscreen",
                    _ => "unknown",
                },
                current: current.map(|value| value == id),
            });
        }
        if !current_present {
            result.complete = false;
            for space in result
                .spaces
                .iter_mut()
                .filter(|space| space.display_id == display_id)
            {
                space.current = None;
            }
        }
    }
    Ok(result)
}

pub fn managed_spaces() -> Result<ManagedSpaceInventory, &'static str> {
    static FUNCTIONS: OnceLock<Option<(ConnectionFn, CopyDisplaysFn, CurrentSpaceFn)>> =
        OnceLock::new();
    let (connection, copy_displays, current_space) = FUNCTIONS
        .get_or_init(|| {
            Some((
                unsafe { as_fn(find_sym(b"CGSMainConnectionID\0")?) },
                unsafe { as_fn(find_sym(b"SLSCopyManagedDisplaySpaces\0")?) },
                unsafe { as_fn(find_sym(b"SLSManagedDisplayGetCurrentSpace\0")?) },
            ))
        })
        .as_ref()
        .ok_or("This macOS version does not expose read-only managed Space inventory.")?;
    let connection_id = unsafe { connection() };
    let pointer = unsafe { copy_displays(connection_id) };
    if pointer.is_null() {
        return Err(
            "WindowServer did not provide managed Space inventory for this graphical session.",
        );
    }
    let root = unsafe { CFType::wrap_under_create_rule(pointer) };
    parse_inventory(&root, |display| {
        let display = CFString::new(display);
        let current = unsafe { current_space(connection_id, display.as_CFTypeRef()) };
        (current > 0 && current <= MAX_SAFE_ID as u64).then_some(current)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dictionary(values: &[(&str, CFType)]) -> CFType {
        CFDictionary::from_CFType_pairs(
            &values
                .iter()
                .map(|(key, value)| (CFString::new(key), value.clone()))
                .collect::<Vec<_>>(),
        )
        .into_CFType()
    }

    fn space(id: i64, uuid: &str) -> CFType {
        dictionary(&[
            ("ManagedSpaceID", CFNumber::from(id).into_CFType()),
            ("uuid", CFString::new(uuid).into_CFType()),
            ("type", CFNumber::from(0).into_CFType()),
        ])
    }

    fn inventory(spaces: &[CFType], current: i64) -> CFType {
        CFArray::from_CFTypes(&[dictionary(&[
            (
                "Display Identifier",
                CFString::new("display-a").into_CFType(),
            ),
            (
                "Current Space",
                dictionary(&[("ManagedSpaceID", CFNumber::from(current).into_CFType())]),
            ),
            ("Spaces", CFArray::from_CFTypes(spaces).into_CFType()),
        ])])
        .into_CFType()
    }

    #[test]
    fn includes_empty_spaces_without_inferring_membership() {
        let value = parse_inventory(&inventory(&[space(1, "one"), space(2, "two")], 1), |_| {
            Some(1)
        })
        .unwrap();
        assert!(value.complete);
        assert_eq!(value.spaces.len(), 2);
        assert_eq!(value.spaces[0].current, Some(true));
        assert_eq!(value.spaces[1].current, Some(false));
    }

    #[test]
    fn current_space_churn_is_unknown_not_false() {
        let value = parse_inventory(&inventory(&[space(1, "one"), space(2, "two")], 1), |_| {
            Some(2)
        })
        .unwrap();
        assert!(!value.complete);
        assert!(value.spaces.iter().all(|space| space.current.is_none()));
    }

    #[test]
    fn duplicate_or_unsafe_ids_refuse_inventory() {
        assert!(parse_inventory(
            &inventory(&[space(1, "one"), space(1, "two")], 1),
            |_| Some(1)
        )
        .is_err());
        assert!(
            parse_inventory(&inventory(&[space(MAX_SAFE_ID + 1, "one")], 1), |_| Some(1)).is_err()
        );
        assert!(
            parse_inventory(&inventory(&[space(1, "same"), space(2, "same")], 1), |_| {
                Some(1)
            })
            .is_err()
        );
    }

    #[test]
    fn wrong_cf_shapes_are_rejected_without_unchecked_casts() {
        assert!(parse_inventory(&CFString::new("wrong").into_CFType(), |_| Some(1)).is_err());
        assert!(parse_inventory(
            &inventory(&[CFString::new("wrong").into_CFType()], 1),
            |_| Some(1)
        )
        .is_err());
    }
}
