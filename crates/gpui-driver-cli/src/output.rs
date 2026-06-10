//! Human-readable rendering. `--json` bypasses all of this and prints the raw RPC
//! result, which is also the default when stdout is not a terminal.

use gpui_driver_protocol::TreeNode;

/// Compact indented tree: cheaper in agent context windows than raw JSON.
pub fn render_tree(node: &TreeNode, interactive_only: bool) -> String {
    let mut out = String::new();
    render_node(node, 0, interactive_only, &mut out);
    out
}

fn render_node(node: &TreeNode, depth: usize, interactive_only: bool, out: &mut String) {
    let include = !interactive_only || node.interactive || has_interactive_descendant(node);
    if include {
        out.push_str(&"  ".repeat(depth));
        match &node.id {
            Some(id) => out.push_str(id),
            None => out.push_str(&format!("({})", node.kind)),
        }
        if node.id.is_some() {
            out.push_str(&format!(" <{}>", node.kind));
        }
        if let Some(text) = &node.text {
            out.push_str(&format!(" {text:?}"));
        }
        out.push_str(&format!(
            " [{:.0},{:.0} {:.0}x{:.0}]",
            node.bounds.x, node.bounds.y, node.bounds.w, node.bounds.h
        ));
        if !node.visible {
            out.push_str(" (hidden)");
        }
        if node.focused {
            out.push_str(" (focused)");
        }
        out.push('\n');
    }
    for child in &node.children {
        render_node(
            child,
            depth + usize::from(include),
            interactive_only,
            out,
        );
    }
}

fn has_interactive_descendant(node: &TreeNode) -> bool {
    node.children
        .iter()
        .any(|c| c.interactive || has_interactive_descendant(c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_driver_protocol::Bounds;

    fn node(id: Option<&str>, interactive: bool, children: Vec<TreeNode>) -> TreeNode {
        TreeNode {
            id: id.map(Into::into),
            kind: "div".into(),
            text: None,
            bounds: Bounds { x: 0.0, y: 0.0, w: 100.0, h: 50.0 },
            visible: true,
            enabled: true,
            focused: false,
            interactive,
            children,
        }
    }

    #[test]
    fn renders_indented_ids() {
        let tree = node(None, false, vec![
            node(Some("save"), true, vec![]),
            node(Some("panel"), false, vec![node(Some("cancel"), true, vec![])]),
        ]);
        let out = render_tree(&tree, false);
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].starts_with("(div)"));
        assert!(lines[1].starts_with("  save <div>"));
        assert!(lines[2].starts_with("  panel <div>"));
        assert!(lines[3].starts_with("    cancel <div>"));
    }

    #[test]
    fn interactive_only_keeps_structural_ancestors() {
        let tree = node(None, false, vec![
            node(Some("decoration"), false, vec![]),
            node(Some("panel"), false, vec![node(Some("ok"), true, vec![])]),
        ]);
        let out = render_tree(&tree, true);
        assert!(!out.contains("decoration"));
        assert!(out.contains("panel"));
        assert!(out.contains("ok"));
    }
}
