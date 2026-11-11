use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use super::{Session, SessionBackend, SessionEvent, SessionKind};
use crate::config::LocalOptions;
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};

struct LocalBackend {
    writer: Mutex<Box<dyn Write + Send>>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    killer: Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>,
    events: async_channel::Sender<SessionEvent>,
}

impl SessionBackend for LocalBackend {
    fn write(&self, data: &[u8]) {
        let mut writer = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(error) = writer.write_all(data).and_then(|_| writer.flush()) {
            let _ = self
                .events
                .try_send(SessionEvent::Error(format!("pty write failed: {error}")));
        }
    }

    fn resize(&self, columns: u16, rows: u16) {
        let master = self.master.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(error) = master.resize(PtySize {
            rows: rows.max(1),
            cols: columns.max(2),
            pixel_width: 0,
            pixel_height: 0,
        }) {
            log::warn!("pty resize failed: {error}");
        }
    }

    fn close(&self) {
        let mut killer = self.killer.lock().unwrap_or_else(|e| e.into_inner());
        let _ = killer.kill();
    }
}

impl Drop for LocalBackend {
    fn drop(&mut self) {
        if let Ok(mut killer) = self.killer.lock() {
            let _ = killer.kill();
        }
    }
}

fn options_environment(options: &LocalOptions) -> Vec<(String, String)> {
    let mut env = Vec::new();
    {
        for (key, value) in &options.env {
            if let (Some(key), Some(value)) = (key.as_str(), value.as_str()) {
                env.push((key.to_string(), value.to_string()));
            }
        }
    }
    env
}

/// Spawn a shell (or arbitrary command) on a local PTY.
pub fn spawn_local(options: LocalOptions, columns: u16, rows: u16) -> anyhow::Result<Session> {
    let (command, mut args) = options.command_and_args();
    let cwd = options
        .cwd
        .as_deref()
        .filter(|path| std::path::Path::new(path).is_dir());
    if cwd.is_none() {
        args.extend(options.home_dir_args.iter().cloned());
    }

    let pty_system = native_pty_system();
    let pair = pty_system.openpty(PtySize {
        rows: rows.max(1),
        cols: columns.max(2),
        pixel_width: 0,
        pixel_height: 0,
    })?;

    let mut builder = CommandBuilder::new(&command);
    builder.args(&args);
    if let Some(cwd) = cwd {
        builder.cwd(cwd);
    }
    for (key, value) in options_environment(&options) {
        builder.env(key, value);
    }
    builder.env("TERM", "xterm-256color");
    builder.env("COLORTERM", "truecolor");
    builder.env("TERM_PROGRAM", "tabby-rust");
    builder.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
    // Tabby identifies its own terminals; shell integrations can look for it.
    builder.env("TABBY", "1");

    let child = pair.slave.spawn_command(builder)?;
    let pid = child.process_id();
    let mut reader = pair.master.try_clone_reader()?;
    let writer = pair.master.take_writer()?;
    let killer = child.clone_killer();

    let (tx, rx) = async_channel::unbounded();
    let reader_tx = tx.clone();
    std::thread::Builder::new()
        .name("tabby-pty-read".into())
        .spawn(move || {
            let mut buffer = vec![0u8; 16 * 1024];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        if reader_tx
                            .try_send(SessionEvent::Data(buffer[..n].to_vec()))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        // EIO on Linux is the normal EOF for a PTY whose child exited.
                        if error.raw_os_error() != Some(libc_eio()) {
                            log::debug!("pty read ended: {error}");
                        }
                        break;
                    }
                }
            }
            let _ = reader_tx.try_send(SessionEvent::Closed);
        })?;

    let wait_tx = tx.clone();
    std::thread::Builder::new()
        .name("tabby-pty-wait".into())
        .spawn(move || {
            let mut child = child;
            match child.wait() {
                Ok(status) => {
                    let _ = wait_tx.try_send(SessionEvent::Exited(Some(status.exit_code() as i32)));
                }
                Err(error) => {
                    let _ = wait_tx.try_send(SessionEvent::Error(format!(
                        "waiting for child failed: {error}"
                    )));
                }
            }
            let _ = wait_tx.try_send(SessionEvent::Closed);
        })?;

    let backend = Arc::new(LocalBackend {
        writer: Mutex::new(writer),
        master: Mutex::new(pair.master),
        killer: Mutex::new(killer),
        events: tx,
    });

    Ok(Session::new(SessionKind::Local, rx, backend, pid))
}

fn libc_eio() -> i32 {
    5
}
