//! Dwindle layout logic.
//!
//! sway places a new window next to the focused one, using the layout of the
//! focused window's parent container. To get Hyprland-style dwindle tiling we
//! therefore prepare the split direction *before* a window opens: whenever a
//! tiled window gains focus, we split it along its longer side.

use std::io;

use crate::ipc::{GET_TREE, Ipc};
use crate::tree::{Layout, Node, find_focused_tiled};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Split {
    Horizontal,
    Vertical,
}

impl Split {
    /// Splits along the longer side, see [`decide`] for `ratio`.
    pub fn for_size(width: i32, height: i32, ratio: f32) -> Self {
        if width as f32 > height as f32 * ratio {
            Split::Horizontal
        } else {
            Split::Vertical
        }
    }

    fn layout(self) -> Layout {
        match self {
            Split::Horizontal => Layout::SplitH,
            Split::Vertical => Layout::SplitV,
        }
    }

    pub fn command_arg(self) -> &'static str {
        match self {
            Split::Horizontal => "h",
            Split::Vertical => "v",
        }
    }
}

/// The facts about the focused window that the split decision depends on.
#[derive(Clone, Copy, Debug)]
pub struct Situation {
    pub width: i32,
    pub height: i32,
    pub fullscreen: bool,
    pub parent_layout: Layout,
    pub parent_children: usize,
}

/// Decides which split (if any) should be applied to the focused window.
///
/// `ratio` mirrors Hyprland's `split_width_multiplier`: a window is split
/// horizontally when `width > height * ratio`, otherwise vertically.
pub fn decide(s: Situation, ratio: f32) -> Option<Split> {
    if s.fullscreen || s.width <= 0 || s.height <= 0 {
        return None;
    }
    // Tabbed and stacked containers are deliberate user choices; leave them be.
    if matches!(s.parent_layout, Layout::Tabbed | Layout::Stacked) {
        return None;
    }

    let split = Split::for_size(s.width, s.height, ratio);

    // Already split the right way: the window is the only child of a
    // container with the target layout, so the next window halves it.
    if s.parent_layout == split.layout() && s.parent_children == 1 {
        return None;
    }

    // If the parent has the target layout but several children, we still
    // split: sway then wraps the window in its own container, so a new window
    // halves the focused one instead of squeezing in as an extra column/row.
    Some(split)
}

pub struct Dwindle {
    pub ratio: f32,
    pub workspaces: Option<Vec<String>>,
    pub verbose: bool,
}

impl Dwindle {
    /// Inspects the current tree and applies a split to the focused window if
    /// needed. `buf` is a scratch buffer reused across calls.
    pub fn apply(&self, ipc: &mut Ipc, buf: &mut Vec<u8>) -> io::Result<()> {
        ipc.request(GET_TREE, b"", buf)?;
        let tree: Node = serde_json::from_slice(buf)?;
        // The tree borrows from `buf`, so it has to be dropped before `buf`
        // is reused for the command.
        let Some(command) = self.plan(&tree) else {
            return Ok(());
        };
        drop(tree);
        ipc.run_command(&command, buf)
    }

    /// Returns the split command for the focused window, if it needs one.
    fn plan(&self, tree: &Node) -> Option<String> {
        let focused = find_focused_tiled(tree)?;

        let workspace = focused.workspace.name.as_deref().unwrap_or_default();
        if let Some(allowed) = &self.workspaces
            && !allowed.iter().any(|w| w == workspace)
        {
            return None;
        }

        let window = focused.window;
        let situation = Situation {
            width: window.rect.width,
            height: window.rect.height,
            fullscreen: window.fullscreen_mode.unwrap_or(0) != 0,
            parent_layout: focused.parent.layout,
            parent_children: focused.parent.nodes.len(),
        };
        let split = decide(situation, self.ratio)?;

        if self.verbose {
            eprintln!(
                "swayling: con {} ({}x{}) on workspace '{}' -> split {}",
                window.id,
                situation.width,
                situation.height,
                workspace,
                split.command_arg()
            );
        }
        Some(format!(
            "[con_id={}] split {}",
            window.id,
            split.command_arg()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn situation(
        width: i32,
        height: i32,
        parent_layout: Layout,
        parent_children: usize,
    ) -> Situation {
        Situation {
            width,
            height,
            fullscreen: false,
            parent_layout,
            parent_children,
        }
    }

    #[test]
    fn wide_window_splits_horizontally() {
        let s = situation(1920, 1080, Layout::SplitV, 2);
        assert_eq!(decide(s, 1.0), Some(Split::Horizontal));
    }

    #[test]
    fn tall_window_splits_vertically() {
        let s = situation(960, 1080, Layout::SplitH, 2);
        assert_eq!(decide(s, 1.0), Some(Split::Vertical));
    }

    #[test]
    fn already_split_only_child_is_skipped() {
        let s = situation(1920, 1080, Layout::SplitH, 1);
        assert_eq!(decide(s, 1.0), None);
    }

    #[test]
    fn same_layout_with_siblings_still_nests() {
        let s = situation(1720, 1440, Layout::SplitH, 2);
        assert_eq!(decide(s, 1.0), Some(Split::Horizontal));
    }

    #[test]
    fn tabbed_and_stacked_are_ignored() {
        assert_eq!(decide(situation(1920, 1080, Layout::Tabbed, 3), 1.0), None);
        assert_eq!(decide(situation(1920, 1080, Layout::Stacked, 3), 1.0), None);
    }

    #[test]
    fn fullscreen_is_ignored() {
        let mut s = situation(1920, 1080, Layout::SplitV, 2);
        s.fullscreen = true;
        assert_eq!(decide(s, 1.0), None);
    }

    #[test]
    fn ratio_shifts_the_threshold() {
        // 1200x1000 is wide at ratio 1.0, but not at ratio 1.5.
        let s = situation(1200, 1000, Layout::SplitV, 2);
        assert_eq!(decide(s, 1.0), Some(Split::Horizontal));
        let s = situation(1200, 1000, Layout::SplitH, 2);
        assert_eq!(decide(s, 1.5), Some(Split::Vertical));
    }
}
