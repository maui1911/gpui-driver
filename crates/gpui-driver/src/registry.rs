//! Per-window registry of `driver_id`-annotated elements.
//!
//! Records are only collected while a "collection draw" is in progress: RPC handlers
//! call [`Registry::begin_collect`], force a full redraw (`window.refresh()` +
//! `window.draw(cx)`, which disables GPUI's view paint-caching for that draw), and then
//! [`Registry::end_collect`]. Normal frames skip recording entirely, so the registry
//! costs one hash lookup per tagged element per frame and never grows between RPCs.
//!
//! Parent/child structure is recovered from prepaint nesting: children prepaint inside
//! their parent's `prepaint` call, so a per-window stack of in-flight nodes gives each
//! record its parent index.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use gpui_driver_protocol::{Bounds, TreeNode};

/// One registered element, recorded during the prepaint of a collection draw.
#[derive(Clone, Debug)]
pub(crate) struct NodeRecord {
    pub id: String,
    pub kind: String,
    pub text: Option<String>,
    /// Window-relative logical pixels.
    pub bounds: Bounds,
    /// Index into the same record vec; `None` for roots.
    pub parent: Option<usize>,
    pub interactive: bool,
    /// The real GPUI hitbox registered for this element during the collection draw.
    /// `None` only in unit tests.
    pub hitbox: Option<gpui::Hitbox>,
}

#[derive(Default)]
struct WindowNodes {
    collecting: bool,
    records: Vec<NodeRecord>,
    stack: Vec<usize>,
}

#[derive(Default)]
pub(crate) struct Registry {
    windows: Mutex<HashMap<u64, WindowNodes>>,
}

pub(crate) fn global() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(Registry::default)
}

impl Registry {
    /// Start collecting for a window: clears previous records.
    pub fn begin_collect(&self, window_id: u64) {
        let mut windows = self.windows.lock().unwrap();
        let entry = windows.entry(window_id).or_default();
        entry.collecting = true;
        entry.records.clear();
        entry.stack.clear();
    }

    /// Stop collecting and return everything recorded since `begin_collect`.
    pub fn end_collect(&self, window_id: u64) -> Vec<NodeRecord> {
        let mut windows = self.windows.lock().unwrap();
        let entry = windows.entry(window_id).or_default();
        entry.collecting = false;
        entry.stack.clear();
        entry.records.clone()
    }

    pub fn is_collecting(&self, window_id: u64) -> bool {
        let windows = self.windows.lock().unwrap();
        windows.get(&window_id).is_some_and(|w| w.collecting)
    }

    /// Record a node and push it onto the nesting stack. Call from prepaint, before
    /// delegating to the wrapped element. Returns `None` when not collecting.
    pub fn enter(&self, window_id: u64, mut record: NodeRecord) -> Option<usize> {
        let mut windows = self.windows.lock().unwrap();
        let entry = windows.entry(window_id).or_default();
        if !entry.collecting {
            return None;
        }
        record.parent = entry.stack.last().copied();
        let index = entry.records.len();
        entry.records.push(record);
        entry.stack.push(index);
        Some(index)
    }

    /// Pop the nesting stack. Call after the wrapped element's prepaint returns,
    /// only if `enter` returned `Some`.
    pub fn exit(&self, window_id: u64) {
        let mut windows = self.windows.lock().unwrap();
        if let Some(entry) = windows.get_mut(&window_id) {
            entry.stack.pop();
        }
    }
}

/// Assemble flat prepaint-ordered records into the nested tree the protocol exposes.
/// `viewport` is the window's content bounds at scale-independent logical pixels.
pub(crate) fn assemble_tree(records: &[NodeRecord], viewport: Bounds) -> TreeNode {
    let mut nodes: Vec<TreeNode> = records
        .iter()
        .map(|r| TreeNode {
            id: Some(r.id.clone()),
            kind: r.kind.clone(),
            text: r.text.clone(),
            bounds: r.bounds,
            visible: is_visible(r.bounds, viewport),
            enabled: true,
            focused: false,
            interactive: r.interactive,
            children: Vec::new(),
        })
        .collect();

    // Attach children to parents back-to-front so each node's children are complete
    // before the node itself is moved into its own parent.
    for index in (0..records.len()).rev() {
        if let Some(parent) = records[index].parent {
            let node = std::mem::replace(
                &mut nodes[index],
                TreeNode {
                    id: None,
                    kind: String::new(),
                    text: None,
                    bounds: records[index].bounds,
                    visible: false,
                    enabled: true,
                    focused: false,
                    interactive: false,
                    children: Vec::new(),
                },
            );
            // Children were pushed in prepaint order; restore that order up front.
            nodes[parent].children.insert(0, node);
        }
    }

    let roots: Vec<TreeNode> = records
        .iter()
        .enumerate()
        .filter(|(_, r)| r.parent.is_none())
        .map(|(i, _)| {
            std::mem::replace(
                &mut nodes[i],
                TreeNode {
                    id: None,
                    kind: String::new(),
                    text: None,
                    bounds: viewport,
                    visible: false,
                    enabled: true,
                    focused: false,
                    interactive: false,
                    children: Vec::new(),
                },
            )
        })
        .collect();

    TreeNode {
        id: None,
        kind: "window".into(),
        text: None,
        bounds: viewport,
        visible: true,
        enabled: true,
        focused: false,
        interactive: false,
        children: roots,
    }
}

fn is_visible(bounds: Bounds, viewport: Bounds) -> bool {
    bounds.w > 0.0
        && bounds.h > 0.0
        && bounds.x < viewport.x + viewport.w
        && bounds.y < viewport.y + viewport.h
        && bounds.x + bounds.w > viewport.x
        && bounds.y + bounds.h > viewport.y
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, parent: Option<usize>, bounds: Bounds) -> NodeRecord {
        NodeRecord {
            id: id.into(),
            kind: "div".into(),
            text: None,
            bounds,
            parent,
            interactive: true,
            hitbox: None,
        }
    }

    fn bounds(x: f32, y: f32, w: f32, h: f32) -> Bounds {
        Bounds { x, y, w, h }
    }

    const VIEWPORT: Bounds = Bounds {
        x: 0.0,
        y: 0.0,
        w: 800.0,
        h: 600.0,
    };

    #[test]
    fn assembles_nested_tree_in_prepaint_order() {
        // prepaint order: panel { save, cancel }, footer
        let records = vec![
            record("panel", None, bounds(0.0, 0.0, 800.0, 500.0)),
            record("save", Some(0), bounds(10.0, 10.0, 100.0, 30.0)),
            record("cancel", Some(0), bounds(120.0, 10.0, 100.0, 30.0)),
            record("footer", None, bounds(0.0, 500.0, 800.0, 100.0)),
        ];
        let tree = assemble_tree(&records, VIEWPORT);
        assert_eq!(tree.kind, "window");
        assert_eq!(tree.children.len(), 2);
        let panel = &tree.children[0];
        assert_eq!(panel.id.as_deref(), Some("panel"));
        assert_eq!(panel.children.len(), 2);
        assert_eq!(panel.children[0].id.as_deref(), Some("save"));
        assert_eq!(panel.children[1].id.as_deref(), Some("cancel"));
        assert_eq!(tree.children[1].id.as_deref(), Some("footer"));
    }

    #[test]
    fn deep_nesting() {
        let records = vec![
            record("a", None, bounds(0.0, 0.0, 100.0, 100.0)),
            record("b", Some(0), bounds(0.0, 0.0, 80.0, 80.0)),
            record("c", Some(1), bounds(0.0, 0.0, 60.0, 60.0)),
        ];
        let tree = assemble_tree(&records, VIEWPORT);
        let a = &tree.children[0];
        let b = &a.children[0];
        let c = &b.children[0];
        assert_eq!(c.id.as_deref(), Some("c"));
        assert!(c.children.is_empty());
    }

    #[test]
    fn visibility_against_viewport() {
        let records = vec![
            record("on", None, bounds(10.0, 10.0, 50.0, 50.0)),
            record("off_right", None, bounds(900.0, 10.0, 50.0, 50.0)),
            record("zero", None, bounds(10.0, 10.0, 0.0, 0.0)),
        ];
        let tree = assemble_tree(&records, VIEWPORT);
        assert!(tree.children[0].visible);
        assert!(!tree.children[1].visible);
        assert!(!tree.children[2].visible);
    }

    #[test]
    fn registry_enter_exit_tracks_parents_only_while_collecting() {
        let reg = Registry::default();
        // Not collecting: enter is a no-op.
        assert!(reg.enter(1, record("x", None, VIEWPORT)).is_none());

        reg.begin_collect(1);
        let a = reg.enter(1, record("a", None, VIEWPORT)).unwrap();
        let b = reg.enter(1, record("b", None, VIEWPORT)).unwrap();
        reg.exit(1);
        let c = reg.enter(1, record("c", None, VIEWPORT)).unwrap();
        reg.exit(1);
        reg.exit(1);
        let records = reg.end_collect(1);

        assert_eq!(records.len(), 3);
        assert_eq!(records[a].parent, None);
        assert_eq!(records[b].parent, Some(a));
        assert_eq!(records[c].parent, Some(a));

        // After end_collect, recording stops again.
        assert!(reg.enter(1, record("y", None, VIEWPORT)).is_none());
    }
}
