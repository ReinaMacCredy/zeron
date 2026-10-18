use super::tree::{Node, Tree};
use serde_json::{Value, json};
use std::collections::HashMap;

const INTERACTIVE: &[&str] = &[
    "Button",
    "DefaultButton",
    "CheckBox",
    "RadioButton",
    "Switch",
    "TextInput",
    "MultilineTextInput",
    "SearchInput",
    "PasswordInput",
    "EmailInput",
    "NumberInput",
    "UrlInput",
    "ComboBox",
    "EditableComboBox",
    "Link",
    "MenuItem",
    "MenuItemCheckBox",
    "MenuItemRadio",
    "Tab",
    "Slider",
    "SpinButton",
    "ListBoxOption",
    "TreeItem",
];
const ROLELESS: &[&str] = &["GenericContainer", "Unknown", "Label"];

fn interactive(node: &Node) -> bool {
    INTERACTIVE.contains(&node.role.as_str())
}

fn unnamed(node: &Node) -> bool {
    [&node.label, &node.value, &node.description]
        .into_iter()
        .all(|text| text.as_deref().is_none_or(|text| text.trim().is_empty()))
}

fn issue(node: &Node, rule: &str) -> Value {
    let mut issue = json!({ "ref": node.id, "rule": rule, "role": node.role });
    for (key, value) in [
        ("label", node.label.as_ref().map(|label| json!(label))),
        ("bounds", node.bounds.map(|bounds| json!(bounds))),
        ("view", node.view.as_ref().map(|view| json!(view))),
        ("source", node.source.as_ref().map(|source| json!(source))),
    ] {
        if let Some(value) = value {
            issue[key] = value;
        }
    }
    issue
}

pub(super) fn audit(tree: &Tree, focus_held: bool) -> Value {
    let (width, height) = tree.size;
    let mut issues = Vec::new();
    let mut siblings: HashMap<(Option<usize>, &str, &str), Vec<usize>> = HashMap::new();
    let (mut interactive_count, mut unlabeled) = (0, 0);
    for (i, node) in tree.nodes.iter().enumerate() {
        if node.supports("click") && ROLELESS.contains(&node.role.as_str()) {
            issues.push(issue(node, "clickable-without-role"));
        }
        if !interactive(node) {
            continue;
        }
        interactive_count += 1;
        if unnamed(node) {
            unlabeled += 1;
            issues.push(issue(node, "unlabeled"));
        }
        if let Some(b) = node.bounds
            && (b.x + b.w <= 0.0 || b.y + b.h <= 0.0 || b.x >= width || b.y >= height)
        {
            issues.push(issue(node, "offscreen-interactive"));
        }
        if let Some(label) = node
            .label
            .as_deref()
            .filter(|label| !label.trim().is_empty())
        {
            siblings
                .entry((node.parent, node.role.as_str(), label))
                .or_default()
                .push(i);
        }
    }
    let mut duplicates: Vec<usize> = siblings
        .into_values()
        .filter(|group| group.len() > 1)
        .flatten()
        .collect();
    duplicates.sort_unstable();
    issues.extend(
        duplicates
            .into_iter()
            .map(|i| issue(&tree.nodes[i], "duplicate-label")),
    );
    if focus_held && tree.focus().is_none() {
        issues.push(json!({
            "rule": "no-focus-node",
            "detail": "an element holds keyboard focus but no a11y node reports focus; give the focused element an id and a role",
        }));
    }
    json!({
        "counts": {
            "nodes": tree.nodes.len(),
            "interactive": interactive_count,
            "tabStops": tree.tab_stops,
            "unlabeledInteractive": unlabeled,
        },
        "issues": issues,
    })
}

#[cfg(test)]
mod tests {
    use super::super::tree::Bounds;
    use super::super::tree::tests::{node, tree_of};
    use super::*;

    fn rules(report: &Value) -> Vec<(String, String)> {
        report["issues"]
            .as_array()
            .unwrap()
            .iter()
            .map(|issue| {
                (
                    issue["ref"].as_str().unwrap_or("-").to_owned(),
                    issue["rule"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    #[test]
    fn audit_rules() {
        let mut icon = node("e3", "Button", None);
        icon.source = Some("crates/ui/src/shell.rs:42:9".into());
        let mut described = node("e4", "Button", None);
        described.description = Some("Close".into());
        let mut div = node("e5", "GenericContainer", None);
        div.actions = vec!["click".into()];
        let mut offscreen = node("e6", "Link", Some("Docs"));
        offscreen.bounds = Some(Bounds {
            x: 900.0,
            y: 10.0,
            w: 40.0,
            h: 20.0,
        });
        let tree = tree_of(&[
            (0, node("e1", "Window", Some("Zeron"))),
            (1, node("e2", "Group", None)),
            (2, icon),
            (2, described),
            (2, div),
            (2, offscreen),
            (2, node("e7", "Tab", Some("Chat"))),
            (2, node("e8", "Tab", Some("Chat"))),
            (2, node("e9", "Button", Some("Chat"))),
            (1, node("e10", "Tab", Some("Chat"))),
        ]);
        let report = audit(&tree, true);
        assert_eq!(
            rules(&report),
            [
                ("e3", "unlabeled"),
                ("e5", "clickable-without-role"),
                ("e6", "offscreen-interactive"),
                ("e7", "duplicate-label"),
                ("e8", "duplicate-label"),
                ("-", "no-focus-node"),
            ]
            .map(|(id, rule)| (id.to_owned(), rule.to_owned()))
        );
        assert_eq!(report["issues"][0]["source"], "crates/ui/src/shell.rs:42:9");
        assert_eq!(
            report["counts"],
            json!({ "nodes": 10, "interactive": 7, "tabStops": 3, "unlabeledInteractive": 1 })
        );
        let mut focused = tree.clone();
        focused.nodes[3].focused = true;
        assert!(
            !rules(&audit(&focused, true))
                .iter()
                .any(|(_, rule)| rule == "no-focus-node")
        );
        assert!(
            !rules(&audit(&tree, false))
                .iter()
                .any(|(_, rule)| rule == "no-focus-node")
        );
    }
}
