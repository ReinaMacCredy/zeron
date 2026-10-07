use std::collections::{HashMap, HashSet};

use zeron_proto::computer::{ComputerAxLine, ComputerAxTree, ComputerUiNode, ComputerWindow};

use crate::computer::ui_tree_targeting::ComputerActionableElements;
use crate::computer::utf8_truncation::clamp_text_to_length;

pub(super) const TREE_REF_LIMIT: usize = 1024;
const TREE_TEXT_MAX_CHARS: usize = 40_000;
const TREE_VALUE_MAX_CHARS: usize = 1_000;
const QUIET_ACTIONS: &[&str] = &[
    "AXPress",
    "press",
    "AXScrollToVisible",
    "AXShowDefaultUI",
    "AXShowAlternateUI",
];

#[derive(Debug, Clone, PartialEq)]
pub(super) struct TreeSnapshot {
    window: String,
    header: String,
    lines: Vec<TreeLine>,
    focused: Option<String>,
    selected_text: Option<String>,
    truncated: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct TreeLine {
    depth: usize,
    element_ref: Option<usize>,
    identity: String,
    text: String,
}

impl TreeLine {
    fn numbered(&self) -> String {
        match self.element_ref {
            Some(element_ref) => format!("{element_ref} {}", self.text),
            None => self.text.clone(),
        }
    }
}

pub(super) fn build_tree(
    window: Option<&ComputerWindow>,
    window_id: &str,
    ax: Option<&ComputerAxTree>,
    root: Option<&ComputerUiNode>,
    refs: &ComputerActionableElements,
) -> Option<TreeSnapshot> {
    let by_path: HashMap<&[u32], usize> = refs
        .ref_index
        .iter()
        .zip(&refs.items)
        .map(|(entry, item)| (entry.path.as_slice(), item.element_ref))
        .collect();
    let ref_of = |line: &ComputerAxLine| {
        line.target
            .as_deref()
            .and_then(|path| by_path.get(path).copied())
    };
    let source = match ax {
        Some(ax) => without_menu_bar(&ax.lines),
        None => fallback_lines(root?, window_id),
    };
    if source.is_empty() {
        return None;
    }
    let mut lines = Vec::with_capacity(source.len());
    let mut ancestors: Vec<(usize, String)> = Vec::new();
    let mut ordinals: HashMap<String, usize> = HashMap::new();
    for line in &source {
        let depth = line.depth as usize;
        while ancestors.last().is_some_and(|(at, _)| *at >= depth) {
            ancestors.pop();
        }
        let element_ref = ref_of(line);
        let identity = match element_ref {
            Some(element_ref) => format!("#{element_ref}"),
            None => {
                let parent = ancestors
                    .last()
                    .map_or("", |(_, identity)| identity.as_str());
                let name = [&line.title, &line.description, &line.identifier]
                    .into_iter()
                    .find_map(|text| text.as_deref().filter(|text| !text.is_empty()))
                    .unwrap_or_default();
                let key = format!("{parent}/{}:{name}", line.role);
                let ordinal = ordinals.entry(key.clone()).or_insert(0);
                *ordinal += 1;
                format!("{key}~{ordinal}")
            }
        };
        ancestors.push((depth, identity.clone()));
        lines.push(TreeLine {
            depth,
            element_ref,
            identity,
            text: describe_line(line),
        });
    }
    let title = window
        .map(|window| window.title.as_str())
        .filter(|title| !title.is_empty())
        .or_else(|| window.and_then(|window| window.app_name.as_deref()))
        .unwrap_or(window_id);
    let mut header = format!("Window: {}", quote(title));
    if let Some(app) = window.and_then(|window| window.app_name.as_deref()) {
        header.push_str(&format!(", App: {app}"));
    }
    if let Some(url) = ax.and_then(|ax| ax.url.as_deref()) {
        header.push_str(&format!(", URL: {url}"));
    }
    header.push('.');
    let focused = ax.and_then(|ax| ax.focused.as_ref()).map(|line| {
        let text = describe_line(line);
        match ref_of(line) {
            Some(element_ref) => format!("The focused UI element is {element_ref} {text}"),
            None => format!("The focused UI element is {text}"),
        }
    });
    Some(TreeSnapshot {
        window: format!("Window: {}", quote(title)),
        header,
        lines,
        focused,
        selected_text: ax
            .and_then(|ax| ax.selected_text.clone())
            .filter(|text| !text.is_empty()),
        truncated: ax.is_some_and(|ax| ax.truncated),
    })
}

pub(super) fn render_full(tree: &TreeSnapshot) -> String {
    let mut out = tree.header.clone();
    let mut shown = 0;
    for line in &tree.lines {
        let row = format!("\n{}{}", "\t".repeat(line.depth), line.numbered());
        if out.len() + row.len() > TREE_TEXT_MAX_CHARS {
            break;
        }
        out.push_str(&row);
        shown += 1;
    }
    if shown < tree.lines.len() {
        out.push_str(&format!(
            "\n… {} more lines not shown: the tree exceeds {TREE_TEXT_MAX_CHARS} characters. Narrow with label_contains on the elements listing.",
            tree.lines.len() - shown
        ));
    }
    if tree.truncated {
        out.push_str(
            "\nThe accessibility walk stopped early; elements past this point are missing.",
        );
    }
    push_footer(&mut out, tree);
    out
}

pub(super) fn render_diff(previous: &TreeSnapshot, current: &TreeSnapshot) -> String {
    let before: HashMap<&str, &str> = previous
        .lines
        .iter()
        .map(|line| (line.identity.as_str(), line.text.as_str()))
        .collect();
    let mut unmatched: HashMap<(usize, &str), usize> = HashMap::new();
    for line in previous
        .lines
        .iter()
        .filter(|line| line.element_ref.is_none())
    {
        *unmatched
            .entry((line.depth, line.text.as_str()))
            .or_insert(0) += 1;
    }
    let mut rows = Vec::new();
    if previous.header != current.header {
        rows.push(format!("~ {}", current.header));
    }
    let current_texts: HashSet<(usize, &str)> = current
        .lines
        .iter()
        .filter(|line| line.element_ref.is_none())
        .map(|line| (line.depth, line.text.as_str()))
        .collect();
    let mut changed_unnumbered = 0;
    for line in &current.lines {
        if line.element_ref.is_none()
            && let Some(count) = unmatched.get_mut(&(line.depth, line.text.as_str()))
            && *count > 0
        {
            *count -= 1;
            continue;
        }
        match before.get(line.identity.as_str()) {
            None => rows.push(format!("+ {}", line.numbered())),
            Some(text)
                if *text != line.text
                    && line.element_ref.is_none()
                    && current_texts.contains(&(line.depth, *text)) =>
            {
                rows.push(format!("+ {}", line.numbered()));
            }
            Some(text) if *text != line.text => {
                if line.element_ref.is_none() {
                    changed_unnumbered += 1;
                }
                rows.push(format!("~ {}", line.numbered()));
            }
            Some(_) => {}
        }
    }
    let present: HashSet<&str> = current
        .lines
        .iter()
        .map(|line| line.identity.as_str())
        .collect();
    let mut removed_refs = Vec::new();
    for line in &previous.lines {
        if let Some(element_ref) = line.element_ref
            && !present.contains(line.identity.as_str())
        {
            removed_refs.push(element_ref);
        }
    }
    let removed_unnumbered = unmatched
        .values()
        .sum::<usize>()
        .saturating_sub(changed_unnumbered);
    let focus_changed =
        previous.focused != current.focused || previous.selected_text != current.selected_text;
    if rows.is_empty() && removed_refs.is_empty() && removed_unnumbered == 0 && !focus_changed {
        return format!(
            "There has been no change in the accessibility tree for {}.",
            current.window
        );
    }
    let mut out = format!(
        "The following is a diff from the previous accessibility tree for {} with ~ and + representing changed and added elements, respectively. Removed elements are summarized by ID range.",
        current.window
    );
    let mut length = out.len();
    let mut shown = 0;
    for row in &rows {
        if length + row.len() + 1 > TREE_TEXT_MAX_CHARS {
            break;
        }
        out.push('\n');
        out.push_str(row);
        length += row.len() + 1;
        shown += 1;
    }
    if shown < rows.len() {
        out.push_str(&format!(
            "\n… {} more changed lines not shown; pass full:true for the whole tree.",
            rows.len() - shown
        ));
    }
    if !removed_refs.is_empty() {
        out.push_str(&format!("\nRemoved: {}", id_ranges(&mut removed_refs)));
    }
    if removed_unnumbered > 0 {
        out.push_str(&format!("\nRemoved {removed_unnumbered} unnumbered lines."));
    }
    if current.truncated {
        out.push_str(
            "\nThe accessibility walk stopped early; elements past this point are missing.",
        );
    }
    if focus_changed {
        push_footer(&mut out, current);
    }
    out
}

fn push_footer(out: &mut String, tree: &TreeSnapshot) {
    if let Some(focused) = &tree.focused {
        out.push('\n');
        out.push_str(focused);
    }
    if let Some(selected) = &tree.selected_text {
        out.push_str(&format!(
            "\nSelected text: ```{}```",
            clamp_text_to_length(selected, TREE_VALUE_MAX_CHARS)
        ));
    }
}

fn id_ranges(ids: &mut [usize]) -> String {
    ids.sort_unstable();
    let mut ranges: Vec<String> = Vec::new();
    let mut index = 0;
    while index < ids.len() {
        let start = ids[index];
        let mut end = start;
        while index + 1 < ids.len() && ids[index + 1] == end + 1 {
            index += 1;
            end = ids[index];
        }
        ranges.push(if start == end {
            start.to_string()
        } else {
            format!("{start}-{end}")
        });
        index += 1;
    }
    ranges.join(", ")
}

fn without_menu_bar(lines: &[ComputerAxLine]) -> Vec<ComputerAxLine> {
    let mut kept = Vec::with_capacity(lines.len());
    let mut skip_below: Option<u32> = None;
    for line in lines {
        if let Some(depth) = skip_below {
            if line.depth > depth {
                continue;
            }
            skip_below = None;
        }
        if line.role == "AXMenuBar" {
            skip_below = Some(line.depth);
            continue;
        }
        kept.push(line.clone());
    }
    kept
}

fn fallback_lines(root: &ComputerUiNode, window_id: &str) -> Vec<ComputerAxLine> {
    fn visit(
        node: &ComputerUiNode,
        path: &mut Vec<u32>,
        depth: u32,
        window_id: &str,
        out: &mut Vec<ComputerAxLine>,
    ) {
        let included = node.window_id.as_deref() == Some(window_id);
        if included {
            out.push(ComputerAxLine {
                depth,
                role: node.role.clone(),
                title: node.label.clone(),
                description: node.description.clone(),
                value: node.value.clone(),
                settable: node.editable == Some(true),
                target: Some(path.clone()),
                ..ComputerAxLine::default()
            });
        }
        let child_depth = if included { depth + 1 } else { depth };
        for (index, child) in node.children.iter().enumerate() {
            path.push(index as u32);
            visit(child, path, child_depth, window_id, out);
            path.pop();
        }
    }
    let mut out = Vec::new();
    visit(root, &mut Vec::new(), 0, window_id, &mut out);
    out
}

fn describe_line(line: &ComputerAxLine) -> String {
    let mut head = role_words(&line.role);
    let mut flags = Vec::new();
    if line.settable {
        flags.push("settable");
    }
    if line.selected == Some(true) {
        flags.push("selected");
    }
    if line.enabled == Some(false) {
        flags.push("disabled");
    }
    match line.expanded {
        Some(true) => flags.push("expanded"),
        Some(false) => flags.push("collapsed"),
        None => {}
    }
    if !flags.is_empty() {
        head.push_str(&format!(" ({})", flags.join(", ")));
    }
    let title = present(&line.title);
    if let Some(title) = &title {
        head.push(' ');
        head.push_str(title);
    }
    let mut attributes = Vec::new();
    if let Some(description) = present(&line.description).filter(|d| Some(d) != title.as_ref()) {
        attributes.push(format!("Description: {description}"));
    }
    if let Some(identifier) = present(&line.identifier) {
        attributes.push(format!("ID: {identifier}"));
    }
    if let Some(value) = present(&line.value) {
        attributes.push(format!(
            "Value: {}",
            clamp_text_to_length(&value, TREE_VALUE_MAX_CHARS)
        ));
    }
    if let Some(help) = present(&line.help) {
        attributes.push(format!("Help: {help}"));
    }
    let secondary: Vec<String> = line
        .actions
        .iter()
        .filter(|action| !QUIET_ACTIONS.contains(&action.as_str()))
        .map(|action| action_words(action))
        .filter(|action| !action.is_empty())
        .collect();
    if !secondary.is_empty() {
        attributes.push(format!("Secondary Actions: {}", secondary.join(", ")));
    }
    match (attributes.is_empty(), title.is_some()) {
        (true, _) => head,
        (false, true) => format!("{head}, {}", attributes.join(", ")),
        (false, false) => format!("{head} {}", attributes.join(", ")),
    }
}

fn present(text: &Option<String>) -> Option<String> {
    let cleaned = text
        .as_deref()?
        .replace("\r\n", "\\n")
        .replace(['\n', '\r'], "\\n")
        .replace('\t', " ");
    let trimmed = cleaned.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn quote(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}

fn role_words(role: &str) -> String {
    let known = match role {
        "AXStaticText" => "text",
        "AXTextArea" => "text entry area",
        "AXTextField" => "text field",
        "AXSearchField" => "search text field",
        "AXSecureTextField" => "secure text field",
        "AXScrollArea" => "scroll area",
        "AXSplitGroup" => "split group",
        "AXGroup" => "container",
        "AXCheckBox" => "checkbox",
        "AXPopUpButton" => "pop up button",
        "AXIncrementor" => "stepper",
        "AXWebArea" => "HTML content",
        _ => "",
    };
    if !known.is_empty() {
        return known.to_owned();
    }
    let bare = role.strip_prefix("AX").unwrap_or(role);
    split_words(bare).replace('-', " ")
}

fn action_words(action: &str) -> String {
    if let Some(named) = action.strip_prefix("Name:") {
        return named.lines().next().unwrap_or_default().trim().to_owned();
    }
    let bare = action.strip_prefix("AX").unwrap_or(action);
    let words = split_words(bare);
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn split_words(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 4);
    let mut previous_lower = false;
    for character in text.chars() {
        if character.is_uppercase() && previous_lower {
            out.push(' ');
        }
        previous_lower = character.is_lowercase() || character.is_ascii_digit();
        out.extend(character.to_lowercase());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(depth: u32, role: &str) -> ComputerAxLine {
        ComputerAxLine {
            depth,
            role: role.into(),
            ..ComputerAxLine::default()
        }
    }

    #[test]
    fn renders_codex_style_attributes_flags_and_secondary_actions() {
        let window = ComputerAxLine {
            title: Some("Calculator".into()),
            identifier: Some("main".into()),
            actions: vec!["AXRaise".into()],
            ..line(0, "AXWindow")
        };
        assert_eq!(
            describe_line(&window),
            "window Calculator, ID: main, Secondary Actions: Raise"
        );
        let display = ComputerAxLine {
            description: Some("Edit field".into()),
            value: Some("408".into()),
            ..line(2, "AXStaticText")
        };
        assert_eq!(
            describe_line(&display),
            "text Description: Edit field, Value: 408"
        );
        let editor = ComputerAxLine {
            identifier: Some("First Text View".into()),
            value: Some("alpha\nbeta".into()),
            settable: true,
            actions: vec![
                "AXShowMenu".into(),
                "Name:Copy\nTarget:0x0\nSelector:(null)".into(),
                "AXScrollToVisible".into(),
            ],
            ..line(1, "AXTextArea")
        };
        assert_eq!(
            describe_line(&editor),
            "text entry area (settable) ID: First Text View, Value: alpha\\nbeta, Secondary Actions: Show menu, Copy"
        );
    }

    #[test]
    fn an_inserted_row_does_not_mark_the_unchanged_rows_below_it_as_changed() {
        let row = |text: &str| TreeLine {
            depth: 2,
            element_ref: None,
            identity: String::new(),
            text: text.to_owned(),
        };
        let snapshot = |lines: Vec<TreeLine>| TreeSnapshot {
            window: "Window: \"Calculator\"".into(),
            header: "Window: \"Calculator\".".into(),
            lines,
            focused: None,
            selected_text: None,
            truncated: false,
        };
        let mut before = vec![row("text Value: 5.535"), row("text Value: 42")];
        for (index, line) in before.iter_mut().enumerate() {
            line.identity = format!("row~{index}");
        }
        let mut after = vec![
            row("text Value: 1.641"),
            row("text Value: 5.535"),
            row("text Value: 42"),
        ];
        for (index, line) in after.iter_mut().enumerate() {
            line.identity = format!("row~{index}");
        }
        let diff = render_diff(&snapshot(before), &snapshot(after));
        assert!(diff.contains("\n+ text Value: 1.641"), "{diff}");
        assert!(!diff.contains("5.535"), "{diff}");
        assert!(!diff.contains("Value: 42"), "{diff}");
        assert!(!diff.contains("\nRemoved"), "{diff}");
    }

    #[test]
    fn summarizes_removed_refs_as_ranges() {
        assert_eq!(id_ranges(&mut [12, 15, 13, 14, 20, 3]), "3, 12-15, 20");
    }
}
