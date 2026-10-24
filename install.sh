#!/usr/bin/env bash
# install.sh - build swayling, install it and wire it into your sway config.
#
# What it changes in your config, and nothing else:
#   * `bindsym ... move left|right|up|down` bindings become
#     `bindsym ... exec <bin>/swayling move <dir>`
#   * a marked block at the end binds `retile` and (unless a systemd user
#     service is used for autostart, see detect_autostart) starts the daemon
# A timestamped backup is written first and the result is checked with
# `sway --validate` before it replaces your config.

set -euo pipefail

BLOCK_BEGIN='# >>> swayling (managed by install.sh, edits inside are overwritten) >>>'
BLOCK_END='# <<< swayling <<<'
DIRS='left|right|up|down'

bin_dir="$HOME/.local/bin"
config=""
retile_key='$mod+Shift+t'
ratio=""
do_build=1
do_move=1
do_retile=1
do_reload=1
dry_run=0
assume_yes=0
check_only=0
uninstall=0
autostart=auto

usage() {
    cat <<'EOF'
Usage: ./install.sh [OPTIONS]

Builds swayling, installs it and updates your sway config.

Options:
  --bin-dir DIR       Where to install the binary [default: ~/.local/bin]
  --config FILE       sway config to edit [default: autodetected]
  --retile-key KEY    Key for `swayling retile` [default: $mod+Shift+t]
  --ratio RATIO       Pass --ratio to the daemon and to retile
  --autostart MODE    How to start the daemon: auto, sway (exec_always in the
                      sway config) or systemd (user service) [default: auto]
  --no-retile         Don't add a retile binding
  --no-move-binds     Don't rewrite your `move` bindings
  --skip-build        Use the existing target/release/swayling
  --no-reload         Don't reload sway at the end. By default a running sway
                      is reloaded so the new bindings and the daemon are active
                      right away (this does not log you out)
  --dry-run           Only show what would change in the config
  --uninstall         Undo everything: restore the move bindings, remove the
                      block and the binary
  --check             Only check the requirements and exit
  -y, --yes           Answer yes to every question (e.g. installing Rust)
  -h, --help          Show this help
EOF
}

die() { printf 'error: %s\n' "$*" >&2; exit 1; }
info() { printf '==> %s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }

while [[ $# -gt 0 ]]; do
    case "$1" in
        --bin-dir) bin_dir="${2:?--bin-dir needs a value}"; shift ;;
        --config) config="${2:?--config needs a value}"; shift ;;
        --retile-key) retile_key="${2:?--retile-key needs a value}"; shift ;;
        --ratio) ratio="${2:?--ratio needs a value}"; shift ;;
        --autostart) autostart="${2:?--autostart needs a value}"; shift ;;
        --no-retile) do_retile=0 ;;
        --no-move-binds) do_move=0 ;;
        --skip-build) do_build=0 ;;
        --reload) do_reload=1 ;;
        --no-reload) do_reload=0 ;;
        --dry-run) dry_run=1 ;;
        --uninstall) uninstall=1 ;;
        --check) check_only=1 ;;
        -y|--yes) assume_yes=1 ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown option '$1' (see --help)" ;;
    esac
    shift
done

case "$autostart" in
    auto|sway|systemd) ;;
    *) die "--autostart must be auto, sway or systemd" ;;
esac

repo_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
bin_dir="${bin_dir%/}"
bin="$bin_dir/swayling"

# --- Requirements ----------------------------------------------------------------

# Asks a yes/no question; "no" without a terminal (unless --yes).
ask() {
    if [[ $assume_yes -eq 1 ]]; then
        return 0
    fi
    [[ -t 0 ]] || return 1
    local answer
    read -r -p "$1 [y/N] " answer
    [[ "$answer" == [yY] || "$answer" == [yY][eE][sS] ]]
}

# Runs a command as root (sudo or doas), for package installs.
as_root() {
    if [[ $EUID -eq 0 ]]; then
        "$@"
    elif command -v sudo >/dev/null; then
        sudo "$@"
    elif command -v doas >/dev/null; then
        doas "$@"
    else
        die "need root to run '$*', but neither sudo nor doas is installed"
    fi
}

# Prints "<package manager> <rust install command> <C toolchain package>".
# Debian/Ubuntu ship old Rust releases, so rustup is preferred there.
detect_pkg_manager() {
    if command -v pacman >/dev/null; then echo "pacman|pacman -S --needed rust|base-devel"
    elif command -v xbps-install >/dev/null; then echo "xbps|xbps-install -S rust cargo|base-devel"
    elif command -v apk >/dev/null; then echo "apk|apk add rust cargo|build-base"
    elif command -v dnf >/dev/null; then echo "dnf|dnf install rust cargo|gcc"
    elif command -v zypper >/dev/null; then echo "zypper|zypper install rust cargo|gcc"
    elif command -v emerge >/dev/null; then echo "portage|emerge --ask=n dev-lang/rust-bin|sys-devel/gcc"
    elif command -v apt-get >/dev/null; then echo "apt||build-essential"
    else echo "unknown||"
    fi
}

# Minimum Rust version, read from Cargo.toml so it is defined only once.
rust_min="$(sed -n 's/^rust-version *= *"\(.*\)"/\1/p' "$repo_dir/Cargo.toml")"
rust_min="${rust_min:-1.88}"

# True if version $1 >= version $2 (major.minor.patch).
version_ge() {
    [[ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n 1)" == "$2" ]]
}

rust_version() {
    rustc --version 2>/dev/null | awk '{print $2}' | sed 's/-.*//'
}

install_rustup() {
    command -v curl >/dev/null || die "curl is needed to install rustup"
    info "installing Rust with rustup (https://rustup.rs)"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
}

# Makes sure cargo/rustc are usable and recent enough, offering to install
# or update them.
check_rust() {
    # rustup installs to ~/.cargo/bin, which may not be in PATH yet.
    if ! command -v cargo >/dev/null && [[ -x "$HOME/.cargo/bin/cargo" ]]; then
        export PATH="$HOME/.cargo/bin:$PATH"
    fi

    if ! command -v cargo >/dev/null || ! command -v rustc >/dev/null; then
        warn "Rust (cargo/rustc) is not installed; it is needed to build swayling"
        local pm cmd
        IFS='|' read -r pm cmd _ <<<"$(detect_pkg_manager)"
        if [[ -n "$cmd" ]] && ask "Install it with $pm ($cmd)?"; then
            # shellcheck disable=SC2086
            as_root $cmd
        elif ask "Install it with rustup instead (into ~/.cargo, no root needed)?"; then
            install_rustup
        else
            die "install Rust $rust_min or newer (https://rustup.rs${cmd:+ or: $cmd}) and run this script again"
        fi
        command -v cargo >/dev/null || die "cargo is still not available"
    fi

    local version
    version="$(rust_version)"
    if ! version_ge "$version" "$rust_min"; then
        warn "Rust $version is too old, swayling needs $rust_min or newer"
        if command -v rustup >/dev/null; then
            if ask "Update it with 'rustup update stable'?"; then
                rustup update stable
            else
                die "update Rust (rustup update stable) and run this script again"
            fi
        elif ask "Your distribution's Rust is too old. Install a current one with rustup (into ~/.cargo)?"; then
            install_rustup
        else
            die "install Rust $rust_min or newer, e.g. with rustup: https://rustup.rs"
        fi
        version="$(rust_version)"
        version_ge "$version" "$rust_min" || die "Rust $version is still older than $rust_min"
    fi
    printf '    %-10s %s\n' "rust" "ok ($version, needs >= $rust_min)"

    # rustc links through the system C compiler.
    if ! command -v cc >/dev/null; then
        local pm toolchain
        IFS='|' read -r pm _ toolchain <<<"$(detect_pkg_manager)"
        warn "no C compiler/linker (cc) found; Rust needs it to link"
        if [[ -n "$toolchain" && "$pm" != unknown ]] && ask "Install $toolchain with $pm?"; then
            case "$pm" in
                pacman) as_root pacman -S --needed "$toolchain" ;;
                xbps) as_root xbps-install -S "$toolchain" ;;
                apk) as_root apk add "$toolchain" ;;
                dnf) as_root dnf install "$toolchain" ;;
                zypper) as_root zypper install "$toolchain" ;;
                portage) as_root emerge --ask=n "$toolchain" ;;
                apt) as_root apt-get install -y "$toolchain" ;;
            esac
        fi
        command -v cc >/dev/null || die "install a C toolchain (${toolchain:-gcc}) and run this script again"
    fi
    printf '    %-10s %s\n' "cc" "ok ($(command -v cc))"
}

# Tools the script itself relies on. sway/swaymsg are only needed for
# validation and reloading, so they are optional.
check_tools() {
    local missing=() tool
    for tool in awk sed grep diff cmp install readlink mktemp sort pgrep pkill; do
        command -v "$tool" >/dev/null || missing+=("$tool")
    done
    if [[ ${#missing[@]} -gt 0 ]]; then
        die "missing tools: ${missing[*]} (install coreutils, diffutils and procps)"
    fi
    printf '    %-10s %s\n' "tools" "ok"
    for tool in sway swaymsg; do
        if command -v "$tool" >/dev/null; then
            printf '    %-10s %s\n' "$tool" "ok"
        else
            printf '    %-10s %s\n' "$tool" "not found (config validation/reload will be skipped)"
        fi
    done
}

info "checking requirements"
check_tools
needs_build=0
if [[ $uninstall -eq 0 && $dry_run -eq 0 && ($do_build -eq 1 || $check_only -eq 1) ]]; then
    needs_build=1
fi
if [[ $needs_build -eq 1 ]]; then
    check_rust
fi
if [[ $check_only -eq 1 ]]; then
    info "all requirements are met"
    exit 0
fi

# --- Find the config -------------------------------------------------------

detect_config() {
    # The config the running sway actually loaded is the most reliable answer.
    if [[ -n "${SWAYSOCK:-}" ]] && command -v swaymsg >/dev/null; then
        local loaded
        loaded="$(swaymsg -r -t get_version 2>/dev/null \
            | sed -n 's/.*"loaded_config_file_name": *"\([^"]*\)".*/\1/p' \
            | sed 's#\\/#/#g')"
        if [[ -n "$loaded" && -f "$loaded" ]]; then
            printf '%s\n' "$loaded"
            return
        fi
    fi
    # Otherwise use sway's own lookup order.
    local xdg="${XDG_CONFIG_HOME:-$HOME/.config}" candidate
    for candidate in "$HOME/.sway/config" "$xdg/sway/config" "$HOME/.i3/config" "$xdg/i3/config"; do
        if [[ -f "$candidate" ]]; then
            printf '%s\n' "$candidate"
            return
        fi
    done
}

if [[ -z "$config" ]]; then
    config="$(detect_config)"
    [[ -n "$config" ]] || die "no sway config found; pass one with --config FILE"
fi
[[ -f "$config" ]] || die "config '$config' does not exist"
# Resolve symlinks (dotfile managers) so the backup and temp file end up next
# to the real file; writing goes through `cat >` so the link itself survives.
config_real="$(readlink -f "$config")"
[[ -w "$config_real" ]] || die "'$config_real' is not writable (system config? copy it to ~/.config/sway/config first)"
info "sway config: $config"

# --- Autostart -----------------------------------------------------------------

SERVICE_MARKER='# managed by swayling install.sh'
service_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
service_file="$service_dir/swayling.service"

# Prints "<method> <systemd target> <reason>". A systemd user service is only
# used when sway runs inside a systemd graphical session (uwsm, sway-systemd,
# ...) and SWAYSOCK is exported to systemd; otherwise the service would never
# start or could not find sway. Everything else (ly, greetd, tty, ...) gets
# `exec_always` in the sway config, which works with any setup.
detect_autostart() {
    if ! command -v systemctl >/dev/null; then
        echo "sway - systemd is not available"
        return
    fi
    case "$(systemctl --user is-system-running 2>/dev/null || true)" in
        running|degraded|starting) ;;
        *)
            echo "sway - no systemd user instance is running"
            return
            ;;
    esac
    local target
    for target in sway-session.target graphical-session.target; do
        if systemctl --user is-active -q "$target" 2>/dev/null; then
            if ! systemctl --user show-environment 2>/dev/null | grep -q '^SWAYSOCK='; then
                echo "sway - $target is active, but SWAYSOCK is not exported to systemd"
                return
            fi
            local how="systemd session"
            if command -v uwsm >/dev/null \
                && systemctl --user list-units --no-legend 'wayland-wm@*' 2>/dev/null | grep -q .; then
                how="uwsm session"
            fi
            echo "systemd $target sway runs in a $how ($target is active)"
            return
        fi
    done
    echo "sway - sway is not part of a systemd graphical session (started by a display manager or from a tty)"
}

read -r detected_method detected_target detected_reason <<<"$(detect_autostart)"
case "$autostart" in
    auto)
        method="$detected_method" target="$detected_target" reason="$detected_reason"
        ;;
    sway)
        method=sway target=- reason="chosen with --autostart sway"
        ;;
    systemd)
        method=systemd target="$detected_target" reason="chosen with --autostart systemd"
        if [[ "$target" == - ]]; then
            target=graphical-session.target
            warn "no active session target detected, using $target; the service only starts once that target is reached"
        fi
        ;;
esac
if [[ $uninstall -eq 0 ]]; then
    if [[ "$method" == systemd ]]; then
        info "autostart: systemd user service (WantedBy=$target), because $reason"
    else
        info "autostart: exec_always in the sway config, because $reason"
    fi
fi

render_service() {
    local args=""
    if [[ -n "$ratio" ]]; then
        args=" --ratio $ratio"
    fi
    cat <<UNIT
$SERVICE_MARKER
[Unit]
Description=swayling - Hyprland-style dwindle tiling for sway
PartOf=$target
After=$target

[Service]
ExecStart=$bin$args
Restart=on-failure
RestartSec=1

[Install]
WantedBy=$target
UNIT
}

# True if the service file exists and was written by this script.
have_managed_service() {
    [[ -f "$service_file" ]] && grep -qxF "$SERVICE_MARKER" "$service_file"
}

install_service() {
    mkdir -p "$service_dir"
    render_service >"$service_file"
    systemctl --user daemon-reload
    systemctl --user enable -q swayling.service
    info "installed and enabled $service_file"
    # Replace an instance started by exec_always or by an older service run.
    pkill -x swayling 2>/dev/null || true
    if systemctl --user is-active -q "$target"; then
        systemctl --user restart swayling.service
        info "started swayling.service"
    fi
}

remove_service() {
    if have_managed_service; then
        systemctl --user disable -q --now swayling.service 2>/dev/null || true
        rm -f "$service_file"
        systemctl --user daemon-reload 2>/dev/null || true
        info "removed $service_file"
    elif [[ -f "$service_file" ]]; then
        warn "$service_file exists but was not written by install.sh; leaving it alone"
    fi
}

# --- Config rewriting --------------------------------------------------------

# Escapes a string for use as a sed replacement with `#` as delimiter.
sed_escape() { printf '%s' "$1" | sed 's/[\\&#]/\\&/g'; }

# `bindsym [--flags] KEY ` prefix of a binding line, captured as \1.
BIND_RE='^([[:space:]]*bind(sym|code)([[:space:]]+--[^[:space:]]+)*[[:space:]]+[^[:space:]]+[[:space:]]+)'


# Removes our marked block.
strip_block() {
    awk -v b="$BLOCK_BEGIN" -v e="$BLOCK_END" '
        $0 == b { skip = 1; next }
        $0 == e && skip { skip = 0; next }
        !skip
    ' "$1"
}

# True if KEY is already bound by a `bindsym` line in FILE.
key_is_bound() {
    awk -v key="$1" '
        $1 == "bindsym" {
            i = 2
            while ($i ~ /^--/) i++
            if ($i == key) found = 1
        }
        END { exit !found }
    ' "$2"
}

render_install() {
    local src="$1" esc_bin
    esc_bin="$(sed_escape "$bin")"
    local args=""
    if [[ -n "$ratio" ]]; then
        args=" --ratio $ratio"
    fi

    local body
    body="$(strip_block "$src")"

    if [[ $do_move -eq 1 ]]; then
        body="$(printf '%s\n' "$body" | sed -E \
            -e "s#${BIND_RE}move[[:space:]]+($DIRS)[[:space:]]*\$#\1exec $esc_bin move \4#" \
            -e "s#${BIND_RE}exec[[:space:]]+[^[:space:]]*swayling[[:space:]]+move[[:space:]]+($DIRS)[[:space:]]*\$#\1exec $esc_bin move \4#")"
    fi

    # Daemon lines added by hand (e.g. from the README) are replaced by the
    # one in our block, so the daemon is never started twice.
    local daemon_awk='/^[[:space:]]*exec(_always)?[[:space:]].*swayling/ && !/swayling[[:space:]]+(move|retile)/'
    if printf '%s\n' "$body" | awk "$daemon_awk { found = 1 } END { exit !found }"; then
        info "replacing a hand-written swayling exec line with the managed one" >&2
        body="$(printf '%s\n' "$body" | awk "$daemon_awk { next } { print }")"
    fi

    local add_retile=$do_retile
    if [[ $do_retile -eq 1 ]]; then
        local tmp_body
        tmp_body="$(mktemp)"
        printf '%s\n' "$body" >"$tmp_body"
        if grep -qE 'swayling[[:space:]]+retile' "$tmp_body"; then
            add_retile=0
        elif key_is_bound "$retile_key" "$tmp_body"; then
            warn "$retile_key is already bound, skipping the retile binding (use --retile-key)"
            add_retile=0
        fi
        rm -f "$tmp_body"
    fi

    printf '%s\n' "$body"
    printf '\n%s\n' "$BLOCK_BEGIN"
    if [[ "$method" == sway ]]; then
        # Quoted, so sway hands the whole line to one shell: without quotes
        # sway splits at `;`, and the `exec` half would only run at startup.
        printf 'exec_always "pkill -x swayling; exec %s%s"\n' "$bin" "$args"
    fi
    if [[ $add_retile -eq 1 ]]; then
        printf 'bindsym %s exec %s retile%s\n' "$retile_key" "$bin" "$args"
    fi
    printf '%s\n' "$BLOCK_END"
}

render_uninstall() {
    strip_block "$1" | sed -E \
        "s#${BIND_RE}exec[[:space:]]+[^[:space:]]*swayling[[:space:]]+move[[:space:]]+($DIRS)[[:space:]]*\$#\1move \4#"
}

# The temp file lives next to the config so relative `include`s still resolve
# during validation.
tmp="$(dirname "$config_real")/.swayling-config.$$"
trap 'rm -f "$tmp"' EXIT

if [[ $uninstall -eq 1 ]]; then
    render_uninstall "$config_real" >"$tmp"
else
    render_install "$config_real" >"$tmp"
fi

# Collapse the blank line left behind when the block is removed again.
sed -i -e ':a' -e '/^\n*$/{$d;N;ba' -e '}' "$tmp"

changed=1
if cmp -s "$config_real" "$tmp"; then
    changed=0
fi

if [[ $dry_run -eq 1 ]]; then
    if [[ $changed -eq 1 ]]; then
        # `--label` is GNU-only (busybox diff lacks it); exit status 2 means
        # an error, so fall back to a plain diff then.
        rc=0
        diff -u --label "$config (current)" --label "$config (new)" "$config_real" "$tmp" 2>/dev/null || rc=$?
        if [[ $rc -eq 2 ]]; then
            diff -u "$config_real" "$tmp" || true
        fi
    else
        info "config is already up to date"
    fi
    if [[ $uninstall -eq 0 && "$method" == systemd ]]; then
        info "would write $service_file:"
        render_service
    elif have_managed_service; then
        info "would remove $service_file"
    fi
    exit 0
fi

# --- Build and install -------------------------------------------------------

install_binary() {
    if [[ $do_build -eq 1 ]]; then
        info "building swayling (release)"
        cargo build --release --locked --manifest-path "$repo_dir/Cargo.toml"
    fi
    local built="$repo_dir/target/release/swayling"
    [[ -x "$built" ]] || die "no binary at $built (run without --skip-build)"

    mkdir -p "$bin_dir"
    # Install next to the target and rename, so a running daemon keeps its
    # old binary instead of failing with "text file busy".
    install -m 755 "$built" "$bin_dir/.swayling.new"
    mv -f "$bin_dir/.swayling.new" "$bin"
    info "installed $bin"

    # Other copies earlier in PATH would shadow this one in a terminal.
    local found
    found="$(command -v swayling || true)"
    if [[ -n "$found" && "$(readlink -f "$found")" != "$(readlink -f "$bin")" ]]; then
        warn "'swayling' in your PATH resolves to $found, not $bin"
        warn "the sway config uses the full path, but you may want to remove the old copy"
        if [[ "$found" == "$HOME/.cargo/bin/swayling" ]]; then
            warn "  cargo uninstall swayling"
        fi
    fi
}

if [[ $uninstall -eq 0 ]]; then
    install_binary
fi

# --- Write the config ----------------------------------------------------------

if [[ $changed -eq 1 ]]; then
    if command -v sway >/dev/null; then
        if ! errors="$(sway --validate -c "$tmp" 2>&1)"; then
            printf '%s\n' "$errors" >&2
            die "the new config does not validate; your config was not touched"
        fi
    else
        warn "sway not found in PATH, skipping config validation"
    fi

    backup_base="$config_real.bak-$(date +%Y%m%d_%H%M%S)"
    backup="$backup_base"
    # Never overwrite an existing backup (two runs within one second).
    n=1
    while [[ -e "$backup" ]]; do
        backup="$backup_base-$n"
        n=$((n + 1))
    done
    cp -p "$config_real" "$backup"
    cat "$tmp" >"$config_real"
    info "updated $config (backup: $backup)"
else
    info "config is already up to date"
fi

# --- Autostart setup -------------------------------------------------------------

if [[ $uninstall -eq 0 && "$method" == systemd ]]; then
    install_service
else
    # Switching to exec_always (or uninstalling): drop a service from an
    # earlier run, so the daemon is never started twice.
    remove_service
fi

if [[ $uninstall -eq 1 ]]; then
    pkill -x swayling 2>/dev/null && info "stopped the running daemon" || true
    if [[ -e "$bin" ]]; then
        rm -f "$bin"
        info "removed $bin"
    fi
fi

# --- Activate ------------------------------------------------------------------

sway_running() {
    [[ -n "${SWAYSOCK:-}" ]] && command -v swaymsg >/dev/null && swaymsg -t get_version >/dev/null 2>&1
}

# Waits up to two seconds for the daemon to show up.
daemon_started() {
    local i
    for i in 1 2 3 4 5 6 7 8 9 10; do
        if pgrep -x swayling >/dev/null; then
            return 0
        fi
        sleep 0.2
    done
    return 1
}

started_ok=1
if sway_running; then
    if [[ $do_reload -eq 1 ]]; then
        # Applies the new bindings and, via exec_always, starts the daemon.
        swaymsg reload >/dev/null
        info "sway reloaded, the new key bindings are active"
    elif [[ $uninstall -eq 0 ]]; then
        info "not reloading: the new key bindings apply after your next reload (\$mod+Shift+c)"
        if [[ "$method" == sway ]]; then
            # Start the daemon through sway, so it runs in sway's environment.
            args=""
            if [[ -n "$ratio" ]]; then
                args=" --ratio $ratio"
            fi
            swaymsg "exec \"pkill -x swayling; exec $bin$args\"" >/dev/null
        fi
    fi
    if [[ $uninstall -eq 0 ]]; then
        if daemon_started; then
            info "swayling is running (pid $(pgrep -x swayling | head -n 1))"
        else
            started_ok=0
            warn "swayling did not start; try running '$bin --verbose' to see why"
        fi
    fi
elif [[ $uninstall -eq 0 ]]; then
    info "sway is not running here; swayling starts with your next sway session"
fi

# --- Done ------------------------------------------------------------------------

# Only reached when every step above succeeded (`set -e` and `die` abort
# earlier otherwise).
if [[ $uninstall -eq 0 && $started_ok -eq 1 ]]; then
    green='' reset=''
    if [[ -t 1 ]]; then
        green=$'\e[1;32m' reset=$'\e[0m'
    fi
    printf '\n%s' "$green"
    cat <<'BANNER'
 ___  _  _  ___  _____    _    _     _
|_ _|| \| |/ __||_   _|  /_\  | |   | |
 | | | .` |\__ \  | |   / _ \ | |__ | |__
|___||_|\_||___/  |_|  /_/ \_\|____||____|

 ___  _   _   ___   ___  ___  ___  ___  ___  _   _  _           _   ____
/ __|| | | | / __| / __|| __|/ __|/ __|| __|| | | || |         (_) |__ /
\__ \| |_| || (__ | (__ | _| \__ \\__ \| _| | |_| || |__        _   |_ \
|___/ \___/  \___| \___||___||___/|___/|_|   \___/ |____|      (_) |___/
BANNER
    printf '%s\n' "$reset"
fi
