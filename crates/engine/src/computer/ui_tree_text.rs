use zeron_proto::computer::ComputerUiNode;

use super::utf8_truncation::clamp_text_to_length;

pub fn describe_computer_ui_tree(root: &ComputerUiNode) -> String {
    fn visit(node: &ComputerUiNode, depth: usize, lines: &mut Vec<String>) {
        let label = node
            .label
            .as_deref()
            .or(node.description.as_deref())
            .unwrap_or("(unlabelled)");
        let value = node
            .value
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(|value| format!(" = {value}"))
            .unwrap_or_default();
        let truncated = if node.truncated == Some(true) {
            " …(truncated)"
        } else {
            ""
        };
        lines.push(format!(
            "{}{}: {label}{value}{truncated}",
            "  ".repeat(depth),
            node.role
        ));
        for child in &node.children {
            visit(child, depth + 1, lines);
        }
    }
    let mut lines = Vec::new();
    visit(root, 0, &mut lines);
    lines.join("\n")
}

pub fn clamp_node_text(text: &str, max_length: usize) -> String {
    clamp_text_to_length(text, max_length)
}

#[cfg(test)]
mod tests {
    use zeron_proto::computer::ComputerRect;

    use super::*;

    fn node(
        role: &str,
        label: Option<&str>,
        value: Option<&str>,
        truncated: Option<bool>,
        children: Vec<ComputerUiNode>,
    ) -> ComputerUiNode {
        ComputerUiNode {
            role: role.into(),
            label: label.map(str::to_owned),
            value: value.map(str::to_owned),
            description: None,
            frame: ComputerRect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            activation_point: None,
            on_screen: true,
            window_id: None,
            accessibility_root: None,
            node_path: None,
            editable: None,
            truncated,
            children,
        }
    }

    #[test]
    fn renders_an_indented_tree_with_values_and_truncation_markers() {
        let tree = node(
            "window",
            Some("Editor"),
            None,
            Some(true),
            vec![
                node("entry", Some("Name"), Some("Ada"), None, vec![]),
                node("entry", None, Some(""), None, vec![]),
            ],
        );
        assert_eq!(
            describe_computer_ui_tree(&tree),
            "window: Editor …(truncated)\n  entry: Name = Ada\n  entry: (unlabelled)"
        );
        assert_eq!(clamp_node_text("abcdef", 4), "abc…");
    }
}
