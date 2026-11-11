//! Core of the Tabby Rust reimplementation.
//!
//! * [`terminal`] — the VT engine (alacritty_terminal) and colour resolution.
//! * [`session`] — local PTYs, SSH and telnet backends.
//! * [`config`] — Tabby-compatible `config.yaml`, profiles and shell discovery.
//! * [`colors`] — colour schemes and the xterm 256-colour palette.
//! * [`recovery`] — tab/layout persistence.

pub use alacritty_terminal::{event, grid, index, selection, term, vte};

pub mod colors;
pub mod config;
pub mod recovery;
pub mod session;
pub mod terminal;

pub use colors::{ColorScheme, Rgb8};
pub use config::{Config, LocalOptions, Profile, SshOptions, TelnetOptions};
pub use session::{KnownHostsPolicy, NewSession, Session, SessionEvent, SessionKind};
pub use terminal::{TermSize, Terminal};
