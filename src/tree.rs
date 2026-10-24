//! The parts of sway's layout tree that swayling needs.

use std::borrow::Cow;

use serde::Deserialize;

/// Name sway uses for the hidden scratchpad workspace.
pub const SCRATCHPAD: &str = "__i3_scratch";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    SplitH,
    SplitV,
    Stacked,
    Tabbed,
    #[serde(other)]
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeType {
    Workspace,
    Con,
    #[serde(other)]
    Other,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// The subset of a sway tree node that swayling needs. All other fields are
/// skipped while parsing. Names borrow from the JSON buffer, so window
/// titles are not copied.
#[derive(Deserialize)]
pub struct Node<'a> {
    pub id: i64,
    #[serde(rename = "type")]
    pub node_type: NodeType,
    #[serde(borrow)]
    pub name: Option<Cow<'a, str>>,
    pub layout: Layout,
    pub rect: Rect,
    pub focused: bool,
    pub fullscreen_mode: Option<u8>,
    /// Only set for windows: whether it is on screen (false e.g. for
    /// background tabs).
    #[serde(default)]
    pub visible: bool,
    // Only tiled children; floating ones live in `floating_nodes`, which we
    // never look at, so floating windows are skipped automatically.
    #[serde(borrow, default)]
    pub nodes: Vec<Node<'a>>,
}

impl Node<'_> {
    fn is_window(&self) -> bool {
        self.node_type == NodeType::Con && self.nodes.is_empty()
    }
}

/// The focused tiled window together with its surroundings in the tree.
pub struct Focused<'a> {
    pub window: &'a Node<'a>,
    pub parent: &'a Node<'a>,
    pub workspace: &'a Node<'a>,
}

/// Finds the focused window if it is a tiled leaf on a regular workspace.
pub fn find_focused_tiled<'a>(root: &'a Node<'a>) -> Option<Focused<'a>> {
    fn walk<'a>(
        node: &'a Node<'a>,
        parent: &'a Node<'a>,
        workspace: Option<&'a Node<'a>>,
    ) -> Option<Focused<'a>> {
        let workspace = if node.node_type == NodeType::Workspace {
            Some(node)
        } else {
            workspace
        };

        if node.focused {
            // Only leaf windows; a focused parent container (`focus parent`)
            // is left alone.
            if !node.is_window() {
                return None;
            }
            let workspace = workspace?;
            if workspace.name.as_deref() == Some(SCRATCHPAD) {
                return None;
            }
            return Some(Focused {
                window: node,
                parent,
                workspace,
            });
        }

        node.nodes
            .iter()
            .find_map(|child| walk(child, node, workspace))
    }

    root.nodes.iter().find_map(|child| walk(child, root, None))
}

/// All tiled windows below `node`, in tree order.
pub fn windows<'a>(node: &'a Node<'a>) -> Vec<&'a Node<'a>> {
    fn walk<'a>(node: &'a Node<'a>, out: &mut Vec<&'a Node<'a>>) {
        if node.is_window() {
            out.push(node);
        }
        for child in &node.nodes {
            walk(child, out);
        }
    }
    let mut out = Vec::new();
    walk(node, &mut out);
    out
}

/// Tiled windows below `node` that are actually on screen (background tabs
/// of tabbed/stacked containers are left out).
pub fn visible_windows<'a>(node: &'a Node<'a>) -> Vec<&'a Node<'a>> {
    let mut out = windows(node);
    out.retain(|w| w.visible);
    out
}

/// Finds the workspace called `name`.
pub fn find_workspace<'a>(root: &'a Node<'a>, name: &str) -> Option<&'a Node<'a>> {
    root.nodes
        .iter()
        .flat_map(|output| &output.nodes)
        .find(|ws| ws.node_type == NodeType::Workspace && ws.name.as_deref() == Some(name))
}

#[cfg(test)]
pub(crate) const SAMPLE_TREE: &[u8] = br#"{"id":1,"type":"root","name":"root","layout":"splith","rect":{"x":0,"y":0,"width":1920,"height":1080},"focused":false,"nodes":[
    {"id":2,"type":"output","name":"DP-1","layout":"output","rect":{"x":0,"y":0,"width":1920,"height":1080},"focused":false,"nodes":[
        {"id":3,"type":"workspace","name":"1","layout":"splith","rect":{"x":0,"y":0,"width":1920,"height":1080},"focused":false,"floating_nodes":[],"nodes":[
            {"id":4,"type":"con","name":"term","layout":"none","rect":{"x":0,"y":0,"width":960,"height":1080},"focused":true,"visible":true,"fullscreen_mode":0,"marks":[],"nodes":[]},
            {"id":5,"type":"con","name":null,"layout":"tabbed","rect":{"x":960,"y":0,"width":960,"height":1080},"focused":false,"nodes":[
                {"id":6,"type":"con","name":"a","layout":"none","rect":{"x":960,"y":30,"width":960,"height":1050},"focused":false,"visible":false,"fullscreen_mode":0,"nodes":[]},
                {"id":7,"type":"con","name":"b \"quoted\"","layout":"none","rect":{"x":960,"y":30,"width":960,"height":1050},"focused":false,"visible":true,"fullscreen_mode":0,"nodes":[]}
            ]}
        ]}
    ]}
]}"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(nodes: &[&Node]) -> Vec<i64> {
        nodes.iter().map(|n| n.id).collect()
    }

    #[test]
    fn finds_focused_window() {
        let tree: Node = serde_json::from_slice(SAMPLE_TREE).unwrap();
        let focused = find_focused_tiled(&tree).unwrap();
        assert_eq!(focused.window.id, 4);
        assert_eq!(focused.parent.id, 3);
        assert_eq!(focused.parent.layout, Layout::SplitH);
        assert_eq!(focused.workspace.name.as_deref(), Some("1"));
    }

    #[test]
    fn lists_all_and_visible_windows() {
        let tree: Node = serde_json::from_slice(SAMPLE_TREE).unwrap();
        let ws = find_focused_tiled(&tree).unwrap().workspace;
        assert_eq!(ids(&windows(ws)), [4, 6, 7]);
        // Only the active tab (7) of the tabbed container is visible.
        assert_eq!(ids(&visible_windows(ws)), [4, 7]);
    }

    #[test]
    fn names_with_escapes_still_parse() {
        let tree: Node = serde_json::from_slice(SAMPLE_TREE).unwrap();
        let ws = find_workspace(&tree, "1").unwrap();
        assert_eq!(ws.id, 3);
        assert_eq!(windows(ws)[2].name.as_deref(), Some("b \"quoted\""));
        assert!(find_workspace(&tree, "2").is_none());
    }
}
