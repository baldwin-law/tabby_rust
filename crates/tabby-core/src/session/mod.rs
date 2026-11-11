//! Session backends: local PTYs, SSH channels and raw/telnet sockets.

mod local;
mod ssh;
mod telnet;

use std::path::PathBuf;
use std::sync::Arc;

pub use local::spawn_local;
pub use ssh::{KnownHostsPolicy, spawn_ssh};
pub use telnet::spawn_telnet;

use crate::config::{LocalOptions, SshOptions, TelnetOptions};

/// Data coming out of a session. The UI owns VT state; a session only moves
/// bytes and lifecycle events.
#[derive(Clone, Debug)]
pub enum SessionEvent {
    Data(Vec<u8>),
    Exited(Option<i32>),
    Closed,
    Error(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionKind {
    Local,
    Ssh {
        host: String,
        user: String,
        port: u16,
    },
    Telnet {
        host: String,
        port: u16,
    },
}

impl SessionKind {
    pub fn title(&self) -> String {
        match self {
            Self::Local => "Local".into(),
            Self::Ssh { host, user, .. } => format!("{user}@{host}"),
            Self::Telnet { host, .. } => host.clone(),
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::Local => "terminal",
            Self::Ssh { .. } => "ssh",
            Self::Telnet { .. } => "network",
        }
    }
}

pub(crate) trait SessionBackend: Send + Sync {
    fn write(&self, data: &[u8]);
    fn resize(&self, columns: u16, rows: u16);
    fn close(&self);
}

pub struct Session {
    pub kind: SessionKind,
    events: async_channel::Receiver<SessionEvent>,
    backend: Arc<dyn SessionBackend>,
    pid: Option<u32>,
}

impl Session {
    pub(crate) fn new(
        kind: SessionKind,
        events: async_channel::Receiver<SessionEvent>,
        backend: Arc<dyn SessionBackend>,
        pid: Option<u32>,
    ) -> Self {
        Self {
            kind,
            events,
            backend,
            pid,
        }
    }

    pub fn events(&self) -> async_channel::Receiver<SessionEvent> {
        self.events.clone()
    }

    pub fn write(&self, data: &[u8]) {
        self.backend.write(data);
    }

    pub fn resize(&self, columns: u16, rows: u16) {
        self.backend.resize(columns, rows);
    }

    pub fn close(&self) {
        self.backend.close();
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Working directory of the child process, used by "new tab here" like
    /// Tabby's `getWorkingDirectory()`.
    pub fn working_directory(&self) -> Option<PathBuf> {
        let pid = self.pid?;
        std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
    }
}

/// Build a session from a profile's `type` and options.
pub enum NewSession {
    Local(LocalOptions),
    Ssh {
        options: SshOptions,
        password: Option<String>,
    },
    Telnet(TelnetOptions),
}

impl NewSession {
    pub fn kind(&self) -> SessionKind {
        match self {
            Self::Local(_) => SessionKind::Local,
            Self::Ssh { options, .. } => SessionKind::Ssh {
                host: options.host.clone(),
                user: options.user.clone(),
                port: options.port,
            },
            Self::Telnet(options) => SessionKind::Telnet {
                host: options.host.clone(),
                port: options.port,
            },
        }
    }
}

/// Spawn `session` at the given initial terminal size.
pub fn spawn(
    session: NewSession,
    columns: u16,
    rows: u16,
    known_hosts: KnownHostsPolicy,
) -> anyhow::Result<Session> {
    match session {
        NewSession::Local(options) => spawn_local(options, columns, rows),
        NewSession::Ssh { options, password } => {
            spawn_ssh(options, password, columns, rows, known_hosts)
        }
        NewSession::Telnet(options) => spawn_telnet(options),
    }
}
