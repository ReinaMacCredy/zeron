use std::collections::HashMap;

use serde_json::{Map, Value, json};

use crate::computer::element_identity::computer_element_ref_identity;
use crate::computer::ui_tree_targeting::{
    ComputerActionableElement, ComputerActionableElementChange, ComputerActionableElementRef,
    ComputerActionableElements, diff_actionable_elements,
};

const MAX_ELEMENT_REFS: usize = 512;
const MAX_ELEMENT_DIGESTS: usize = 64;

#[derive(Default)]
struct ElementRefTable {
    next: usize,
    by_key: HashMap<String, usize>,
    entries: HashMap<usize, ComputerActionableElementRef>,
}

#[derive(Default)]
pub(super) struct ElementState {
    digests: Vec<(String, ComputerActionableElements)>,
    ref_tables: HashMap<String, ElementRefTable>,
}

pub(super) enum RefLookup {
    NoListing,
    Unknown,
    Found(ComputerActionableElementRef),
}

pub(super) fn digest_scope_key(
    thread_id: &str,
    window_id: Option<&str>,
    label_contains: Option<&str>,
) -> String {
    json!([thread_id, window_id, label_contains]).to_string()
}

impl ElementState {
    fn sync_element_refs(
        &mut self,
        thread_id: &str,
        elements: ComputerActionableElements,
    ) -> ComputerActionableElements {
        let table = self.ref_tables.entry(thread_id.to_owned()).or_default();
        if table.entries.len() + elements.items.len() > MAX_ELEMENT_REFS {
            table.by_key.clear();
            table.entries.clear();
        }
        let mut stable = elements;
        for (index, item) in stable.items.iter_mut().enumerate() {
            let id = &stable.ref_index[index];
            let identity = match computer_element_ref_identity(id) {
                Some(native) => json!(["native", native]),
                None => json!(["ordinal", id.ordinal]),
            };
            let key = json!([id.window_id, id.role, id.label, identity]).to_string();
            let element_ref = match table.by_key.get(&key) {
                Some(existing) => *existing,
                None => {
                    let minted = table.next;
                    table.next += 1;
                    table.by_key.insert(key, minted);
                    table.entries.insert(minted, id.clone());
                    minted
                }
            };
            item.element_ref = element_ref;
        }
        stable
    }

    pub(super) fn remember_digest(
        &mut self,
        thread_id: &str,
        key: &str,
        elements: ComputerActionableElements,
    ) -> ComputerActionableElements {
        let stable = self.sync_element_refs(thread_id, elements);
        self.digests.retain(|(existing, _)| existing != key);
        self.digests.push((key.to_owned(), stable.clone()));
        while self.digests.len() > MAX_ELEMENT_DIGESTS {
            self.digests.remove(0);
        }
        stable
    }

    pub(super) fn digest(&self, key: &str) -> Option<ComputerActionableElements> {
        self.digests
            .iter()
            .find(|(existing, _)| existing == key)
            .map(|(_, digest)| digest.clone())
    }

    pub(super) fn lookup_ref(&self, thread_id: &str, element_ref: u64) -> RefLookup {
        let Some(table) = self.ref_tables.get(thread_id) else {
            return RefLookup::NoListing;
        };
        match usize::try_from(element_ref)
            .ok()
            .and_then(|element_ref| table.entries.get(&element_ref))
        {
            Some(entry) => RefLookup::Found(entry.clone()),
            None => RefLookup::Unknown,
        }
    }
}

pub(super) fn element_json(item: &ComputerActionableElement, with_window: bool) -> Value {
    let mut entry = json!({
        "ref": item.element_ref,
        "role": item.role,
        "label": item.label,
    });
    if let Some(value) = &item.value {
        entry["value"] = json!(value);
    }
    if with_window && let Some(window_id) = &item.window_id {
        entry["windowId"] = json!(window_id);
    }
    entry
}

fn removed_json(item: &ComputerActionableElement, with_window: bool) -> Value {
    let mut entry = element_json(item, with_window);
    if let Value::Object(map) = &mut entry {
        map.remove("ref");
    }
    entry
}

fn change_json(change: &ComputerActionableElementChange, with_window: bool) -> Value {
    let mut entry = json!({
        "ref": change.element_ref,
        "role": change.role,
        "label": change.label,
    });
    if with_window && let Some(window_id) = &change.window_id {
        entry["windowId"] = json!(window_id);
    }
    if let Some(was) = &change.was {
        entry["was"] = json!(was);
    }
    if let Some(value) = &change.value {
        entry["value"] = json!(value);
    }
    entry
}

fn uniform_window_id<'a>(mut ids: impl Iterator<Item = Option<&'a String>>) -> Option<String> {
    let first = ids.next()??;
    ids.all(|id| id == Some(first)).then(|| first.clone())
}

pub(super) struct HoistedElements {
    pub items: Vec<Value>,
    pub element_window_id: Option<String>,
}

pub(super) fn hoist_element_window_id(items: &[ComputerActionableElement]) -> HoistedElements {
    let element_window_id = uniform_window_id(items.iter().map(|item| item.window_id.as_ref()));
    HoistedElements {
        items: items
            .iter()
            .map(|item| element_json(item, element_window_id.is_none()))
            .collect(),
        element_window_id,
    }
}

pub(super) fn listing_fields(stable: &ComputerActionableElements) -> Map<String, Value> {
    let wire = hoist_element_window_id(&stable.items);
    let mut fields = Map::new();
    fields.insert("elements".into(), Value::Array(wire.items));
    if let Some(window_id) = wire.element_window_id {
        fields.insert("elementWindowId".into(), json!(window_id));
    }
    if stable.source_incomplete {
        fields.insert("elementsSourceIncomplete".into(), json!(true));
    }
    if !stable.complete {
        fields.insert("elementsTruncated".into(), json!(true));
        fields.insert("elementsOmitted".into(), json!(stable.omitted));
    }
    fields
}

pub(super) fn element_change_fields(
    before: Option<&ComputerActionableElements>,
    stable: &ComputerActionableElements,
    limit: Option<usize>,
) -> Map<String, Value> {
    let empty = Vec::new();
    let changes = diff_actionable_elements(before.map_or(&empty, |before| &before.items), &stable.items);
    let mut remaining = limit.unwrap_or(usize::MAX);
    let mut take = |count: usize| {
        let selected = count.min(remaining);
        remaining -= selected;
        selected
    };
    let added = take(changes.added.len());
    let removed = take(changes.removed.len());
    let changed = take(changes.changed.len());
    let omitted = changes.added.len() + changes.removed.len() + changes.changed.len()
        - added
        - removed
        - changed;
    let element_window_id = uniform_window_id(
        changes
            .added
            .iter()
            .map(|item| item.window_id.as_ref())
            .chain(changes.changed.iter().map(|item| item.window_id.as_ref()))
            .chain(changes.removed.iter().map(|item| item.window_id.as_ref())),
    );
    let per_entry = element_window_id.is_none();
    let mut fields = Map::new();
    fields.insert(
        "elementChanges".into(),
        json!({
            "added": changes.added[..added].iter().map(|item| element_json(item, per_entry)).collect::<Vec<_>>(),
            "removed": changes.removed[..removed].iter().map(|item| removed_json(item, per_entry)).collect::<Vec<_>>(),
            "changed": changes.changed[..changed].iter().map(|item| change_json(item, per_entry)).collect::<Vec<_>>(),
        }),
    );
    if omitted > 0 {
        fields.insert("elementChangesOmitted".into(), json!(omitted));
    }
    if let Some(window_id) = element_window_id {
        fields.insert("elementWindowId".into(), json!(window_id));
    }
    if before.is_some_and(|before| !before.complete) || !stable.complete {
        fields.insert("elementChangesIncomplete".into(), json!(true));
    }
    fields
}
