//! PTY session: a real terminal backed by `portable-pty`.
//!
//! Each `PtySession` owns a pseudo-terminal pair, a spawned shell child
//! process, and a background thread that continuously drains the master
//! read-end into an in-memory ring buffer.

use crate::error::{Result, TerminalError};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::{
    io::{Read, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::{debug, warn};

/// Maximum bytes kept in the output ring buffer (128 KiB).
const OUTPUT_BUF_MAX: usize = 131_072;

/// A live PTY session wrapping a single shell process.
///
/// The struct uses:
/// - `Mutex<Box<dyn Write>>` for the write half (caller stdin → shell)
/// - `Mutex<Box<dyn MasterPty>>` for resize (requires exclusive access)
/// - `Mutex<String>` as an accumulating ring buffer for shell output
/// - `AtomicBool` to track whether the background reader thread is alive
pub struct PtySession {
    /// The shell binary that was launched.
    pub shell: String,

    /// The working directory the shell started in.
    pub cwd: String,

    /// Unix timestamp (seconds since epoch) when the session was created.
    pub created_at: u64,

    /// Write half — sends bytes to the shell's stdin.
    writer: Mutex<Box<dyn Write + Send>>,

    /// Master PTY handle — used for resize.
    master: Mutex<Box<dyn MasterPty + Send>>,

    /// Accumulated ANSI-stripped output from the shell.
    output_buf: Arc<Mutex<String>>,

    /// Set to `false` by the reader thread when the child exits or errors.
    alive: Arc<AtomicBool>,
}

impl PtySession {
    /// Spawn a new PTY session running `shell` in `cwd`.
    ///
    /// A background thread is started immediately to drain the master
    /// read-end into the output buffer.
    pub fn new(shell: &str, cwd: &str) -> Result<Self> {
        let pty_system = native_pty_system();

        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| TerminalError::PtySpawn(e.to_string()))?;

        let mut cmd = CommandBuilder::new(shell);
        cmd.cwd(cwd);

        pair.slave
            .spawn_command(cmd)
            .map_err(|e| TerminalError::PtySpawn(e.to_string()))?;

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| TerminalError::PtySpawn(e.to_string()))?;

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| TerminalError::PtySpawn(e.to_string()))?;

        let output_buf: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
        let alive = Arc::new(AtomicBool::new(true));

        // Clones for the background reader thread.
        let buf_clone = Arc::clone(&output_buf);
        let alive_clone = Arc::clone(&alive);

        // Blocking I/O runs in a dedicated OS thread so it never blocks Tokio.
        std::thread::spawn(move || {
            let mut raw = [0u8; 4096];
            loop {
                match reader.read(&mut raw) {
                    Ok(0) => break, // EOF — shell exited
                    Ok(n) => {
                        // Strip ANSI escape sequences for clean AI-readable text.
                        let clean = strip_ansi_escapes::strip(&raw[..n]);
                        let text = String::from_utf8_lossy(&clean).into_owned();

                        let mut guard = buf_clone.lock().unwrap();
                        guard.push_str(&text);

                        // Trim oldest data when the ring buffer overflows.
                        if guard.len() > OUTPUT_BUF_MAX {
                            let excess = guard.len() - OUTPUT_BUF_MAX;
                            guard.drain(..excess);
                        }
                    }
                    Err(e) => {
                        warn!("PTY reader error: {e}");
                        break;
                    }
                }
            }
            alive_clone.store(false, Ordering::Release);
            debug!("PTY reader thread exited");
        });

        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Ok(Self {
            shell: shell.to_string(),
            cwd: cwd.to_string(),
            created_at,
            writer: Mutex::new(writer),
            master: Mutex::new(pair.master),
            output_buf,
            alive,
        })
    }

    /// Write `input` bytes to the shell's stdin.
    ///
    /// Common sequences:
    /// - `"ls -la\n"` — run a command
    /// - `"\x03"` — Ctrl-C (interrupt)
    /// - `"\x04"` — Ctrl-D (EOF / logout)
    pub fn write(&self, input: &str) -> Result<()> {
        let mut guard = self.writer.lock().unwrap();
        guard.write_all(input.as_bytes())?;
        guard.flush()?;
        Ok(())
    }

    /// Drain and return all accumulated output from the buffer, then clear it.
    pub fn read(&self) -> Result<String> {
        let mut guard = self.output_buf.lock().unwrap();
        Ok(std::mem::take(&mut *guard))
    }

    /// Resize the terminal window.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        let guard = self.master.lock().unwrap();
        guard
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| TerminalError::IoError(std::io::Error::other(e.to_string())))
    }

    /// Returns `true` if the background reader thread (and thus the shell) is
    /// still running.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    /// Ask the shell to exit by sending Ctrl-D (EOF).
    pub fn kill(&self) -> Result<()> {
        // "\x04" = Ctrl-D: triggers shell EOF → clean exit for most shells.
        let _ = self.write("\x04");
        Ok(())
    }
}
