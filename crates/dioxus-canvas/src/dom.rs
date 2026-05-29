//! The retained node tree + the dioxus mutation interpreter.
//!
//! dioxus drives a renderer through [`WriteMutations`] — a **stack machine** over
//! compile-time [`Template`]s, not a flat "create element / set attribute" list.
//! (`rebuild_to_vec` drops the `Template` payload and is "for testing"; a real
//! renderer must implement `WriteMutations` to receive `load_template` with the
//! actual template so it can instantiate the static skeleton.) This mirrors the
//! exact call sequence dioxus-web / dioxus-tui / blitz handle.
//!
//! The machine:
//!   - `load_template(tmpl, idx, id)` → instantiate `tmpl.roots[idx]` (elements +
//!     static text + placeholders for `Dynamic` slots), assign the root `id`,
//!     push it on the stack.
//!   - `assign_node_id(path, id)` → navigate from the top-of-stack root down the
//!     child-index `path`, give that descendant an `ElementId`.
//!   - `set_attribute` / `create_event_listener` → mutate the addressed node.
//!   - `replace_placeholder_with_nodes(path, m)` → pop `m` stack nodes, swap the
//!     placeholder at `path` for them.
//!   - `append_children(id, m)` / `insert_nodes_{after,before}(id, m)` /
//!     `replace_node_with(id, m)` → pop `m`, splice into the tree.
//!   - `ElementId(0)` is the root container we provide.

use std::collections::BTreeMap;

use dioxus_core::{AttributeValue, ElementId, Template, TemplateAttribute, TemplateNode, WriteMutations};

pub type NodeId = usize;

#[derive(Debug)]
pub enum NodeKind {
    /// `tag` plus the merged CSS-ish style map (parsed from the `style`
    /// attribute + any direct layout attributes). `listeners` records event
    /// names (e.g. `"click"`) so hit-testing can find a dispatch target.
    Element {
        tag: String,
        style: BTreeMap<String, String>,
        listeners: Vec<String>,
    },
    Text(String),
    /// A re-entrance point for dynamic content (list diffing, conditionals).
    Placeholder,
}

#[derive(Debug)]
pub struct Node {
    pub kind: NodeKind,
    pub children: Vec<NodeId>,
    pub parent: Option<NodeId>,
    /// The dioxus `ElementId` if one was assigned (used for event dispatch).
    pub element_id: Option<u32>,
}

/// The retained tree. Index `ROOT` is the synthetic container mapped to
/// `ElementId(0)`; its children are the app's root nodes.
pub struct Dom {
    nodes: Vec<Option<Node>>,
    free: Vec<NodeId>,
    /// `ElementId` (usize) → arena `NodeId`.
    element_map: Vec<Option<NodeId>>,
    /// Render stack used while applying mutations.
    stack: Vec<NodeId>,
}

pub const ROOT: NodeId = 0;

impl Dom {
    pub fn new() -> Self {
        let root = Node {
            kind: NodeKind::Element {
                tag: "root".into(),
                style: BTreeMap::new(),
                listeners: Vec::new(),
            },
            children: Vec::new(),
            parent: None,
            element_id: Some(0),
        };
        let mut element_map = Vec::new();
        element_map.push(Some(ROOT)); // ElementId(0) → root
        Dom {
            nodes: vec![Some(root)],
            free: Vec::new(),
            element_map,
            stack: Vec::new(),
        }
    }

    pub fn node(&self, id: NodeId) -> &Node {
        self.nodes[id].as_ref().expect("live node")
    }
    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        self.nodes[id].as_mut().expect("live node")
    }
    pub fn root_children(&self) -> &[NodeId] {
        &self.node(ROOT).children
    }

    fn alloc(&mut self, node: Node) -> NodeId {
        if let Some(slot) = self.free.pop() {
            self.nodes[slot] = Some(node);
            slot
        } else {
            self.nodes.push(Some(node));
            self.nodes.len() - 1
        }
    }

    fn map_element(&mut self, eid: ElementId, node: NodeId) {
        let i = eid.0;
        if self.element_map.len() <= i {
            self.element_map.resize(i + 1, None);
        }
        self.element_map[i] = Some(node);
        self.node_mut(node).element_id = Some(i as u32);
    }

    fn resolve(&self, eid: ElementId) -> NodeId {
        self.element_map
            .get(eid.0)
            .copied()
            .flatten()
            .unwrap_or(ROOT)
    }

    /// Navigate from `root` down a child-index path.
    fn navigate(&self, root: NodeId, path: &[u8]) -> NodeId {
        let mut cur = root;
        for &idx in path {
            cur = self.node(cur).children[idx as usize];
        }
        cur
    }

    /// Recursively materialise a static template node into the arena.
    fn instantiate(&mut self, tn: &TemplateNode) -> NodeId {
        match tn {
            TemplateNode::Element { tag, attrs, children, .. } => {
                let mut style = BTreeMap::new();
                for attr in attrs.iter() {
                    if let TemplateAttribute::Static { name, value, .. } = attr {
                        merge_attr(&mut style, name, value);
                    }
                    // Dynamic attrs are filled later via set_attribute.
                }
                let id = self.alloc(Node {
                    kind: NodeKind::Element { tag: (*tag).to_string(), style, listeners: Vec::new() },
                    children: Vec::new(),
                    parent: None,
                    element_id: None,
                });
                let child_ids: Vec<NodeId> = children.iter().map(|c| self.instantiate(c)).collect();
                for &c in &child_ids {
                    self.node_mut(c).parent = Some(id);
                }
                self.node_mut(id).children = child_ids;
                id
            }
            TemplateNode::Text { text } => self.alloc(Node {
                kind: NodeKind::Text((*text).to_string()),
                children: Vec::new(),
                parent: None,
                element_id: None,
            }),
            TemplateNode::Dynamic { .. } => self.alloc(Node {
                kind: NodeKind::Placeholder,
                children: Vec::new(),
                parent: None,
                element_id: None,
            }),
        }
    }

    /// Pop the top `m` nodes off the stack, preserving order.
    fn pop_n(&mut self, m: usize) -> Vec<NodeId> {
        let at = self.stack.len() - m;
        self.stack.split_off(at)
    }

    fn detach(&mut self, node: NodeId) {
        if let Some(p) = self.node(node).parent {
            let parent = self.node_mut(p);
            if let Some(pos) = parent.children.iter().position(|&c| c == node) {
                parent.children.remove(pos);
            }
        }
    }

    /// Free a node and its whole subtree (and clear element-map entries).
    fn free_subtree(&mut self, node: NodeId) {
        let kids = std::mem::take(&mut self.node_mut(node).children);
        for c in kids {
            self.free_subtree(c);
        }
        if let Some(eid) = self.node(node).element_id {
            if let Some(slot) = self.element_map.get_mut(eid as usize) {
                *slot = None;
            }
        }
        self.nodes[node] = None;
        self.free.push(node);
    }

    fn splice_into_parent(&mut self, parent: NodeId, at: usize, new: &[NodeId]) {
        for (k, &n) in new.iter().enumerate() {
            self.node_mut(n).parent = Some(parent);
            self.node_mut(parent).children.insert(at + k, n);
        }
    }
}

/// Parse one `name=value` attribute into the style map. The `style` attribute is
/// a CSS declaration list (`display:flex; gap:8px`); everything else (width,
/// padding, background, color, font-size, …) is treated as a single property.
fn merge_attr(style: &mut BTreeMap<String, String>, name: &str, value: &str) {
    if name == "style" {
        for decl in value.split(';') {
            if let Some((k, v)) = decl.split_once(':') {
                style.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
    } else {
        style.insert(name.to_string(), value.trim().to_string());
    }
}

fn attr_to_string(value: &AttributeValue) -> Option<String> {
    match value {
        AttributeValue::Text(s) => Some(s.clone()),
        AttributeValue::Float(n) => Some(n.to_string()),
        AttributeValue::Int(n) => Some(n.to_string()),
        AttributeValue::Bool(b) => Some(b.to_string()),
        AttributeValue::None => None,
        // Listeners arrive via create_event_listener; Any is unsupported.
        _ => None,
    }
}

impl WriteMutations for Dom {
    fn append_children(&mut self, id: ElementId, m: usize) {
        let parent = self.resolve(id);
        let new = self.pop_n(m);
        let at = self.node(parent).children.len();
        self.splice_into_parent(parent, at, &new);
    }

    fn assign_node_id(&mut self, path: &'static [u8], id: ElementId) {
        let root = *self.stack.last().expect("stack non-empty for assign");
        let target = self.navigate(root, path);
        self.map_element(id, target);
    }

    fn create_placeholder(&mut self, id: ElementId) {
        let n = self.alloc(Node {
            kind: NodeKind::Placeholder,
            children: Vec::new(),
            parent: None,
            element_id: None,
        });
        self.map_element(id, n);
        self.stack.push(n);
    }

    fn create_text_node(&mut self, value: &str, id: ElementId) {
        let n = self.alloc(Node {
            kind: NodeKind::Text(value.to_string()),
            children: Vec::new(),
            parent: None,
            element_id: None,
        });
        self.map_element(id, n);
        self.stack.push(n);
    }

    fn load_template(&mut self, template: Template, index: usize, id: ElementId) {
        let root = self.instantiate(&template.roots[index]);
        self.map_element(id, root);
        self.stack.push(root);
    }

    fn replace_node_with(&mut self, id: ElementId, m: usize) {
        let target = self.resolve(id);
        let new = self.pop_n(m);
        if let Some(parent) = self.node(target).parent {
            let at = self.node(parent).children.iter().position(|&c| c == target).unwrap_or(0);
            self.detach(target);
            self.splice_into_parent(parent, at, &new);
        }
        self.free_subtree(target);
    }

    fn replace_placeholder_with_nodes(&mut self, path: &'static [u8], m: usize) {
        // The `m` replacement nodes were pushed ABOVE the template root; pop
        // them first so `stack.last()` is the template root the path is
        // relative to (paths navigate the template skeleton, not the stack top).
        let new = self.pop_n(m);
        let root = *self.stack.last().expect("stack non-empty for replace_placeholder");
        let target = self.navigate(root, path);
        if let Some(parent) = self.node(target).parent {
            let at = self.node(parent).children.iter().position(|&c| c == target).unwrap_or(0);
            self.detach(target);
            self.splice_into_parent(parent, at, &new);
        }
        self.free_subtree(target);
    }

    fn insert_nodes_after(&mut self, id: ElementId, m: usize) {
        let anchor = self.resolve(id);
        let new = self.pop_n(m);
        if let Some(parent) = self.node(anchor).parent {
            let at = self.node(parent).children.iter().position(|&c| c == anchor).map(|p| p + 1).unwrap_or(0);
            self.splice_into_parent(parent, at, &new);
        }
    }

    fn insert_nodes_before(&mut self, id: ElementId, m: usize) {
        let anchor = self.resolve(id);
        let new = self.pop_n(m);
        if let Some(parent) = self.node(anchor).parent {
            let at = self.node(parent).children.iter().position(|&c| c == anchor).unwrap_or(0);
            self.splice_into_parent(parent, at, &new);
        }
    }

    fn set_attribute(
        &mut self,
        name: &'static str,
        _ns: Option<&'static str>,
        value: &AttributeValue,
        id: ElementId,
    ) {
        let node = self.resolve(id);
        if let NodeKind::Element { style, .. } = &mut self.node_mut(node).kind {
            match attr_to_string(value) {
                Some(v) => merge_attr(style, name, &v),
                None => {
                    style.remove(name);
                }
            }
        }
    }

    fn set_node_text(&mut self, value: &str, id: ElementId) {
        let node = self.resolve(id);
        if let NodeKind::Text(t) = &mut self.node_mut(node).kind {
            *t = value.to_string();
        }
    }

    fn create_event_listener(&mut self, name: &'static str, id: ElementId) {
        let node = self.resolve(id);
        if let NodeKind::Element { listeners, .. } = &mut self.node_mut(node).kind {
            if !listeners.iter().any(|l| l == name) {
                listeners.push(name.to_string());
            }
        }
    }

    fn remove_event_listener(&mut self, name: &'static str, id: ElementId) {
        let node = self.resolve(id);
        if let NodeKind::Element { listeners, .. } = &mut self.node_mut(node).kind {
            listeners.retain(|l| l != name);
        }
    }

    fn remove_node(&mut self, id: ElementId) {
        let node = self.resolve(id);
        self.detach(node);
        self.free_subtree(node);
    }

    fn push_root(&mut self, id: ElementId) {
        let node = self.resolve(id);
        self.stack.push(node);
    }
}
