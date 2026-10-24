//! Minimal sway IPC client (only what swayling needs).
//!
//! Wire format: `"i3-ipc"` magic, payload length (u32), message type (u32),
//! both in native byte order, followed by a JSON payload.

use std::env;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;

use serde::Deserialize;

const MAGIC: &[u8; 6] = b"i3-ipc";
const HEADER_LEN: usize = MAGIC.len() + 8;

pub const RUN_COMMAND: u32 = 0;
pub const GET_WORKSPACES: u32 = 1;
pub const SUBSCRIBE: u32 = 2;
pub const GET_TREE: u32 = 4;

pub const EVENT_WINDOW: u32 = 0x8000_0003;
pub const EVENT_SHUTDOWN: u32 = 0x8000_0006;

#[derive(Deserialize)]
struct CommandOutcome {
    success: bool,
    error: Option<String>,
}

pub struct Ipc(UnixStream);

impl Ipc {
    pub fn connect() -> io::Result<Self> {
        let path = env::var_os("SWAYSOCK")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "SWAYSOCK is not set"))?;
        UnixStream::connect(path).map(Self)
    }

    pub fn send(&mut self, kind: u32, payload: &[u8]) -> io::Result<()> {
        let len = u32::try_from(payload.len()).map_err(io::Error::other)?;
        let mut msg = Vec::with_capacity(HEADER_LEN + payload.len());
        msg.extend_from_slice(MAGIC);
        msg.extend_from_slice(&len.to_ne_bytes());
        msg.extend_from_slice(&kind.to_ne_bytes());
        msg.extend_from_slice(payload);
        self.0.write_all(&msg)
    }

    /// Reads the next message into `buf` (reused between calls to avoid
    /// allocations) and returns its type.
    pub fn recv(&mut self, buf: &mut Vec<u8>) -> io::Result<u32> {
        let mut header = [0u8; HEADER_LEN];
        self.0.read_exact(&mut header)?;
        if &header[..MAGIC.len()] != MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "bad IPC magic"));
        }
        let len = u32::from_ne_bytes(header[6..10].try_into().unwrap()) as usize;
        let kind = u32::from_ne_bytes(header[10..14].try_into().unwrap());
        buf.resize(len, 0);
        self.0.read_exact(buf)?;
        Ok(kind)
    }

    /// Sends a request and reads its reply. Only valid on a connection that is
    /// not subscribed to events.
    pub fn request(&mut self, kind: u32, payload: &[u8], buf: &mut Vec<u8>) -> io::Result<()> {
        self.send(kind, payload)?;
        self.recv(buf)?;
        Ok(())
    }

    /// Runs sway commands. Failing commands are reported but not fatal, since
    /// e.g. a window may have closed in the meantime.
    pub fn run_command(&mut self, command: &str, buf: &mut Vec<u8>) -> io::Result<()> {
        self.request(RUN_COMMAND, command.as_bytes(), buf)?;
        let outcomes: Vec<CommandOutcome> = serde_json::from_slice(buf)?;
        for outcome in outcomes.iter().filter(|o| !o.success) {
            let error = outcome.error.as_deref().unwrap_or("unknown error");
            eprintln!("swayling: '{command}' failed: {error}");
        }
        Ok(())
    }
}
