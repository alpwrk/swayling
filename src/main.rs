//! swayling: Hyprland-style dwindle tiling for sway and SwayFX.

mod commands;
mod dwindle;
mod ipc;
mod tree;

use std::io;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use serde::Deserialize;

use commands::Direction;
use dwindle::Dwindle;
use ipc::{EVENT_SHUTDOWN, EVENT_WINDOW, Ipc, SUBSCRIBE};

const HELP: &str = "\
swayling - Hyprland-style dwindle tiling for sway and SwayFX

USAGE:
    swayling [OPTIONS]               Run the tiling daemon
    swayling move <DIRECTION>        Swap the focused window with its neighbour
                                     (left, right, up, down); bind instead of `move`
    swayling retile [--ratio RATIO]  Rebuild the focused workspace as a spiral

OPTIONS:
    -r, --ratio <RATIO>        Split horizontally when width > height * RATIO,
                               otherwise vertically [default: 1.0]
    -w, --workspaces <LIST>    Only tile these workspaces (comma-separated names,
                               e.g. \"1,2,web\"). Default: all workspaces
    -v, --verbose              Log every split decision to stderr
    -h, --help                 Print this help
    -V, --version              Print version
";

/// How often to retry connecting before giving up (sway is probably gone).
const MAX_RECONNECT_ATTEMPTS: u32 = 10;
const RECONNECT_DELAY: Duration = Duration::from_millis(500);

/// What the invocation asks for.
enum Mode {
    Daemon(Dwindle),
    Move(Direction),
    Retile(f32),
    /// `--help` or `--version` was handled already.
    Done,
}

fn parse_args() -> Result<Mode, String> {
    let mut args = std::env::args().skip(1).peekable();
    let subcommand = match args.peek().map(String::as_str) {
        Some(cmd @ ("move" | "retile")) => {
            let cmd = cmd.to_string();
            args.next();
            Some(cmd)
        }
        _ => None,
    };

    if subcommand.as_deref() == Some("move") {
        let dir = args
            .next()
            .ok_or("move requires a direction (left, right, up, down)")?;
        let dir = Direction::parse(&dir).ok_or_else(|| format!("invalid direction '{dir}'"))?;
        if let Some(extra) = args.next() {
            return Err(format!("unexpected argument '{extra}'"));
        }
        return Ok(Mode::Move(dir));
    }

    let mut dwindle = Dwindle {
        ratio: 1.0,
        workspaces: None,
        verbose: false,
    };

    while let Some(arg) = args.next() {
        // Accept both `--ratio 1.2` and `--ratio=1.2`.
        let (flag, inline_value) = match arg.split_once('=') {
            Some((flag, value)) => (flag.to_string(), Some(value.to_string())),
            None => (arg, None),
        };
        let mut value = |name: &str| {
            inline_value
                .clone()
                .or_else(|| args.next())
                .ok_or_else(|| format!("{name} requires a value"))
        };

        match flag.as_str() {
            "-r" | "--ratio" => {
                let raw = value("--ratio")?;
                let ratio: f32 = raw.parse().map_err(|_| format!("invalid ratio '{raw}'"))?;
                if !ratio.is_finite() || ratio <= 0.0 {
                    return Err(format!("ratio must be a positive number, got '{raw}'"));
                }
                dwindle.ratio = ratio;
            }
            "-w" | "--workspaces" if subcommand.is_none() => {
                let list: Vec<String> = value("--workspaces")?
                    .split(',')
                    .map(str::trim)
                    .filter(|w| !w.is_empty())
                    .map(String::from)
                    .collect();
                if list.is_empty() {
                    return Err("--workspaces needs at least one workspace name".into());
                }
                dwindle.workspaces = Some(list);
            }
            "-v" | "--verbose" if subcommand.is_none() => dwindle.verbose = true,
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(Mode::Done);
            }
            "-V" | "--version" => {
                println!("swayling {}", env!("CARGO_PKG_VERSION"));
                return Ok(Mode::Done);
            }
            other => return Err(format!("unknown argument '{other}' (see --help)")),
        }
    }
    Ok(match subcommand {
        Some(_) => Mode::Retile(dwindle.ratio),
        None => Mode::Daemon(dwindle),
    })
}

/// Only the `change` field of a window event is parsed; the embedded
/// container is skipped.
#[derive(Deserialize)]
struct WindowEvent<'a> {
    change: &'a str,
}

#[derive(Deserialize)]
struct Success {
    success: bool,
}

/// Adds a short description to an IPC error.
fn context(msg: &'static str) -> impl FnOnce(io::Error) -> io::Error {
    move |e| io::Error::new(e.kind(), format!("{msg}: {e}"))
}

/// Connects to sway and processes window events until sway shuts down
/// (`Ok`) or the connection breaks (`Err`).
fn run(dwindle: &Dwindle, attempts: &mut u32) -> io::Result<()> {
    // The subscribed connection only receives events, so a second one is
    // used to query the tree and send commands.
    let mut commands = Ipc::connect().map_err(context("cannot connect to sway IPC"))?;
    let mut events = Ipc::connect().map_err(context("cannot connect to sway IPC"))?;
    let mut buf = Vec::new();
    events
        .request(SUBSCRIBE, br#"["window","shutdown"]"#, &mut buf)
        .map_err(context("cannot subscribe to sway events"))?;
    let reply: Success = serde_json::from_slice(&buf)?;
    if !reply.success {
        return Err(io::Error::other("sway rejected the event subscription"));
    }

    // Connected successfully, so later failures get a fresh retry budget.
    *attempts = 0;

    // Prepare the currently focused window right away.
    dwindle.apply(&mut commands, &mut buf)?;

    // Separate buffer, because `apply` reuses `buf` while an event is handled.
    let mut event = Vec::new();
    loop {
        match events
            .recv(&mut event)
            .map_err(context("lost connection to sway"))?
        {
            EVENT_WINDOW => {
                let e: WindowEvent = serde_json::from_slice(&event)?;
                if matches!(e.change, "focus" | "new" | "move" | "floating") {
                    dwindle.apply(&mut commands, &mut buf)?;
                }
            }
            EVENT_SHUTDOWN => return Ok(()),
            _ => {}
        }
    }
}

/// Exit status for the one-shot commands.
fn report(result: io::Result<()>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("swayling: {e}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let dwindle = match parse_args() {
        Ok(Mode::Daemon(dwindle)) => dwindle,
        Ok(Mode::Move(dir)) => return report(commands::move_window(dir)),
        Ok(Mode::Retile(ratio)) => return report(commands::retile(ratio)),
        Ok(Mode::Done) => return ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("swayling: {e}");
            return ExitCode::from(2);
        }
    };

    let mut attempts = 0;
    loop {
        match run(&dwindle, &mut attempts) {
            Ok(()) => {
                if dwindle.verbose {
                    eprintln!("swayling: sway is shutting down, exiting");
                }
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                // sway removes its socket on exit, often before the shutdown
                // event reaches us; no point in retrying then.
                if !std::env::var_os("SWAYSOCK").is_some_and(|p| std::path::Path::new(&p).exists())
                {
                    if dwindle.verbose {
                        eprintln!("swayling: sway is gone ({e}), exiting");
                    }
                    return ExitCode::SUCCESS;
                }
                attempts += 1;
                if attempts > MAX_RECONNECT_ATTEMPTS {
                    eprintln!("swayling: {e}; giving up after {MAX_RECONNECT_ATTEMPTS} attempts");
                    return ExitCode::FAILURE;
                }
                eprintln!("swayling: {e}; reconnecting ({attempts}/{MAX_RECONNECT_ATTEMPTS})");
                thread::sleep(RECONNECT_DELAY);
            }
        }
    }
}
