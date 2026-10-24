# swayling

Hyprland-style **dwindle tiling** for [sway](https://swaywm.org) and
[SwayFX](https://github.com/WillPower3309/swayfx).

Out of the box, sway keeps putting new windows next to each other in the same
direction until you manually press `split`. swayling is a tiny daemon that
talks to sway over IPC and does the splitting for you: every new window
halves the focused one along its longer side, just like Hyprland's dwindle
layout.

```
┌───────────────┐   ┌───────┬───────┐   ┌───────┬───────┐   ┌───────┬───────┐
│               │   │       │       │   │       │   2   │   │       │   2   │
│       1       │ → │   1   │   2   │ → │   1   ├───────┤ → │   1   ├───┬───┤
│               │   │       │       │   │       │   3   │   │       │ 3 │ 4 │
└───────────────┘   └───────┴───────┘   └───────┴───────┘   └───────┴───┴───┘
```

- **Dwindle tiling**: new windows spiral in automatically
- **`swayling move`**: moving a window swaps it with its neighbour instead of
  breaking the layout, like Hyprland's `movewindow`
- **`swayling retile`**: turns a messed-up workspace back into a clean spiral
- **Lightweight**: ~380 KB, ~2.4 MB RAM, 0% CPU while idle
- **No patches, no init system required**: only ordinary sway IPC commands.
  Works with any sway or SwayFX version (tested on SwayFX 0.6 / sway 1.12),
  with or without systemd
- **One-command install** that detects your config and wires everything up

## Quick start

```sh
git clone https://github.com/alpwrk/swayling.git
cd swayling
./install.sh
```

That's it: the script checks for Rust, builds swayling, installs it, wires it
into your sway config and reloads sway (this does not log you out). Open a
few terminals on an empty workspace and watch them form a spiral. See
[The install script](#the-install-script) for details.

Want to see what it would change in your sway config first?

```sh
./install.sh --dry-run
```

## Usage

### The daemon

```sh
swayling [--ratio RATIO] [--workspaces LIST] [--verbose]
```

Runs in the background and prepares the split direction of whatever window
is focused. It is normally started by `exec_always` in your sway config (the
install script sets this up). Run it by hand with `--verbose`
to watch its decisions.

| Option | Default | Description |
| --- | --- | --- |
| `-r`, `--ratio <RATIO>` | `1.0` | Split horizontally when `width > height × RATIO`, otherwise vertically. Higher values prefer vertical splits. Same idea as Hyprland's `split_width_multiplier`. |
| `-w`, `--workspaces <LIST>` | all | Only tile these workspaces, comma-separated names, e.g. `1,2,web`. |
| `-v`, `--verbose` | off | Print every split decision to stderr. |
| `-h`, `--help` | | Show help. |
| `-V`, `--version` | | Show version. |

### `swayling move <left|right|up|down>`

sway's own `move` doesn't know about the spiral: it pulls windows out of
their containers and can leave you with flat rows or extra columns.
`swayling move` instead **swaps** the focused window with its neighbour in
that direction, so the spiral stays intact and focus stays on the window you
moved. The neighbour is the closest visible window on that side that
overlaps the focused one (background tabs are ignored).

Without a neighbour on the workspace, or for floating and fullscreen
windows, it falls back to sway's normal `move`, so moving a window to the
next monitor or nudging a floating window still works.

### `swayling retile [--ratio RATIO]`

Rebuilds the focused workspace as a clean spiral, keeping the windows'
current order. Floating windows are left alone; tabbed and stacked
containers are dissolved into the spiral. Pass the same `--ratio` as the
daemon. Internally the tiled windows are briefly parked on a temporary
workspace and put back one by one, all in a single sway command batch.

Both commands talk to sway directly and work whether the daemon runs or not.

### Keybindings

The install script sets these up. To do it by hand, replace your `move`
bindings and add one for retiling:

```
bindsym $mod+Shift+$left  exec swayling move left
bindsym $mod+Shift+$down  exec swayling move down
bindsym $mod+Shift+$up    exec swayling move up
bindsym $mod+Shift+$right exec swayling move right

bindsym $mod+Shift+t exec swayling retile
```

Use the full path to the binary if its directory is not on the `PATH` sway
was started with.

## The install script

```sh
./install.sh [OPTIONS]
```

It runs these steps and stops at the first problem, before touching anything
it can't undo:

1. **Checks the requirements**: the usual shell tools, `sway`/`swaymsg`, Rust
   (at least the `rust-version` from `Cargo.toml`, currently 1.88) and a C
   linker (`cc`). If Rust is missing it offers to install it with your
   package manager (pacman, xbps, apk, dnf, zypper, portage, using sudo or
   doas) or with rustup, which needs no root. On Debian/Ubuntu it goes
   straight to rustup, since their packaged Rust is too old. A too-old Rust
   is updated with `rustup update stable`. It always asks first; without a
   terminal it installs nothing (unless you pass `--yes`) and tells you what
   to run instead.
2. **Builds** swayling with `cargo build --release`.
3. **Installs** the binary to `~/.local/bin/swayling`. A running daemon keeps
   working, because the file is replaced, not overwritten.
4. **Finds your sway config**: it asks the running sway which file it loaded,
   otherwise it uses sway's lookup order (`~/.sway/config`,
   `~/.config/sway/config`, ...).
5. **Edits only these places** in it:
   - `bindsym <key> move left|right|up|down` becomes
     `bindsym <key> exec ~/.local/bin/swayling move <dir>`. Other move
     commands such as `move left 20px` or `move container to workspace`
     are left alone.
   - A marked block at the end binds `swayling retile` to `$mod+Shift+t`
     (skipped with a warning if that key is already taken) and starts the
     daemon with `exec_always` (see [Autostart](#autostart)).
   - A hand-written `exec swayling` line is replaced by the managed one, so
     the daemon never runs twice.
6. **Writes a backup** (`config.bak-YYYYMMDD_HHMMSS`), checks the new config
   with `sway --validate` and only then replaces yours. If validation fails,
   your config is not touched.
7. **Removes an old systemd service** that earlier versions of the script
   may have set up, so the daemon never runs twice.
8. **Reloads sway** if it is running, so the bindings and the daemon are
   active right away, and checks that the daemon really runs. Then it
   celebrates.

Running it again is safe: it only updates the managed parts. Symlinked
configs (dotfile managers) keep their symlink.

### Autostart

The daemon is always started by sway itself, with this line in the managed
block:

```
exec_always "pkill -x swayling; exec ~/.local/bin/swayling"
```

So it runs exactly as long as your sway session does, and it always sees the
right `SWAYSOCK`. This works the same everywhere: with or without systemd
(OpenRC, runit, s6, dinit, ...), with uwsm, a display manager or a tty. On
every reload the old daemon is replaced, so new options and a freshly
installed binary take effect right away.

### Options

| Option | Description |
| --- | --- |
| `--bin-dir DIR` | Install location, default `~/.local/bin` |
| `--config FILE` | sway config to edit, default: autodetected |
| `--retile-key KEY` | Key for `swayling retile`, default `$mod+Shift+t` |
| `--ratio RATIO` | Pass `--ratio` to the daemon and to `retile` |
| `--no-retile` | Don't add a retile binding |
| `--no-move-binds` | Don't rewrite your `move` bindings |
| `--skip-build` | Use the existing `target/release/swayling` |
| `--no-reload` | Don't reload sway at the end (the daemon is still started; new bindings apply on your next reload) |
| `--dry-run` | Only print a diff of the config changes |
| `--check` | Only check the requirements |
| `-y`, `--yes` | Answer yes to every question, e.g. installing Rust |
| `--uninstall` | Undo everything, see [Uninstalling](#uninstalling) |

### Manual install

If you'd rather not use the script:

```sh
cargo build --release      # binary at target/release/swayling
cargo install --path .     # or install it to ~/.cargo/bin
```

Then add the keybindings above and this line to your sway config:

```
exec_always "pkill -x swayling; exec ~/.cargo/bin/swayling"
```

Keep the quotes: without them sway splits the line at `;` into two commands,
and the `exec` part would only run when sway starts, not on reload, so every
reload would kill the daemon for good.

## How it works

sway places a new window next to the focused one, using the layout of the
focused window's parent. So swayling prepares the split *before* a window
opens: whenever a tiled window gains focus (or is created, moved, or switched
between floating and tiled), it looks at the window's size:

- `width > height × ratio` → `split h` (the next window opens to the right)
- otherwise → `split v` (the next window opens below)

It leaves things alone when the window is floating or fullscreen, when its
parent is **tabbed** or **stacked** (your manual layouts are respected), when
a parent container is focused (`focus parent`), when the window is in the
scratchpad, or when it is already split the right way.

If the parent already has the target orientation but contains other
windows, swayling still splits, so the new window halves the focused one
instead of squeezing in as a third column. That is what makes the result a
real dwindle spiral, for example on ultrawide monitors.

When sway exits, swayling exits too. If the connection drops while sway is
still running, it reconnects (10 attempts, every 500 ms).

### Footprint

- ~380 KB binary (links only libc), one thread, ~2.4 MB RSS
- 0% CPU while idle: it blocks on the IPC socket until sway sends an event
- about 50 µs of CPU per focus change, including fetching the layout tree
  (measured: 400 focus changes cost 20 ms in total)
- its own minimal IPC client; the only dependencies are `serde` and
  `serde_json`
- noisy events such as title changes are dismissed after reading a single
  field; only the handful of tree fields that matter are parsed, and window
  titles are not even copied out of the receive buffer

## Troubleshooting

- **Nothing happens.** Check that the daemon runs: `pgrep -a swayling`.
  Start it by hand with `swayling --verbose` to see its decisions. If it
  reports `cannot connect to sway IPC`, `$SWAYSOCK` is not set in that
  environment; starting it from the sway config always works.
- **The daemon is gone after a reload.** Your config has an unquoted
  `exec_always pkill -x swayling; exec ...` line (see
  [Manual install](#manual-install)). Run `./install.sh` again, it fixes the
  line.
- **A workspace doesn't tile.** It may be excluded by `--workspaces`, or the
  windows are inside a tabbed/stacked container. Switch back with
  `layout toggle split`, or use `swayling retile`.
- **`move` does nothing.** Without a neighbour in that direction swayling
  falls back to sway's own `move`; at the screen edge with a single monitor
  that has no visible effect, the same as in Hyprland.
- **Conflicts with other autotiling tools.** Don't run swayling together with
  `autotiling`, `autotiling-rs` or similar scripts, since they fight over the
  same split commands.
- **Logs.** sway sends a program's stderr to its own log.

## Uninstalling

```sh
./install.sh --uninstall
```

This turns the `swayling move` bindings back into plain `move` bindings,
removes the managed block (with a backup and validation first), disables and
deletes an old systemd service if an earlier version created one, stops the
daemon, deletes the binary and reloads sway.

After a manual install, run `cargo uninstall swayling` and remove the
swayling lines from your sway config.

## Development

```sh
cargo test                    # unit tests
cargo clippy --all-targets    # lints
```

| File | Purpose |
| --- | --- |
| `src/main.rs` | CLI parsing, daemon event loop, reconnect handling |
| `src/ipc.rs` | Minimal sway IPC client |
| `src/tree.rs` | The parts of sway's layout tree swayling parses |
| `src/dwindle.rs` | Split decision for the focused window |
| `src/commands.rs` | `move` and `retile` |
| `install.sh` | Requirements check, build, install and config setup |
