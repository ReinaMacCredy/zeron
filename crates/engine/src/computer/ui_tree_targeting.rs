use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use zeron_proto::computer::{
    ComputerPoint, ComputerRect, ComputerScreenSize, ComputerSpaceErrorCode, ComputerTarget,
    ComputerUiNode,
};

use super::element_identity::{ObservedComputerElement, retain_computer_element};
use super::utf8_truncation::clamp_text_to_length;
use super::wait_for_control::ComputerTargetLookup;

const MAX_REPORTED_CANDIDATES: usize = 16;
const ELEMENT_DIGEST_MAX_LENGTH: usize = 60;
const ELEMENT_TEXT_MAX_LENGTH: usize = 80;
const ELEMENT_VALUE_MAX_LENGTH: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputerTargetErrorCode {
    Space(ComputerSpaceErrorCode),
    Invalid,
    NotFound,
    Ambiguous,
    Offscreen,
    Occluded,
    Refused,
    DenylistRefused,
}

impl ComputerTargetErrorCode {
    pub fn as_str(self) -> &'static str {
        use ComputerSpaceErrorCode as S;
        match self {
            Self::Invalid => "computer_target_invalid",
            Self::NotFound => "computer_target_not_found",
            Self::Ambiguous => "computer_target_ambiguous",
            Self::Offscreen => "computer_target_offscreen",
            Self::Occluded => "computer_target_occluded",
            Self::Refused => "computer_target_refused",
            Self::DenylistRefused => "computer_denylist_refused",
            Self::Space(code) => match code {
                S::ComputerSpacesUnavailable => "computer_spaces_unavailable",
                S::ComputerSpaceNotFound => "computer_space_not_found",
                S::ComputerSpaceCurrent => "computer_space_current",
                S::ComputerSpaceIdentityUnproven => "computer_space_identity_unproven",
                S::ComputerSpaceNotDesignated => "computer_space_not_designated",
                S::ComputerSpaceReserved => "computer_space_reserved",
                S::ComputerSpaceReservationRequired => "computer_space_reservation_required",
                S::ComputerSpaceReservationChanged => "computer_space_reservation_changed",
                S::ComputerSpaceReservationLimit => "computer_space_reservation_limit",
                S::ComputerSpaceWindowNotSelected => "computer_space_window_not_selected",
                S::ComputerSpaceTargetOutsideReservation => {
                    "computer_space_target_outside_reservation"
                }
                S::ComputerSpaceTargetMembershipUnproven => {
                    "computer_space_target_membership_unproven"
                }
                S::ComputerSpaceOperationUnsupported => "computer_space_operation_unsupported",
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputerTargetCandidate {
    pub label: String,
    pub role: String,
    pub window_id: Option<String>,
    pub on_screen: bool,
    pub frame: ComputerRect,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputerTargetError {
    pub code: ComputerTargetErrorCode,
    pub message: String,
    pub candidates: Vec<ComputerTargetCandidate>,
    pub not_found: bool,
    pub unresolved_text_control: bool,
}

impl ComputerTargetError {
    pub fn new(
        code: ComputerTargetErrorCode,
        message: impl Into<String>,
        candidates: Vec<ComputerTargetCandidate>,
    ) -> Self {
        Self {
            code,
            message: message_with_candidates(message.into(), &candidates),
            candidates,
            not_found: code == ComputerTargetErrorCode::NotFound,
            unresolved_text_control: false,
        }
    }

    pub fn space(code: ComputerSpaceErrorCode, message: impl Into<String>) -> Self {
        Self::new(ComputerTargetErrorCode::Space(code), message, Vec::new())
    }

    fn unresolved_text_control(mut self) -> Self {
        self.unresolved_text_control = true;
        self
    }
}

impl fmt::Display for ComputerTargetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ComputerTargetError {}

#[derive(Debug, Clone, Copy)]
pub struct ComputerTargetMatch<'a> {
    pub point: ComputerPoint,
    pub node: &'a ComputerUiNode,
}

fn quoted(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

fn flatten_ui_tree(root: &ComputerUiNode) -> Vec<&ComputerUiNode> {
    fn visit<'a>(node: &'a ComputerUiNode, nodes: &mut Vec<&'a ComputerUiNode>) {
        nodes.push(node);
        for child in &node.children {
            visit(child, nodes);
        }
    }
    let mut nodes = Vec::new();
    visit(root, &mut nodes);
    nodes
}

pub fn resolve_computer_point(
    target: &ComputerTarget,
    screen_size: &ComputerScreenSize,
) -> Result<ComputerPoint, ComputerTargetError> {
    let (x, y) = match (target.x, target.y) {
        (Some(x), Some(y)) => (x, y),
        (None, None) => {
            return Err(ComputerTargetError::new(
                ComputerTargetErrorCode::Invalid,
                "Computer actions require x/y coordinates or a labelled target.",
                Vec::new(),
            ));
        }
        _ => {
            return Err(ComputerTargetError::new(
                ComputerTargetErrorCode::Invalid,
                "Computer coordinate targets must include both x and y.",
                Vec::new(),
            ));
        }
    };
    let (width, height) = (f64::from(screen_size.width), f64::from(screen_size.height));
    if x < 0.0 || y < 0.0 || x >= width || y >= height {
        return Err(ComputerTargetError::new(
            ComputerTargetErrorCode::Offscreen,
            format!(
                "Computer target ({x}, {y}) is outside the {}x{} screen.",
                screen_size.width, screen_size.height
            ),
            Vec::new(),
        ));
    }
    Ok(ComputerPoint { x, y })
}

fn offscreen(target: &ComputerTarget, node: &ComputerUiNode) -> ComputerTargetError {
    ComputerTargetError::new(
        ComputerTargetErrorCode::Offscreen,
        format!(
            "Computer target {} is off-screen; refusing to guess a click.",
            describe_target(target)
        ),
        candidate_descriptions(&[node]),
    )
}

fn no_match(target: &ComputerTarget, pool: &[&ComputerUiNode]) -> ComputerTargetError {
    ComputerTargetError::new(
        ComputerTargetErrorCode::NotFound,
        format!(
            "No visible computer target matched {}.",
            describe_target(target)
        ),
        candidate_descriptions(pool),
    )
}

fn match_key(label: &str) -> String {
    normalize_label_spaces(label).to_lowercase()
}

fn target_pool<'a>(root: &'a ComputerUiNode, target: &ComputerTarget) -> Vec<&'a ComputerUiNode> {
    flatten_ui_tree(root)
        .into_iter()
        .filter(|node| {
            target
                .window_id
                .as_ref()
                .is_none_or(|window_id| node.window_id.as_ref() == Some(window_id))
        })
        .collect()
}

pub fn resolve_computer_semantic_target<'a>(
    root: &'a ComputerUiNode,
    target: &ComputerTarget,
    allow_offscreen: bool,
) -> Result<ComputerTargetMatch<'a>, ComputerTargetError> {
    if let (Some(ordinal), Some(label)) = (target.ref_ordinal, target.label.as_deref()) {
        return resolve_computer_ordinal_target(root, target, label, ordinal, allow_offscreen);
    }
    let pool = target_pool(root, target);
    let by_role: Vec<&ComputerUiNode> = match target.role.as_deref() {
        Some(role) => pool.iter().copied().filter(|n| n.role == role).collect(),
        None => pool.clone(),
    };
    let matches = match target.label.as_deref() {
        None => by_role,
        Some(label) => {
            let exact = normalize_label_spaces(label);
            let exact_matches: Vec<&ComputerUiNode> = by_role
                .iter()
                .copied()
                .filter(|n| normalize_label_spaces(matchable_label(n)) == exact)
                .collect();
            if exact_matches.is_empty() {
                let wanted = match_key(label);
                by_role
                    .into_iter()
                    .filter(|n| match_key(matchable_label(n)).contains(&wanted))
                    .collect()
            } else {
                exact_matches
            }
        }
    };
    let matches = if target.role.is_none() && matches.len() > 1 {
        let controls: Vec<&ComputerUiNode> = matches
            .iter()
            .copied()
            .filter(|node| !STATIC_TEXT_ROLES.contains(&node.role.as_str()))
            .collect();
        if controls.is_empty() {
            matches
        } else {
            controls
        }
    } else {
        matches
    };
    let node = match matches.as_slice() {
        [] => return Err(no_match(target, &pool)),
        [node] => *node,
        _ => {
            return Err(ComputerTargetError::new(
                ComputerTargetErrorCode::Ambiguous,
                format!(
                    "Computer target {} matched more than one control.",
                    describe_target(target)
                ),
                candidate_descriptions(&matches),
            ));
        }
    };
    if !node.on_screen && !allow_offscreen {
        return Err(offscreen(target, node));
    }
    Ok(ComputerTargetMatch {
        node,
        point: activation_point_for_node(node),
    })
}

fn resolve_computer_ordinal_target<'a>(
    root: &'a ComputerUiNode,
    target: &ComputerTarget,
    label: &str,
    ordinal: u64,
    allow_offscreen: bool,
) -> Result<ComputerTargetMatch<'a>, ComputerTargetError> {
    let pool = target_pool(root, target);
    let exact = normalize_label_spaces(label);
    let matches: Vec<&ComputerUiNode> = pool
        .iter()
        .copied()
        .filter(|node| {
            target.role.as_deref().is_none_or(|role| node.role == role)
                && normalize_label_spaces(matchable_label(node)) == exact
        })
        .collect();
    let on_screen: Vec<&ComputerUiNode> = matches.iter().copied().filter(|n| n.on_screen).collect();
    let ordered = if on_screen.is_empty() {
        matches
    } else {
        on_screen
    };
    let node = match usize::try_from(ordinal)
        .ok()
        .and_then(|ordinal| ordered.get(ordinal))
    {
        Some(node) => *node,
        None if ordered.is_empty() => return Err(no_match(target, &pool)),
        None => {
            return Err(ComputerTargetError::new(
                ComputerTargetErrorCode::NotFound,
                format!(
                    "Computer target {} no longer exists at its observed duplicate position. Observe again with computer_get_state before acting.",
                    describe_target(target)
                ),
                candidate_descriptions(&ordered),
            ));
        }
    };
    if !node.on_screen && !allow_offscreen {
        return Err(offscreen(target, node));
    }
    Ok(ComputerTargetMatch {
        node,
        point: activation_point_for_node(node),
    })
}

// A form label's static text carries the same name as the control it labels.
const STATIC_TEXT_ROLES: &[&str] = &["AXStaticText", "label", "static"];

const SEMANTIC_TEXT_ROLES: &[&str] = &[
    "AXTextField",
    "AXTextArea",
    "AXSearchField",
    "AXSecureTextField",
    "entry",
    "text field",
    "text-field",
    "search field",
    "text area",
    "textarea",
];

pub fn is_writable_text_node(node: &ComputerUiNode) -> bool {
    node.editable != Some(false)
        && (node.editable == Some(true) || SEMANTIC_TEXT_ROLES.contains(&node.role.as_str()))
}

pub fn resolve_computer_unique_text_target<'a>(
    root: &'a ComputerUiNode,
    window_id: &str,
    allow_offscreen: bool,
) -> Result<ComputerTargetMatch<'a>, ComputerTargetError> {
    let in_window = |node: &&ComputerUiNode| node.window_id.as_deref() == Some(window_id);
    let candidates: Vec<&ComputerUiNode> = flatten_ui_tree(root)
        .into_iter()
        .filter(|node| {
            in_window(node) && (allow_offscreen || node.on_screen) && is_writable_text_node(node)
        })
        .collect();
    let visible = if allow_offscreen { "" } else { "visible " };
    // A browser's address bar is always writable, so being unique never picks the page's field.
    if flatten_ui_tree(root)
        .into_iter()
        .any(|node| in_window(&node) && node.role == "AXWebArea")
    {
        return Err(ComputerTargetError::new(
            ComputerTargetErrorCode::NotFound,
            format!(
                "Window {} shows a web page; pass the page field's element index.",
                quoted(window_id)
            ),
            candidate_descriptions(&candidates),
        )
        .unresolved_text_control());
    }
    match candidates.as_slice() {
        [] => {
            let in_window_nodes: Vec<&ComputerUiNode> =
                flatten_ui_tree(root).into_iter().filter(in_window).collect();
            Err(ComputerTargetError::new(
                ComputerTargetErrorCode::NotFound,
                format!(
                    "Window {} has no {visible}writable text control. Observe it and pass the exact label and role.",
                    quoted(window_id)
                ),
                candidate_descriptions(&in_window_nodes),
            )
            .unresolved_text_control())
        }
        [node] => Ok(ComputerTargetMatch {
            node,
            point: activation_point_for_node(node),
        }),
        _ => Err(ComputerTargetError::new(
            ComputerTargetErrorCode::Ambiguous,
            format!(
                "Window {} has more than one {visible}writable text control. Pass the exact label and role.",
                quoted(window_id)
            ),
            candidate_descriptions(&candidates),
        )
        .unresolved_text_control()),
    }
}

pub fn resolve_computer_window_target<'a>(
    root: &'a ComputerUiNode,
    window_id: &str,
) -> Result<Option<ComputerTargetMatch<'a>>, ComputerTargetError> {
    let Some(node) = flatten_ui_tree(root)
        .into_iter()
        .find(|candidate| candidate.window_id.as_deref() == Some(window_id))
    else {
        return Ok(None);
    };
    if !node.on_screen {
        return Err(ComputerTargetError::new(
            ComputerTargetErrorCode::Offscreen,
            format!(
                "Computer window {} is off-screen; refusing to guess a scroll point.",
                quoted(window_id)
            ),
            candidate_descriptions(&[node]),
        ));
    }
    Ok(Some(ComputerTargetMatch {
        node,
        point: activation_point_for_node(node),
    }))
}

pub fn computer_target_lookup<E: From<ComputerTargetError>>(
    root: &ComputerUiNode,
    target: &ComputerTarget,
) -> Result<(), ComputerTargetLookup<E>> {
    match resolve_computer_semantic_target(root, target, false) {
        Ok(_) => Ok(()),
        Err(error) if error.code == ComputerTargetErrorCode::Ambiguous => {
            Err(ComputerTargetLookup::Ambiguous)
        }
        Err(error) if error.code == ComputerTargetErrorCode::NotFound => {
            Err(ComputerTargetLookup::NotFound)
        }
        Err(error) => Err(ComputerTargetLookup::Failed(error.into())),
    }
}

pub fn activation_point_for_node(node: &ComputerUiNode) -> ComputerPoint {
    node.activation_point.unwrap_or(ComputerPoint {
        x: node.frame.x + node.frame.width / 2.0,
        y: node.frame.y + node.frame.height / 2.0,
    })
}

pub fn candidate_descriptions(nodes: &[&ComputerUiNode]) -> Vec<ComputerTargetCandidate> {
    nodes
        .iter()
        .take(MAX_REPORTED_CANDIDATES)
        .map(|node| ComputerTargetCandidate {
            label: node
                .label
                .clone()
                .or_else(|| node.description.clone())
                .unwrap_or_else(|| "(unlabelled)".to_owned()),
            role: node.role.clone(),
            window_id: node.window_id.clone(),
            on_screen: node.on_screen,
            frame: node.frame,
        })
        .collect()
}

pub fn computer_target_candidates(root: &ComputerUiNode) -> Vec<ComputerTargetCandidate> {
    candidate_descriptions(&flatten_ui_tree(root))
}

const ACTIONABLE_ROLES: &[&str] = &[
    "AXButton",
    "AXCheckBox",
    "AXRadioButton",
    "AXPopUpButton",
    "AXComboBox",
    "AXTextField",
    "AXTextArea",
    "AXSearchField",
    "AXSecureTextField",
    "AXLink",
    "AXMenuBarItem",
    "AXMenuItem",
    "AXSlider",
    "AXIncrementor",
    "AXTab",
    "AXSwitch",
    "push button",
    "button",
    "toggle button",
    "entry",
    "text field",
    "text-field",
    "search field",
    "check box",
    "radio button",
    "combo box",
    "list box",
    "switch",
    "slider",
    "spin button",
    "link",
    "page tab",
    "menu item",
    "check menu item",
    "radio menu item",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerActionableElement {
    pub element_ref: usize,
    pub role: String,
    pub label: String,
    pub value: Option<String>,
    pub window_id: Option<String>,
    pub(crate) unclamped_value: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputerActionableElementRef {
    pub label: String,
    pub role: String,
    pub window_id: Option<String>,
    pub ordinal: usize,
    pub(crate) observed: Option<Arc<ObservedComputerElement>>,
    pub(crate) path: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputerActionableElements {
    pub items: Vec<ComputerActionableElement>,
    pub ref_index: Vec<ComputerActionableElementRef>,
    pub complete: bool,
    pub source_incomplete: bool,
    pub omitted: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComputerActionableElementFilter {
    pub window_id: Option<String>,
    pub label_contains: Option<String>,
    pub any_role: bool,
}

pub fn actionable_elements(
    root: &ComputerUiNode,
    filter: &ComputerActionableElementFilter,
    native_identity: impl Fn(&ComputerUiNode) -> Option<String>,
) -> ComputerActionableElements {
    actionable_elements_limited(root, filter, native_identity, ELEMENT_DIGEST_MAX_LENGTH)
}

pub fn actionable_elements_limited(
    root: &ComputerUiNode,
    filter: &ComputerActionableElementFilter,
    native_identity: impl Fn(&ComputerUiNode) -> Option<String>,
    limit: usize,
) -> ComputerActionableElements {
    let wanted = filter.label_contains.as_deref().map(match_key);
    let mut digest = ComputerActionableElements {
        items: Vec::new(),
        ref_index: Vec::new(),
        complete: true,
        source_incomplete: false,
        omitted: 0,
    };
    let mut ordinals: HashMap<(String, String, String), usize> = HashMap::new();
    let mut stack = vec![(root, Vec::new())];
    while let Some((node, path)) = stack.pop() {
        if node.truncated == Some(true) {
            digest.source_incomplete = true;
        }
        let label = matchable_label(node);
        let collectible = (filter.any_role || ACTIONABLE_ROLES.contains(&node.role.as_str()))
            && node.on_screen
            && node.window_id.is_some()
            && !label.is_empty()
            && filter
                .window_id
                .as_ref()
                .is_none_or(|window_id| node.window_id.as_ref() == Some(window_id))
            && wanted
                .as_ref()
                .is_none_or(|wanted| match_key(label).contains(wanted.as_str()));
        if collectible {
            let identity = (
                node.window_id.clone().unwrap_or_default(),
                node.role.clone(),
                normalize_label_spaces(label),
            );
            let ordinal = ordinals.entry(identity).or_insert(0);
            let current = *ordinal;
            *ordinal += 1;
            if digest.items.len() < limit {
                digest.ref_index.push(ComputerActionableElementRef {
                    label: label.to_owned(),
                    role: node.role.clone(),
                    window_id: node.window_id.clone(),
                    ordinal: current,
                    observed: retain_computer_element(node, native_identity(node)),
                    path: path.clone(),
                });
                digest.items.push(ComputerActionableElement {
                    element_ref: digest.items.len(),
                    role: node.role.clone(),
                    label: clamp_text_to_length(label, ELEMENT_TEXT_MAX_LENGTH),
                    value: node
                        .value
                        .as_deref()
                        .map(|value| clamp_text_to_length(value, ELEMENT_VALUE_MAX_LENGTH)),
                    window_id: node.window_id.clone(),
                    unclamped_value: node.value.clone(),
                });
            } else {
                digest.omitted += 1;
            }
        }
        stack.extend(
            node.children
                .iter()
                .enumerate()
                .rev()
                .map(|(index, child)| {
                    let mut child_path = path.clone();
                    child_path.push(index as u32);
                    (child, child_path)
                }),
        );
    }
    digest.complete = digest.omitted == 0 && !digest.source_incomplete;
    digest
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComputerActionableElementChange {
    pub element_ref: usize,
    pub role: String,
    pub label: String,
    pub window_id: Option<String>,
    pub was: Option<String>,
    pub value: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ComputerActionableElementsDiff {
    pub added: Vec<ComputerActionableElement>,
    pub removed: Vec<ComputerActionableElement>,
    pub changed: Vec<ComputerActionableElementChange>,
}

type ElementIdentity<'a> = (Option<&'a str>, &'a str, &'a str);

fn group_elements(
    items: &[ComputerActionableElement],
) -> Vec<(ElementIdentity<'_>, Vec<&ComputerActionableElement>)> {
    let mut groups: Vec<(ElementIdentity<'_>, Vec<&ComputerActionableElement>)> = Vec::new();
    let mut index: HashMap<ElementIdentity<'_>, usize> = HashMap::new();
    for item in items {
        let key = (
            item.window_id.as_deref(),
            item.role.as_str(),
            item.label.as_str(),
        );
        match index.get(&key) {
            Some(&at) => groups[at].1.push(item),
            None => {
                index.insert(key, groups.len());
                groups.push((key, vec![item]));
            }
        }
    }
    groups
}

pub fn diff_actionable_elements(
    before: &[ComputerActionableElement],
    after: &[ComputerActionableElement],
) -> ComputerActionableElementsDiff {
    let mut new_groups: Vec<Option<(ElementIdentity<'_>, Vec<&ComputerActionableElement>)>> =
        group_elements(after).into_iter().map(Some).collect();
    let mut diff = ComputerActionableElementsDiff::default();
    for (key, previous) in group_elements(before) {
        let Some(current) = new_groups
            .iter_mut()
            .find(|group| group.as_ref().is_some_and(|(k, _)| *k == key))
            .and_then(Option::take)
            .map(|(_, items)| items)
        else {
            diff.removed.extend(previous.into_iter().cloned());
            continue;
        };
        let overlap = previous.len().min(current.len());
        for (old, item) in previous.iter().zip(&current) {
            if old.unclamped_value != item.unclamped_value {
                diff.changed.push(ComputerActionableElementChange {
                    element_ref: item.element_ref,
                    role: item.role.clone(),
                    label: item.label.clone(),
                    window_id: item.window_id.clone(),
                    was: old.value.clone(),
                    value: item.value.clone(),
                });
            }
        }
        diff.removed
            .extend(previous[overlap..].iter().map(|item| (*item).clone()));
        diff.added
            .extend(current[overlap..].iter().map(|item| (*item).clone()));
    }
    for (_, current) in new_groups.into_iter().flatten() {
        diff.added.extend(current.into_iter().cloned());
    }
    diff
}

pub fn describe_target(target: &ComputerTarget) -> String {
    let parts: Vec<String> = [
        target
            .label
            .as_deref()
            .filter(|label| !label.is_empty())
            .map(|label| format!("label={}", quoted(label))),
        target
            .role
            .as_deref()
            .filter(|role| !role.is_empty())
            .map(|role| format!("role={}", quoted(role))),
        target
            .window_id
            .as_deref()
            .filter(|window_id| !window_id.is_empty())
            .map(|window_id| format!("window={}", quoted(window_id))),
        target
            .ref_ordinal
            .map(|ordinal| format!("duplicate {}", ordinal + 1)),
    ]
    .into_iter()
    .flatten()
    .collect();
    if parts.is_empty() {
        "the supplied coordinates".to_owned()
    } else {
        parts.join(", ")
    }
}

fn matchable_label(node: &ComputerUiNode) -> &str {
    let label = node
        .label
        .as_deref()
        .or(node.description.as_deref())
        .unwrap_or_default();
    if !label.is_empty() {
        return label;
    }
    match node.role.as_str() {
        "AXTextArea" => "text entry area",
        "AXTextField" => "text field",
        "AXSearchField" => "search field",
        "AXComboBox" => "combo box",
        "AXPopUpButton" => "pop up button",
        _ => "",
    }
}

pub fn normalize_label_spaces(label: &str) -> String {
    label.replace(['\u{a0}', '\u{2007}', '\u{202f}'], " ")
}

fn describe_candidate(candidate: &ComputerTargetCandidate) -> String {
    let window = candidate
        .window_id
        .as_deref()
        .map(|window_id| format!(" in window {}", quoted(window_id)))
        .unwrap_or_default();
    format!("{} {}{window}", candidate.role, quoted(&candidate.label))
}

fn message_with_candidates(message: String, candidates: &[ComputerTargetCandidate]) -> String {
    if candidates.is_empty() {
        return message;
    }
    let listed: Vec<String> = candidates.iter().map(describe_candidate).collect();
    format!(
        "{message} Controls in the accessibility tree: {}.",
        listed.join("; ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Node<'a> {
        role: &'a str,
        label: Option<&'a str>,
        value: Option<&'a str>,
        description: Option<&'a str>,
        frame: Option<(f64, f64, f64, f64)>,
        activation_point: Option<(f64, f64)>,
        on_screen: Option<bool>,
        window_id: Option<&'a str>,
        editable: Option<bool>,
        truncated: Option<bool>,
        children: Vec<ComputerUiNode>,
    }

    fn node(partial: Node<'_>) -> ComputerUiNode {
        let (x, y, width, height) = partial.frame.unwrap_or((0.0, 0.0, 1_920.0, 1_080.0));
        ComputerUiNode {
            role: partial.role.to_owned(),
            label: partial.label.map(str::to_owned),
            value: partial.value.map(str::to_owned),
            description: partial.description.map(str::to_owned),
            frame: ComputerRect {
                x,
                y,
                width,
                height,
            },
            activation_point: partial
                .activation_point
                .map(|(x, y)| ComputerPoint { x, y }),
            on_screen: partial.on_screen.unwrap_or(true),
            window_id: partial.window_id.map(str::to_owned),
            accessibility_root: None,
            node_path: None,
            editable: partial.editable,
            truncated: partial.truncated,
            children: partial.children,
        }
    }

    fn desktop() -> ComputerUiNode {
        node(Node {
            role: "desktop",
            description: Some("AT-SPI desktop"),
            children: vec![
                node(Node {
                    role: "push button",
                    label: Some("Save"),
                    window_id: Some("editor"),
                    frame: Some((100.0, 200.0, 80.0, 30.0)),
                    activation_point: Some((140.0, 215.0)),
                    ..Node::default()
                }),
                node(Node {
                    role: "push button",
                    label: Some("Save As"),
                    window_id: Some("editor"),
                    frame: Some((200.0, 200.0, 80.0, 30.0)),
                    ..Node::default()
                }),
                node(Node {
                    role: "menu item",
                    label: Some("Save"),
                    window_id: Some("browser"),
                    frame: Some((900.0, 200.0, 80.0, 30.0)),
                    ..Node::default()
                }),
            ],
            ..Node::default()
        })
    }

    fn target(label: Option<&str>, role: Option<&str>, window_id: Option<&str>) -> ComputerTarget {
        ComputerTarget {
            label: label.map(str::to_owned),
            role: role.map(str::to_owned),
            window_id: window_id.map(str::to_owned),
            ..ComputerTarget::default()
        }
    }

    fn screen(width: u32, height: u32) -> ComputerScreenSize {
        ComputerScreenSize {
            width,
            height,
            scale: Some(1.0),
        }
    }

    fn point_target(x: Option<f64>, y: Option<f64>) -> ComputerTarget {
        ComputerTarget {
            x,
            y,
            ..ComputerTarget::default()
        }
    }

    fn semantic_error(root: &ComputerUiNode, target: &ComputerTarget) -> ComputerTargetError {
        resolve_computer_semantic_target(root, target, false).unwrap_err()
    }

    #[test]
    fn takes_a_point_that_is_on_the_screen() {
        assert_eq!(
            resolve_computer_point(&point_target(Some(10.0), Some(20.0)), &screen(1_920, 1_080)),
            Ok(ComputerPoint { x: 10.0, y: 20.0 })
        );
    }

    #[test]
    fn refuses_half_a_coordinate_and_a_coordinate_past_the_edge() {
        let half = resolve_computer_point(&point_target(Some(10.0), None), &screen(1_920, 1_080));
        assert_eq!(half.unwrap_err().code, ComputerTargetErrorCode::Invalid);
        let past = resolve_computer_point(
            &point_target(Some(1_920.0), Some(10.0)),
            &screen(1_920, 1_080),
        );
        assert_eq!(past.unwrap_err().code, ComputerTargetErrorCode::Offscreen);
    }

    #[test]
    fn returns_the_controls_own_activation_point_when_it_has_one() {
        let root = desktop();
        let found = resolve_computer_semantic_target(
            &root,
            &target(Some("Save"), Some("push button"), None),
            false,
        )
        .unwrap();
        assert_eq!(found.point, ComputerPoint { x: 140.0, y: 215.0 });
        assert_eq!(found.node.window_id.as_deref(), Some("editor"));
    }

    #[test]
    fn falls_back_to_the_frame_centre_for_a_control_with_no_activation_point() {
        let plain = node(Node {
            role: "x",
            frame: Some((10.0, 20.0, 100.0, 40.0)),
            ..Node::default()
        });
        assert_eq!(
            activation_point_for_node(&plain),
            ComputerPoint { x: 60.0, y: 40.0 }
        );
    }

    #[test]
    fn prefers_an_exact_label_over_a_longer_one_that_contains_it() {
        let root = desktop();
        let found = resolve_computer_semantic_target(
            &root,
            &target(Some("Save"), None, Some("editor")),
            false,
        )
        .unwrap();
        assert_eq!(found.node.frame.x, 100.0);
    }

    #[test]
    fn scopes_matching_to_the_named_window() {
        let root = desktop();
        let found = resolve_computer_semantic_target(
            &root,
            &target(Some("Save"), None, Some("browser")),
            false,
        )
        .unwrap();
        assert_eq!(found.node.role, "menu item");
    }

    #[test]
    fn falls_back_to_substring_when_no_label_matches_exactly() {
        let root = desktop();
        let found =
            resolve_computer_semantic_target(&root, &target(Some("ave As"), None, None), false)
                .unwrap();
        assert_eq!(found.node.label.as_deref(), Some("Save As"));
    }

    #[test]
    fn compares_a_role_verbatim_rather_than_loosely() {
        let error = semantic_error(&desktop(), &target(None, Some("Push Button"), None));
        assert_eq!(error.code, ComputerTargetErrorCode::NotFound);
    }

    #[test]
    fn matches_a_label_without_trimming_the_query() {
        let error = semantic_error(&desktop(), &target(Some(" Save "), None, None));
        assert_eq!(error.code, ComputerTargetErrorCode::NotFound);
    }

    #[test]
    fn matches_a_visible_form_label_containing_a_non_breaking_space() {
        for space in ["\u{a0}", "\u{2007}", "\u{202f}"] {
            let label = format!("First name{space}*");
            let root = node(Node {
                role: "desktop",
                children: vec![
                    node(Node {
                        role: "AXTextField",
                        label: Some(&label),
                        ..Node::default()
                    }),
                    node(Node {
                        role: "AXTextField",
                        label: Some("First name * (optional)"),
                        ..Node::default()
                    }),
                ],
                ..Node::default()
            });
            let found = resolve_computer_semantic_target(
                &root,
                &target(Some("First name *"), None, None),
                false,
            )
            .unwrap();
            assert!(std::ptr::eq(found.node, &root.children[0]), "{space:?}");
        }
    }

    #[test]
    fn refuses_ambiguity_between_labels_differing_only_in_non_breaking_spaces() {
        let root = node(Node {
            role: "desktop",
            children: vec![
                node(Node {
                    role: "AXTextField",
                    label: Some("First name\u{a0}*"),
                    ..Node::default()
                }),
                node(Node {
                    role: "AXTextField",
                    label: Some("First name *"),
                    ..Node::default()
                }),
            ],
            ..Node::default()
        });
        let error = semantic_error(&root, &target(Some("First name *"), None, None));
        assert_eq!(error.code, ComputerTargetErrorCode::Ambiguous);
        assert_eq!(error.candidates.len(), 2);
    }

    #[test]
    fn refuses_a_label_that_names_a_control_in_two_windows() {
        let error = semantic_error(&desktop(), &target(Some("Save"), None, None));
        assert_eq!(error.code, ComputerTargetErrorCode::Ambiguous);
        assert_eq!(error.candidates.len(), 2);
    }

    #[test]
    fn trusts_the_perception_sources_own_on_screen_flag() {
        let root = node(Node {
            role: "desktop",
            children: vec![node(Node {
                role: "push button",
                label: Some("Hidden"),
                on_screen: Some(false),
                frame: Some((10.0, 10.0, 80.0, 30.0)),
                ..Node::default()
            })],
            ..Node::default()
        });
        let hidden = target(Some("Hidden"), None, None);
        let error = semantic_error(&root, &hidden);
        assert_eq!(error.code, ComputerTargetErrorCode::Offscreen);
        assert_eq!(error.candidates.len(), 1);
        assert_eq!(
            resolve_computer_semantic_target(&root, &hidden, true)
                .unwrap()
                .node
                .label
                .as_deref(),
            Some("Hidden")
        );
    }

    #[test]
    fn selects_an_unlabelled_native_text_area_without_guessing_across_windows() {
        let root = node(Node {
            role: "desktop",
            children: vec![
                node(Node {
                    role: "AXTextArea",
                    window_id: Some("editor"),
                    frame: Some((20.0, 40.0, 300.0, 200.0)),
                    ..Node::default()
                }),
                node(Node {
                    role: "AXTextArea",
                    window_id: Some("other"),
                    ..Node::default()
                }),
                node(Node {
                    role: "AXButton",
                    label: Some("Save"),
                    window_id: Some("editor"),
                    ..Node::default()
                }),
            ],
            ..Node::default()
        });
        let found = resolve_computer_unique_text_target(&root, "editor", false).unwrap();
        assert!(std::ptr::eq(found.node, &root.children[0]));
    }

    #[test]
    fn never_resolves_the_address_bar_of_a_web_page_window_by_uniqueness() {
        let root = node(Node {
            role: "desktop",
            children: vec![
                node(Node {
                    role: "AXTextField",
                    label: Some("Address and search bar"),
                    window_id: Some("browser"),
                    ..Node::default()
                }),
                node(Node {
                    role: "AXWebArea",
                    label: Some("Results"),
                    window_id: Some("browser"),
                    ..Node::default()
                }),
            ],
            ..Node::default()
        });
        let error = resolve_computer_unique_text_target(&root, "browser", false).unwrap_err();
        assert!(error.unresolved_text_control);
    }

    #[test]
    fn refuses_multiple_writable_controls_and_names_each_candidate() {
        let root = node(Node {
            role: "desktop",
            children: vec![
                node(Node {
                    role: "AXTextField",
                    label: Some("Title"),
                    window_id: Some("editor"),
                    ..Node::default()
                }),
                node(Node {
                    role: "AXTextArea",
                    label: Some("Body"),
                    window_id: Some("editor"),
                    ..Node::default()
                }),
            ],
            ..Node::default()
        });
        let error = resolve_computer_unique_text_target(&root, "editor", false).unwrap_err();
        assert_eq!(error.code, ComputerTargetErrorCode::Ambiguous);
        assert!(error.unresolved_text_control);
        let labels: Vec<&str> = error.candidates.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["Title", "Body"]);
    }

    #[test]
    fn a_label_shared_with_its_static_caption_resolves_to_the_control() {
        let labelled = |role, label| {
            node(Node {
                role,
                label: Some(label),
                window_id: Some("page"),
                ..Node::default()
            })
        };
        let root = node(Node {
            role: "desktop",
            children: vec![
                labelled("AXStaticText", "Notes area"),
                labelled("AXTextArea", "Notes area"),
                labelled("AXStaticText", "Caption"),
                labelled("AXStaticText", "Caption"),
            ],
            ..Node::default()
        });
        let target = |label: &str| ComputerTarget {
            label: Some(label.into()),
            window_id: Some("page".into()),
            ..ComputerTarget::default()
        };
        let found = resolve_computer_semantic_target(&root, &target("Notes area"), true).unwrap();
        assert!(std::ptr::eq(found.node, &root.children[1]));
        let error = resolve_computer_semantic_target(&root, &target("Caption"), true).unwrap_err();
        assert_eq!(error.code, ComputerTargetErrorCode::Ambiguous);
    }

    #[test]
    fn does_not_select_a_text_shaped_control_explicitly_marked_read_only() {
        let root = node(Node {
            role: "desktop",
            children: vec![node(Node {
                role: "AXTextArea",
                label: Some("Transcript"),
                editable: Some(false),
                window_id: Some("editor"),
                ..Node::default()
            })],
            ..Node::default()
        });
        let error = resolve_computer_unique_text_target(&root, "editor", false).unwrap_err();
        assert_eq!(error.code, ComputerTargetErrorCode::NotFound);
    }

    #[test]
    fn can_retain_an_exact_writable_control_after_its_window_leaves_the_active_space() {
        let root = node(Node {
            role: "desktop",
            children: vec![node(Node {
                role: "AXTextArea",
                label: Some("Body"),
                window_id: Some("editor"),
                on_screen: Some(false),
                ..Node::default()
            })],
            ..Node::default()
        });
        let found = resolve_computer_unique_text_target(&root, "editor", true).unwrap();
        assert!(std::ptr::eq(found.node, &root.children[0]));
        assert!(resolve_computer_unique_text_target(&root, "editor", false).is_err());
    }

    #[test]
    fn puts_the_candidates_in_the_message_not_only_in_the_structured_field() {
        let error = semantic_error(&desktop(), &target(Some("Print"), None, None));
        assert_eq!(error.code, ComputerTargetErrorCode::NotFound);
        assert!(!error.candidates.is_empty());
        let message = error.to_string();
        assert!(message.contains("Save As"));
        assert!(message.contains("push button"));
        assert!(message.contains("in window \"editor\""));
    }

    #[test]
    fn leaves_a_message_alone_when_there_are_no_candidates_to_name() {
        let screen = ComputerScreenSize {
            width: 100,
            height: 100,
            scale: None,
        };
        let error =
            resolve_computer_point(&point_target(Some(5_000.0), Some(10.0)), &screen).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Computer target (5000, 10) is outside the 100x100 screen."
        );
    }

    #[test]
    fn caps_a_candidate_list_at_sixteen_entries() {
        let root = node(Node {
            role: "desktop",
            children: (0..40)
                .map(|index| {
                    node(Node {
                        role: "push button",
                        label: Some(&format!("Button {index}")),
                        ..Node::default()
                    })
                })
                .collect(),
            ..Node::default()
        });
        assert_eq!(computer_target_candidates(&root).len(), 16);
        let error = semantic_error(&root, &target(Some("Nope"), None, None));
        assert_eq!(error.candidates.len(), 16);
    }

    fn digest(root: &ComputerUiNode) -> ComputerActionableElements {
        actionable_elements(root, &ComputerActionableElementFilter::default(), |_| None)
    }

    fn element(
        element_ref: usize,
        role: &str,
        label: &str,
        value: Option<&str>,
        window_id: &str,
    ) -> ComputerActionableElement {
        ComputerActionableElement {
            element_ref,
            role: role.into(),
            label: label.into(),
            value: value.map(str::to_owned),
            window_id: Some(window_id.into()),
            unclamped_value: value.map(str::to_owned),
        }
    }

    fn element_ref(
        label: &str,
        role: &str,
        window_id: &str,
        ordinal: usize,
        path: &[u32],
    ) -> ComputerActionableElementRef {
        ComputerActionableElementRef {
            label: label.into(),
            role: role.into(),
            window_id: Some(window_id.into()),
            ordinal,
            observed: None,
            path: path.to_vec(),
        }
    }

    #[test]
    fn an_unlabelled_text_area_still_gets_a_ref_under_its_role_name() {
        let root = node(Node {
            role: "AXWindow",
            children: vec![node(Node {
                role: "AXTextArea",
                value: Some("alpha beta"),
                window_id: Some("editor"),
                ..Node::default()
            })],
            ..Node::default()
        });
        let listed = digest(&root);
        assert_eq!(
            listed.items,
            [element(
                0,
                "AXTextArea",
                "text entry area",
                Some("alpha beta"),
                "editor"
            )]
        );
    }

    #[test]
    fn finds_non_breaking_space_labels_through_the_same_query_used_for_targeting() {
        let root = node(Node {
            role: "desktop",
            children: vec![node(Node {
                role: "AXTextField",
                label: Some("First name\u{a0}*"),
                window_id: Some("browser"),
                ..Node::default()
            })],
            ..Node::default()
        });
        let result = actionable_elements(
            &root,
            &ComputerActionableElementFilter {
                window_id: Some("browser".into()),
                label_contains: Some("FIRST NAME *".into()),
                ..ComputerActionableElementFilter::default()
            },
            |_| None,
        );
        let labels: Vec<&str> = result.items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["First name\u{a0}*"]);
        assert!(result.complete);
    }

    #[test]
    fn lists_labeled_on_screen_actionable_elements_in_tree_order() {
        let root = node(Node {
            role: "desktop",
            children: vec![
                node(Node {
                    role: "push button",
                    label: Some("Reload"),
                    window_id: Some("browser"),
                    children: vec![node(Node {
                        role: "entry",
                        label: Some("Email"),
                        value: Some(""),
                        window_id: Some("browser"),
                        ..Node::default()
                    })],
                    ..Node::default()
                }),
                node(Node {
                    role: "heading",
                    label: Some("Settings"),
                    window_id: Some("browser"),
                    ..Node::default()
                }),
            ],
            ..Node::default()
        });
        assert_eq!(
            digest(&root),
            ComputerActionableElements {
                complete: true,
                source_incomplete: false,
                omitted: 0,
                items: vec![
                    element(0, "push button", "Reload", None, "browser"),
                    element(1, "entry", "Email", Some(""), "browser"),
                ],
                ref_index: vec![
                    element_ref("Reload", "push button", "browser", 0, &[0]),
                    element_ref("Email", "entry", "browser", 0, &[0, 0]),
                ],
            }
        );
    }

    #[test]
    fn drops_unlabeled_controls_static_text_and_off_screen_elements() {
        let root = node(Node {
            role: "desktop",
            children: vec![
                node(Node {
                    role: "push button",
                    window_id: Some("w"),
                    ..Node::default()
                }),
                node(Node {
                    role: "text",
                    label: Some("A paragraph of static text"),
                    window_id: Some("w"),
                    ..Node::default()
                }),
                node(Node {
                    role: "check box",
                    label: Some("Off screen"),
                    on_screen: Some(false),
                    window_id: Some("w"),
                    ..Node::default()
                }),
                node(Node {
                    role: "check box",
                    label: Some("Subscribed"),
                    window_id: Some("w"),
                    ..Node::default()
                }),
            ],
            ..Node::default()
        });
        assert_eq!(
            digest(&root).items,
            [element(0, "check box", "Subscribed", None, "w")]
        );
    }

    #[test]
    fn falls_back_to_the_description_when_there_is_no_label_matching_targeting() {
        let root = node(Node {
            role: "desktop",
            children: vec![node(Node {
                role: "slider",
                description: Some("Volume"),
                window_id: Some("player"),
                ..Node::default()
            })],
            ..Node::default()
        });
        assert_eq!(
            digest(&root).items,
            [element(0, "slider", "Volume", None, "player")]
        );
    }

    #[test]
    fn keeps_duplicate_labels_but_separates_windows() {
        let save = |role: &'static str, window: &'static str| {
            node(Node {
                role,
                label: Some("Save"),
                window_id: Some(window),
                ..Node::default()
            })
        };
        let root = node(Node {
            role: "desktop",
            children: vec![
                save("push button", "editor"),
                save("push button", "editor"),
                save("menu item", "browser"),
            ],
            ..Node::default()
        });
        let elements = digest(&root);
        assert!(elements.complete);
        assert_eq!(elements.items.len(), 3);
        assert_eq!(elements.items[0].window_id.as_deref(), Some("editor"));
        assert_eq!(elements.items[2].window_id.as_deref(), Some("browser"));
    }

    #[test]
    fn truncates_long_labels_and_values_on_whole_characters() {
        let label = "x".repeat(300);
        let value = "v".repeat(200);
        let root = node(Node {
            role: "desktop",
            children: vec![node(Node {
                role: "entry",
                label: Some(&label),
                value: Some(&value),
                window_id: Some("editor"),
                ..Node::default()
            })],
            ..Node::default()
        });
        let elements = digest(&root);
        assert!(elements.items[0].label.chars().count() <= 80);
        assert!(elements.items[0].value.as_ref().unwrap().chars().count() <= 40);
    }

    #[test]
    fn caps_the_list_and_reports_when_it_had_to_cut_elements_off() {
        let root = node(Node {
            role: "desktop",
            children: (0..80)
                .map(|index| {
                    node(Node {
                        role: "push button",
                        label: Some(&format!("Button {index}")),
                        window_id: Some("panel"),
                        children: vec![node(Node {
                            role: "link",
                            label: Some("child link"),
                            window_id: Some("panel"),
                            ..Node::default()
                        })],
                        ..Node::default()
                    })
                })
                .collect(),
            ..Node::default()
        });
        let elements = digest(&root);
        assert_eq!(elements.items.len(), 60);
        assert!(!elements.complete);
        assert_eq!(elements.omitted, 100);
    }

    #[test]
    fn includes_native_macos_controls_and_reports_a_partial_accessibility_source() {
        let root = node(Node {
            role: "desktop",
            truncated: Some(true),
            children: vec![
                node(Node {
                    role: "AXButton",
                    label: Some("Save"),
                    window_id: Some("native"),
                    ..Node::default()
                }),
                node(Node {
                    role: "AXMenuBarItem",
                    label: Some("File"),
                    window_id: Some("native"),
                    ..Node::default()
                }),
            ],
            ..Node::default()
        });
        let result = digest(&root);
        let labels: Vec<&str> = result.items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["Save", "File"]);
        assert!(!result.complete);
        assert!(result.source_incomplete);
    }

    fn button(label: &str, value: Option<&str>) -> ComputerActionableElement {
        element(0, "push button", label, value, "editor")
    }

    fn in_window(label: &str, window_id: &str) -> ComputerActionableElement {
        element(0, "push button", label, None, window_id)
    }

    #[test]
    fn reports_every_element_as_added_against_an_empty_baseline() {
        let after = vec![button("Save", None), button("Cancel", None)];
        assert_eq!(
            diff_actionable_elements(&[], &after),
            ComputerActionableElementsDiff {
                added: after.clone(),
                ..Default::default()
            }
        );
    }

    #[test]
    fn reports_an_identical_digest_as_no_change() {
        let items = vec![button("Save", None), button("Cancel", Some("armed"))];
        assert_eq!(
            diff_actionable_elements(&items, &items.clone()),
            ComputerActionableElementsDiff::default()
        );
    }

    #[test]
    fn separates_added_removed_and_value_changed_entries() {
        let before = [
            button("Save", None),
            button("Display", Some("0")),
            button("Help", None),
        ];
        let after = [
            button("Save", None),
            button("Display", Some("42")),
            button("About", None),
        ];
        assert_eq!(
            diff_actionable_elements(&before, &after),
            ComputerActionableElementsDiff {
                added: vec![button("About", None)],
                removed: vec![button("Help", None)],
                changed: vec![ComputerActionableElementChange {
                    element_ref: 0,
                    role: "push button".into(),
                    label: "Display".into(),
                    window_id: Some("editor".into()),
                    was: Some("0".into()),
                    value: Some("42".into()),
                }],
            }
        );
    }

    #[test]
    fn treats_a_reordering_of_the_same_elements_as_no_change() {
        let before = [button("Save", None), button("Cancel", None)];
        let after = [button("Cancel", None), button("Save", None)];
        assert_eq!(
            diff_actionable_elements(&before, &after),
            ComputerActionableElementsDiff::default()
        );
    }

    #[test]
    fn keeps_duplicate_labels_distinct_by_count() {
        let before = [button("Save", None), button("Save", None)];
        let after = [button("Save", None)];
        assert_eq!(
            diff_actionable_elements(&before, &after),
            ComputerActionableElementsDiff {
                removed: vec![button("Save", None)],
                ..Default::default()
            }
        );
    }

    #[test]
    fn separates_identical_labels_across_windows() {
        let before = [in_window("Save", "editor"), in_window("Save", "browser")];
        let after = [in_window("Save", "editor")];
        assert_eq!(
            diff_actionable_elements(&before, &after),
            ComputerActionableElementsDiff {
                removed: vec![in_window("Save", "browser")],
                ..Default::default()
            }
        );
    }

    #[test]
    fn reports_an_edit_past_the_clamped_prefix_of_a_long_value() {
        let tree = |value: &str| {
            node(Node {
                role: "AXTextArea",
                label: Some("Body"),
                value: Some(value),
                window_id: Some("editor"),
                ..Default::default()
            })
        };
        let prefix = "a".repeat(ELEMENT_VALUE_MAX_LENGTH + 10);
        let before = digest(&tree(&format!("{prefix} one")));
        let after = digest(&tree(&format!("{prefix} two")));
        assert_eq!(before.items[0].value, after.items[0].value);
        assert_eq!(
            diff_actionable_elements(&before.items, &after.items)
                .changed
                .len(),
            1
        );
    }

    #[test]
    fn reports_a_gained_or_lost_value_as_a_change_not_a_remove_add_pair() {
        let before = [button("Notes", None)];
        let after = [button("Notes", Some("draft text"))];
        let change = |was: Option<&str>, value: Option<&str>| ComputerActionableElementChange {
            element_ref: 0,
            role: "push button".into(),
            label: "Notes".into(),
            window_id: Some("editor".into()),
            was: was.map(str::to_owned),
            value: value.map(str::to_owned),
        };
        assert_eq!(
            diff_actionable_elements(&before, &after).changed,
            [change(None, Some("draft text"))]
        );
        assert_eq!(
            diff_actionable_elements(&after, &before).changed,
            [change(Some("draft text"), None)]
        );
    }

    fn two_saves() -> ComputerUiNode {
        node(Node {
            role: "desktop",
            children: vec![node(Node {
                role: "window",
                label: Some("Editor"),
                window_id: Some("editor"),
                children: vec![
                    node(Node {
                        role: "push button",
                        label: Some("Save"),
                        window_id: Some("editor"),
                        activation_point: Some((10.0, 10.0)),
                        ..Node::default()
                    }),
                    node(Node {
                        role: "push button",
                        label: Some("Save"),
                        window_id: Some("editor"),
                        activation_point: Some((20.0, 20.0)),
                        ..Node::default()
                    }),
                ],
                ..Node::default()
            })],
            ..Node::default()
        })
    }

    fn ordinal_target(label: &str, ordinal: u64) -> ComputerTarget {
        ComputerTarget {
            ref_ordinal: Some(ordinal),
            ..target(Some(label), None, None)
        }
    }

    #[test]
    fn picks_the_ordinal_th_control_sharing_the_exact_identity() {
        let root = two_saves();
        let found =
            resolve_computer_semantic_target(&root, &ordinal_target("Save", 1), false).unwrap();
        assert_eq!(found.point, ComputerPoint { x: 20.0, y: 20.0 });
    }

    #[test]
    fn refuses_a_disappeared_ordinal_instead_of_targeting_the_sole_survivor() {
        let one_save = node(Node {
            role: "desktop",
            children: vec![node(Node {
                role: "push button",
                label: Some("Save"),
                window_id: Some("editor"),
                activation_point: Some((10.0, 10.0)),
                ..Node::default()
            })],
            ..Node::default()
        });
        for allow_offscreen in [false, true] {
            let error = resolve_computer_semantic_target(
                &one_save,
                &ordinal_target("Save", 1),
                allow_offscreen,
            )
            .unwrap_err();
            assert_eq!(error.code, ComputerTargetErrorCode::NotFound);
            assert!(error.not_found);
            assert!(
                error
                    .message
                    .contains("Observe again with computer_get_state")
            );
        }
    }

    #[test]
    fn refuses_a_disappeared_ordinal_even_when_multiple_controls_survive() {
        let error = semantic_error(&two_saves(), &ordinal_target("Save", 2));
        assert_eq!(error.code, ComputerTargetErrorCode::NotFound);
        assert_eq!(error.candidates.len(), 2);
    }

    #[test]
    fn reports_the_pools_near_misses_when_nothing_matches() {
        let error = semantic_error(&two_saves(), &ordinal_target("Missing", 0));
        assert_eq!(error.code, ComputerTargetErrorCode::NotFound);
    }

    #[test]
    fn indexes_each_item_with_its_full_identity_and_duplicate_ordinal() {
        let tree = node(Node {
            role: "desktop",
            children: vec![node(Node {
                role: "window",
                window_id: Some("editor"),
                children: vec![
                    node(Node {
                        role: "push button",
                        label: Some("Save"),
                        window_id: Some("editor"),
                        ..Node::default()
                    }),
                    node(Node {
                        role: "push button",
                        label: Some("Save"),
                        window_id: Some("editor"),
                        ..Node::default()
                    }),
                ],
                ..Node::default()
            })],
            ..Node::default()
        });
        let digest = digest(&tree);
        let refs: Vec<usize> = digest.items.iter().map(|i| i.element_ref).collect();
        assert_eq!(refs, [0, 1]);
        assert_eq!(
            digest.ref_index,
            [
                element_ref("Save", "push button", "editor", 0, &[0, 0]),
                element_ref("Save", "push button", "editor", 1, &[0, 1]),
            ]
        );
    }

    #[test]
    fn keeps_the_full_label_in_the_ref_index_when_the_wire_label_clamps() {
        let long_label = "x".repeat(200);
        let tree = node(Node {
            role: "desktop",
            children: vec![node(Node {
                role: "window",
                window_id: Some("editor"),
                children: vec![node(Node {
                    role: "push button",
                    label: Some(&long_label),
                    window_id: Some("editor"),
                    ..Node::default()
                })],
                ..Node::default()
            })],
            ..Node::default()
        });
        let digest = digest(&tree);
        assert!(digest.items[0].label.len() < long_label.len());
        assert_eq!(digest.ref_index[0].label, long_label);
    }

    #[test]
    fn wait_for_control_lookup_classifies_like_synara() {
        let root = desktop();
        let lookup =
            |target: &ComputerTarget| computer_target_lookup::<ComputerTargetError>(&root, target);
        assert_eq!(lookup(&target(Some("Save As"), None, None)), Ok(()));
        assert_eq!(
            lookup(&target(Some("Save"), None, None)),
            Err(ComputerTargetLookup::Ambiguous)
        );
        assert_eq!(
            lookup(&target(Some("Print"), None, None)),
            Err(ComputerTargetLookup::NotFound)
        );
        let hidden = node(Node {
            role: "desktop",
            children: vec![node(Node {
                role: "push button",
                label: Some("Hidden"),
                on_screen: Some(false),
                ..Node::default()
            })],
            ..Node::default()
        });
        assert!(matches!(
            computer_target_lookup::<ComputerTargetError>(&hidden, &target(Some("Hidden"), None, None)),
            Err(ComputerTargetLookup::Failed(error)) if error.code == ComputerTargetErrorCode::Offscreen
        ));
    }
}
