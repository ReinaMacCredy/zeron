use std::sync::Arc;

use zeron_proto::computer::{ComputerTarget, ComputerUiNode};

use super::ui_tree_targeting::ComputerActionableElementRef;

#[derive(Debug, Clone, PartialEq)]
pub struct ObservedComputerElement {
    pub node: ComputerUiNode,
    pub identity: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ObservedComputerTarget {
    pub target: ComputerTarget,
    observed: Option<Arc<ObservedComputerElement>>,
}

impl From<ComputerTarget> for ObservedComputerTarget {
    fn from(target: ComputerTarget) -> Self {
        Self {
            target,
            observed: None,
        }
    }
}

pub(crate) fn retain_computer_element(
    node: &ComputerUiNode,
    identity: Option<String>,
) -> Option<Arc<ObservedComputerElement>> {
    identity.map(|identity| {
        Arc::new(ObservedComputerElement {
            node: ComputerUiNode {
                children: Vec::new(),
                ..node.clone()
            },
            identity,
        })
    })
}

pub fn computer_element_ref_identity(element: &ComputerActionableElementRef) -> Option<&str> {
    element
        .observed
        .as_deref()
        .map(|observed| observed.identity.as_str())
}

pub fn bind_computer_target_ref(
    target: ComputerTarget,
    element: &ComputerActionableElementRef,
) -> ObservedComputerTarget {
    ObservedComputerTarget {
        target,
        observed: element.observed.clone(),
    }
}

pub fn observed_computer_target_node(target: &ObservedComputerTarget) -> Option<&ComputerUiNode> {
    target.observed.as_deref().map(|observed| &observed.node)
}

#[cfg(test)]
mod tests {
    use zeron_proto::computer::ComputerRect;

    use super::*;
    use crate::computer::ui_tree_targeting::{
        ComputerActionableElementFilter, actionable_elements,
    };

    fn node(label: &str, children: Vec<ComputerUiNode>) -> ComputerUiNode {
        ComputerUiNode {
            role: "AXButton".into(),
            label: Some(label.into()),
            value: None,
            description: None,
            frame: ComputerRect {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
            },
            activation_point: None,
            on_screen: true,
            window_id: Some("w".into()),
            accessibility_root: None,
            node_path: None,
            editable: None,
            truncated: None,
            children,
        }
    }

    #[test]
    fn binds_only_refs_whose_node_carries_a_native_identity() {
        let tree = node("Root", vec![node("Save", vec![]), node("Open", vec![])]);
        let digest = actionable_elements(&tree, &ComputerActionableElementFilter::default(), |n| {
            (n.label.as_deref() == Some("Save")).then(|| "token-save".to_owned())
        });
        let save = &digest.ref_index[1];
        let open = &digest.ref_index[2];
        assert_eq!(computer_element_ref_identity(save), Some("token-save"));
        assert_eq!(computer_element_ref_identity(open), None);

        let bound = bind_computer_target_ref(ComputerTarget::default(), save);
        let observed = observed_computer_target_node(&bound).unwrap();
        assert_eq!(observed.label.as_deref(), Some("Save"));
        assert!(observed.children.is_empty());
        assert!(
            observed_computer_target_node(&bind_computer_target_ref(
                ComputerTarget::default(),
                open
            ))
            .is_none()
        );
        let wire = serde_json::to_value(&bound.target).unwrap();
        assert!(!wire.to_string().contains("token-save"));
    }
}
