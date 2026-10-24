//! One-shot commands meant to be bound to keys: `move` and `retile`.

use std::fmt::Write as _;
use std::io;

use crate::dwindle::Split;
use serde::Deserialize;

use crate::ipc::{GET_TREE, GET_WORKSPACES, Ipc};
use crate::tree::{Node, Rect, find_focused_tiled, find_workspace, visible_windows, windows};

/// Windows closer than this (in px) to the edge still count as neighbours,
/// to tolerate rounding in sway's layout.
const EDGE_TOLERANCE: i32 = 2;

/// Temporary workspace used while rebuilding a workspace.
const RETILE_WORKSPACE: &str = "__swayling_retile";
const RETILE_MARK: &str = "_swayling_retile";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "left" => Some(Direction::Left),
            "right" => Some(Direction::Right),
            "up" => Some(Direction::Up),
            "down" => Some(Direction::Down),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Direction::Left => "left",
            Direction::Right => "right",
            Direction::Up => "up",
            Direction::Down => "down",
        }
    }
}

/// Finds the window next to `from` in direction `dir`: the closest one on
/// that side that overlaps `from` on the other axis. Ties go to the largest
/// overlap, then to the top/left-most window.
pub fn neighbor(
    from: Rect,
    candidates: impl IntoIterator<Item = (i64, Rect)>,
    dir: Direction,
) -> Option<i64> {
    candidates
        .into_iter()
        .filter_map(|(id, c)| {
            let (gap, overlap, position) = match dir {
                Direction::Left => (
                    from.x - (c.x + c.width),
                    overlap(from.y, from.height, c.y, c.height),
                    c.y,
                ),
                Direction::Right => (
                    c.x - (from.x + from.width),
                    overlap(from.y, from.height, c.y, c.height),
                    c.y,
                ),
                Direction::Up => (
                    from.y - (c.y + c.height),
                    overlap(from.x, from.width, c.x, c.width),
                    c.x,
                ),
                Direction::Down => (
                    c.y - (from.y + from.height),
                    overlap(from.x, from.width, c.x, c.width),
                    c.x,
                ),
            };
            (gap >= -EDGE_TOLERANCE && overlap > 0).then_some((gap, -overlap, position, id))
        })
        .min()
        .map(|(.., id)| id)
}

fn overlap(a: i32, a_len: i32, b: i32, b_len: i32) -> i32 {
    (a + a_len).min(b + b_len) - a.max(b)
}

/// Swaps the focused tiled window with its neighbour in `dir`, so the
/// dwindle structure stays intact. Without a neighbour on the workspace (or
/// for floating/fullscreen windows) it falls back to sway's own `move`, which
/// e.g. moves the window to the next output.
pub fn move_window(dir: Direction) -> io::Result<()> {
    let mut ipc = Ipc::connect()?;
    let mut tree_buf = Vec::new();
    ipc.request(GET_TREE, b"", &mut tree_buf)?;
    let tree: Node = serde_json::from_slice(&tree_buf)?;

    let command = match find_focused_tiled(&tree) {
        Some(f) if f.window.fullscreen_mode.unwrap_or(0) == 0 => {
            let id = f.window.id;
            let candidates = visible_windows(f.workspace)
                .into_iter()
                .filter(|w| w.id != id)
                .map(|w| (w.id, w.rect));
            match neighbor(f.window.rect, candidates, dir) {
                // Swapping can move focus to the other window; keep it on
                // the one that was moved.
                Some(other) => {
                    format!("[con_id={id}] swap container with con_id {other}; [con_id={id}] focus")
                }
                None => format!("[con_id={id}] move {}", dir.name()),
            }
        }
        _ => format!("move {}", dir.name()),
    };
    ipc.run_command(&command, &mut Vec::new())
}

#[derive(Deserialize)]
struct WorkspaceInfo<'a> {
    #[serde(borrow)]
    name: std::borrow::Cow<'a, str>,
    focused: bool,
}

/// Quotes a workspace name for use in a sway command.
fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Builds the sway commands that rebuild `workspace` as a dwindle spiral of
/// `windows` (in this order) and restore focus afterwards.
///
/// All windows are parked on a temporary workspace first. Then they are put
/// back one by one: each previous window is split along the longer side of
/// the area it will occupy and the next window is moved right next to it.
pub fn retile_commands(
    workspace: &str,
    area: Rect,
    windows: &[i64],
    focused: Option<i64>,
    ratio: f32,
) -> String {
    let mut cmd = String::new();
    let Some((&first, rest)) = windows.split_first() else {
        return cmd;
    };
    let tmp = quote(RETILE_WORKSPACE);
    for id in windows {
        let _ = write!(cmd, "[con_id={id}] move container to workspace {tmp}; ");
    }
    let _ = write!(
        cmd,
        "[con_id={first}] move container to workspace {}; ",
        quote(workspace)
    );

    let mut area = area;
    let mut prev = first;
    for &id in rest {
        let split = Split::for_size(area.width, area.height, ratio);
        let _ = write!(
            cmd,
            "[con_id={prev}] mark --add {RETILE_MARK}; [con_id={prev}] split {}; [con_id={id}] move container to mark {RETILE_MARK}; ",
            split.command_arg()
        );
        // The new window takes the second half of the previous one's area.
        area = match split {
            Split::Horizontal => Rect {
                x: area.x + area.width / 2,
                width: area.width - area.width / 2,
                ..area
            },
            Split::Vertical => Rect {
                y: area.y + area.height / 2,
                height: area.height - area.height / 2,
                ..area
            },
        };
        prev = id;
    }
    let _ = write!(cmd, "unmark {RETILE_MARK}");
    if let Some(id) = focused {
        let _ = write!(cmd, "; [con_id={id}] focus");
    }
    cmd
}

/// Rebuilds the focused workspace as a dwindle spiral. Floating windows are
/// left alone; tabbed/stacked containers are dissolved into the spiral.
pub fn retile(ratio: f32) -> io::Result<()> {
    let mut ipc = Ipc::connect()?;
    let mut ws_buf = Vec::new();
    ipc.request(GET_WORKSPACES, b"", &mut ws_buf)?;
    let infos: Vec<WorkspaceInfo> = serde_json::from_slice(&ws_buf)?;
    let Some(info) = infos.iter().find(|w| w.focused) else {
        return Ok(());
    };

    let mut tree_buf = Vec::new();
    ipc.request(GET_TREE, b"", &mut tree_buf)?;
    let tree: Node = serde_json::from_slice(&tree_buf)?;
    let Some(workspace) = find_workspace(&tree, &info.name) else {
        return Ok(());
    };
    let ids: Vec<i64> = windows(workspace).iter().map(|w| w.id).collect();
    if ids.len() < 2 {
        return Ok(());
    }
    let focused = find_focused_tiled(&tree).map(|f| f.window.id);
    let name = workspace.name.as_deref().unwrap_or_default();
    let command = retile_commands(name, workspace.rect, &ids, focused, ratio);
    ipc.run_command(&command, &mut Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: i32, height: i32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    // Layout used below (1920x1080, 6px gaps):
    // ┌──────┬──────┐
    // │      │  2   │
    // │  1   ├──┬───┤
    // │      │3 │ 4 │
    // └──────┴──┴───┘
    fn spiral() -> Vec<(i64, Rect)> {
        vec![
            (1, rect(0, 0, 954, 1080)),
            (2, rect(960, 0, 960, 537)),
            (3, rect(960, 543, 477, 537)),
            (4, rect(1443, 543, 477, 537)),
        ]
    }

    fn find(id: i64) -> Rect {
        spiral().into_iter().find(|(i, _)| *i == id).unwrap().1
    }

    fn others(id: i64) -> Vec<(i64, Rect)> {
        spiral().into_iter().filter(|(i, _)| *i != id).collect()
    }

    #[test]
    fn finds_neighbours_in_each_direction() {
        assert_eq!(neighbor(find(1), others(1), Direction::Right), Some(2));
        assert_eq!(neighbor(find(2), others(2), Direction::Down), Some(3));
        assert_eq!(neighbor(find(2), others(2), Direction::Left), Some(1));
        assert_eq!(neighbor(find(3), others(3), Direction::Right), Some(4));
        assert_eq!(neighbor(find(4), others(4), Direction::Up), Some(2));
        assert_eq!(neighbor(find(4), others(4), Direction::Left), Some(3));
    }

    #[test]
    fn no_neighbour_at_the_edge() {
        assert_eq!(neighbor(find(1), others(1), Direction::Left), None);
        assert_eq!(neighbor(find(2), others(2), Direction::Up), None);
        assert_eq!(neighbor(find(4), others(4), Direction::Right), None);
    }

    #[test]
    fn prefers_largest_overlap_on_equal_distance() {
        // Both touch window 1 at the same distance; 3 overlaps it more.
        let candidates = vec![(2, rect(960, 0, 960, 300)), (3, rect(960, 306, 960, 774))];
        assert_eq!(neighbor(find(1), candidates, Direction::Right), Some(3));
    }

    #[test]
    fn builds_spiral_commands() {
        let cmd = retile_commands(
            "web \"x\"",
            rect(0, 0, 1920, 1080),
            &[10, 11, 12],
            Some(11),
            1.0,
        );
        assert_eq!(
            cmd,
            "[con_id=10] move container to workspace \"__swayling_retile\"; \
             [con_id=11] move container to workspace \"__swayling_retile\"; \
             [con_id=12] move container to workspace \"__swayling_retile\"; \
             [con_id=10] move container to workspace \"web \\\"x\\\"\"; \
             [con_id=10] mark --add _swayling_retile; [con_id=10] split h; [con_id=11] move container to mark _swayling_retile; \
             [con_id=11] mark --add _swayling_retile; [con_id=11] split v; [con_id=12] move container to mark _swayling_retile; \
             unmark _swayling_retile; [con_id=11] focus"
        );
    }

    #[test]
    fn single_window_needs_no_commands() {
        assert_eq!(
            retile_commands("1", rect(0, 0, 1920, 1080), &[], None, 1.0),
            ""
        );
    }
}
