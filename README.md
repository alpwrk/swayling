# swayling

Hyprland-style **dwindle tiling** for [sway](https://swaywm.org) and
[SwayFX](https://github.com/WillPower3309/swayfx).

Out of the box, sway keeps putting new windows next to each other in the same
direction until you manually press `split`. swayling is a tiny daemon that
talks to sway over IPC and does the splitting for you, so every new window
halves the focused one along its longer side, the same way Hyprland's dwindle
layout works:

```
┌───────────────┐   ┌───────┬───────┐   ┌───────┬───────┐   ┌───────┬───────┐
│               │   │       │       │   │       │   2   │   │       │   2   │
│       1       │ → │   1   │   2   │ → │   1   ├───────┤ → │   1   ├───┬───┤
│               │   │       │       │   │       │   3   │   │       │ 3 │ 4 │
└───────────────┘   └───────┴───────┘   └───────┴───────┘   └───────┴───┴───┘
```

No compositor patches are involved: swayling only sends ordinary
`split h` / `split v` commands, so it works with any sway or SwayFX version
that has IPC (tested on SwayFX 0.6 / sway 1.12).

## Footprint

swayling is built to stay out of the way:

- ~380 KB binary (links only libc), single thread, ~2.4 MB RSS
- 0% CPU while idle: it blocks on the IPC socket until sway sends an event
- about 50 µs of CPU per focus change, including fetching the layout tree
  (measured: 400 focus changes cost 20 ms in total)
- its own minimal IPC client; the only dependencies are `serde` and `serde_json`
- noisy events such as title changes are dismissed after reading a single
  field; the tree is only fetched on focus/new/move/floating, only the
  handful of fields that matter are parsed, and window titles are not even
  copied out of the receive buffer

## How it works

Whenever a tiled window gains focus (or is created, moved, or switched
between floating and tiled), swayling looks at its size:

- `width > height × ratio` → `split h` (the next window opens to the right)
- otherwise → `split v` (the next window opens below)

It leaves things alone when:

- the window is floating or fullscreen,
- its parent container is **tabbed** or **stacked** (your manual layouts are respected),
- a parent container is focused instead of a window (`focus parent`),
- the window is in the scratchpad,
- the window is already split the right way.

If the parent already has the target orientation but contains other windows,
swayling still splits, so the new window halves the focused one instead of
squeezing in as a third column. That is what makes the result a real dwindle
spiral, for example on ultrawide monitors.

## Requirements

- sway or SwayFX, running (swayling connects through `$SWAYSOCK`)
- A Rust toolchain, Rust 1.88 or newer, plus a C linker (`cc`, e.g. gcc).
  `install.sh` checks both and offers to install them (see below).
  Install it with [rustup](https://rustup.rs) or your distribution's package:

  ```sh
  # Arch / CachyOS
  sudo pacman -S rust
  # or
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```

## Building

```sh
git clone https://github.com/alpwrk/swayling.git
cd swayling

# Build an optimized binary at target/release/swayling
cargo build --release

# Optional: run the unit tests
cargo test
```

## Installing

### Quick install (recommended)

```sh
./install.sh --dry-run   # optional: show what would change in your config
./install.sh
```

The script:

1. checks the requirements: the usual shell tools, `sway`/`swaymsg`, Rust
   (at least the `rust-version` from `Cargo.toml`, currently 1.88) and a C
   linker. If Rust is missing it offers to install it with your package
   manager (pacman, xbps, apk, dnf, zypper, portage) or with rustup, which
   needs no root. On Debian/Ubuntu it goes straight to rustup, since their
   packaged Rust is too old. A too-old Rust is updated with
   `rustup update stable`. It always asks before installing anything;
   without a terminal it installs nothing (unless you pass `--yes`) and tells
   you what to run instead.
   `./install.sh --check` only runs this step,
2. builds swayling (`cargo build --release`),
3. installs the binary to `~/.local/bin/swayling` (change with `--bin-dir`),
4. finds your sway config: it asks the running sway which file it loaded,
   otherwise it uses sway's lookup order (`~/.sway/config`,
   `~/.config/sway/config`, ...). Override with `--config FILE`,
5. edits **only** these places in it:
   - `bindsym <key> move left|right|up|down` becomes
     `bindsym <key> exec ~/.local/bin/swayling move <dir>`. Other move
     commands such as `move left 20px` or `move container to workspace`
     are left alone,
   - a marked block at the end binds `swayling retile` to `$mod+Shift+t`
     (skipped with a warning if that key is already taken; change it with
     `--retile-key`) and, with the default autostart method, starts the daemon,
   - a hand-written `exec swayling` line is replaced by the one in the block,
     so the daemon never runs twice,
6. sets up **autostart** with whatever your system supports (see below),
7. writes a backup (`config.bak-YYYYMMDD_HHMMSS`), checks the new config with
   `sway --validate` and only then replaces your config. If validation fails,
   your config is not touched,
8. reloads sway if it is running, so the new bindings and the daemon are
   active right away (skip with `--no-reload`), and checks that the daemon
   really runs. Reloading does not log you out.

### Autostart

swayling has no dependency on systemd or any other init system: it only talks
to sway's IPC socket. The install script picks the autostart method on its
own (`--autostart auto`, the default):

| Your setup | Method |
| --- | --- |
| sway runs in a systemd graphical session (uwsm, sway-systemd, ...): `graphical-session.target` or `sway-session.target` is active and `SWAYSOCK` is exported to systemd | systemd user service `~/.config/systemd/user/swayling.service`, enabled for that target |
| everything else: no systemd at all (OpenRC, runit, s6, dinit, ... on Artix, Void, Gentoo, Alpine), or systemd but sway started by a display manager such as ly/greetd/SDDM or from a tty | `exec_always` in the sway config |

`exec_always` works on every system, because sway itself starts swayling and
it comes and goes with your sway session. Force a method with
`--autostart sway` or `--autostart systemd`. Switching methods later
removes the old one, so the daemon is never started twice.

Running it again is safe: it updates the block and the paths and changes
nothing else. Symlinked configs (dotfile managers) keep their symlink.

If sway was not running during the install (e.g. you ran it from a tty),
swayling starts with your next sway session.

| Option | Description |
| --- | --- |
| `--bin-dir DIR` | Install location, default `~/.local/bin` |
| `--config FILE` | sway config to edit, default: autodetected |
| `--retile-key KEY` | Key for `swayling retile`, default `$mod+Shift+t` |
| `--ratio RATIO` | Pass `--ratio` to the daemon and to `retile` |
| `--autostart MODE` | `auto` (default), `sway` or `systemd`, see [Autostart](#autostart) |
| `--no-retile` | Don't add a retile binding |
| `--no-move-binds` | Don't rewrite your `move` bindings |
| `--skip-build` | Use the existing `target/release/swayling` |
| `--no-reload` | Don't reload sway at the end (the daemon is still started) |
| `--dry-run` | Only print a diff of the config changes |
| `--check` | Only check the requirements |
| `-y`, `--yes` | Answer yes to every question, e.g. installing Rust |
| `--uninstall` | Undo everything, see [Uninstalling](#uninstalling) |

### Manual install

```sh
cargo install --path .
```

This puts the binary in `~/.cargo/bin/swayling`. Make sure that directory is
on your `PATH`, or use the full path in your sway config as shown below.
You can also copy `target/release/swayling` anywhere you like.

## Using it with sway

(`install.sh` does this for you.) Add this line to `~/.config/sway/config`:

```
exec_always "pkill -x swayling; exec ~/.cargo/bin/swayling"
```

`exec_always` together with `pkill` ensures that exactly one instance runs,
even after `swaymsg reload`. Keep the quotes: without them sway splits the
line at `;` into two commands, and the `exec` part would only run when sway
starts, not on reload. Then reload sway (`$mod+Shift+c` by default) or
log out and back in.

To try it out without touching your config, run it in a terminal:

```sh
swayling --verbose
```

Open a few terminals on an empty workspace and watch them form a spiral.
Press `Ctrl+C` to stop it; windows that are already open keep their layout.

## Moving windows and retiling

sway's own `move left/right/up/down` doesn't know about the spiral: it pulls
windows out of their containers and can leave you with flat rows or extra
columns. swayling ships two one-shot commands for that, meant to be bound to
keys. They talk to sway directly, so the daemon doesn't need to be involved.

### `swayling move <left|right|up|down>`

Like Hyprland's `movewindow` in dwindle: the focused window **swaps places**
with its neighbour in that direction, so the spiral stays intact and focus
stays on the window you moved. The neighbour is the closest visible window on
that side that overlaps the focused one (background tabs are ignored).

If there is no neighbour on the workspace, or the window is floating or
fullscreen, it falls back to sway's normal `move`, so moving a window to the
next monitor or nudging a floating window still works.

### `swayling retile [--ratio RATIO]`

Rebuilds the focused workspace as a clean dwindle spiral, keeping the windows'
current order. Use it when a workspace got messed up. Floating windows are
left alone; tabbed and stacked containers are dissolved into the spiral. Pass
the same `--ratio` you use for the daemon.

Internally the tiled windows are briefly parked on a temporary workspace and
put back one by one. sway runs all of it as a single command batch.

### Keybindings

Replace your `move` bindings in `~/.config/sway/config` and add one for
retiling, for example:

```
bindsym $mod+Shift+$left  exec swayling move left
bindsym $mod+Shift+$down  exec swayling move down
bindsym $mod+Shift+$up    exec swayling move up
bindsym $mod+Shift+$right exec swayling move right
bindsym $mod+Shift+Left   exec swayling move left
bindsym $mod+Shift+Down   exec swayling move down
bindsym $mod+Shift+Up     exec swayling move up
bindsym $mod+Shift+Right  exec swayling move right

bindsym $mod+Shift+t exec swayling retile
```

Use the full path (`~/.cargo/bin/swayling`) if `~/.cargo/bin` is not on the
`PATH` sway was started with.

## Options

| Option | Default | Description |
| --- | --- | --- |
| `-r`, `--ratio <RATIO>` | `1.0` | Split horizontally when `width > height × RATIO`. Higher values prefer vertical splits. Same idea as Hyprland's `split_width_multiplier`. |
| `-w`, `--workspaces <LIST>` | all | Only tile these workspaces, comma-separated names, e.g. `1,2,web`. |
| `-v`, `--verbose` | off | Print every split decision to stderr. |
| `move <DIRECTION>` | | Swap the focused window with its neighbour, see above. |
| `retile` | | Rebuild the focused workspace as a spiral; accepts `--ratio`. |
| `-h`, `--help` | | Show help. |
| `-V`, `--version` | | Show version. |

Example: only tile workspaces 1 to 3 and prefer stacking vertically a bit more:

```
exec_always "pkill -x swayling; exec ~/.cargo/bin/swayling --ratio 1.3 --workspaces 1,2,3"
```

## Troubleshooting

- **Nothing happens.** Check that it is running (`pgrep -a swayling`) and
  start it manually with `--verbose` to see its decisions. If it reports
  `cannot connect to sway IPC`, `$SWAYSOCK` is not set in that environment.
  Launching it from the sway config always works.
- **A workspace doesn't tile.** It may be excluded by `--workspaces`, or the
  windows are inside a tabbed/stacked container. Switch it back with
  `layout toggle split`.
- **Conflicts with other autotiling tools.** Don't run swayling together with
  `autotiling`, `autotiling-rs` or similar scripts, since they fight over the
  same split commands.
- **Logs.** sway sends a program's stderr to its own log. Run swayling
  manually with `--verbose` for easier debugging.

- **`move` does nothing.** Without a neighbour in that direction swayling
  falls back to sway's own `move`; on a single monitor at the screen edge that
  usually has no visible effect, the same as in Hyprland.

When sway exits, swayling exits too. If the IPC connection drops while sway
is still running, it tries to reconnect 10 times (every 500 ms) before giving
up.

## Uninstalling

If you used the install script:

```sh
./install.sh --uninstall
```

This turns the `swayling move` bindings back into plain `move` bindings,
removes the managed block (again with a backup and validation first),
disables and deletes the systemd service if the script created one, stops
the daemon and deletes the binary. Then reload sway.

After a manual install, run `cargo uninstall swayling` and remove the
swayling lines from your sway config.
