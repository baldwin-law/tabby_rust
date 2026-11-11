use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_channel::{Receiver, Sender};
use russh::client;
use russh::keys::agent::AgentIdentity;
use russh::keys::{
    HashAlg, PrivateKeyWithHashAlg, PublicKey, check_known_hosts_path, load_secret_key,
};
use russh::{ChannelMsg, Disconnect};

use super::{Session, SessionBackend, SessionEvent, SessionKind};
use crate::config::SshOptions;

/// How server host keys are checked.
#[derive(Clone, Debug)]
pub enum KnownHostsPolicy {
    /// Trust every server key (only for tests or explicit opt-in).
    AcceptAll,
    /// Verify against `path`, recording unseen keys when `accept_new`.
    Verify { path: PathBuf, accept_new: bool },
}

impl KnownHostsPolicy {
    /// Tabby keeps its known hosts in its own store; so do we.
    pub fn tabby_default() -> Self {
        Self::Verify {
            path: crate::config::Config::state_dir().join("known_hosts"),
            accept_new: true,
        }
    }
}

#[derive(Debug)]
struct ServerKeyCheck {
    host: String,
    port: u16,
    policy: KnownHostsPolicy,
    error: Mutex<Option<String>>,
}

impl ServerKeyCheck {
    fn error(&self) -> Option<String> {
        self.error.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set_error(&self, message: impl Into<String>) {
        *self.error.lock().unwrap_or_else(|e| e.into_inner()) = Some(message.into());
    }

    fn host_token(&self) -> String {
        if self.port == 22 {
            self.host.clone()
        } else {
            format!("[{}]:{}", self.host, self.port)
        }
    }

    fn check(&self, key: &PublicKey) -> bool {
        match &self.policy {
            KnownHostsPolicy::AcceptAll => true,
            KnownHostsPolicy::Verify { path, accept_new } => {
                match check_known_hosts_path(&self.host, self.port, key, path) {
                    Ok(true) => true,
                    Ok(false) if *accept_new => {
                        let fingerprint = key.fingerprint(HashAlg::Sha256);
                        match learn_host_key(path, &self.host_token(), key) {
                            Ok(()) => {
                                log::info!(
                                    "learned host key for {}: {fingerprint}",
                                    self.host_token()
                                );
                                true
                            }
                            Err(error) => {
                                self.set_error(format!(
                                    "could not record host key for {}: {error}",
                                    self.host_token()
                                ));
                                false
                            }
                        }
                    }
                    Ok(false) => {
                        self.set_error(format!(
                            "host key for {} is not known (fingerprint {})",
                            self.host_token(),
                            key.fingerprint(HashAlg::Sha256)
                        ));
                        false
                    }
                    Err(error) => {
                        self.set_error(format!(
                            "host key verification failed for {}: {error}",
                            self.host_token()
                        ));
                        false
                    }
                }
            }
        }
    }
}

fn learn_host_key(path: &Path, host_token: &str, key: &PublicKey) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let line = key
        .to_openssh()
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    // Keep the file safe from other users even in a permissive home.
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{host_token} {line}")
}

struct ClientHandler {
    key_check: Arc<ServerKeyCheck>,
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        Ok(self.key_check.check(&server_public_key.public_key()))
    }
}

#[derive(Debug)]
enum SshCommand {
    Write(Vec<u8>),
    Resize(u16, u16),
    Close,
}

struct SshBackend {
    commands: Sender<SshCommand>,
}

impl SessionBackend for SshBackend {
    fn write(&self, data: &[u8]) {
        let _ = self.commands.try_send(SshCommand::Write(data.to_vec()));
    }

    fn resize(&self, columns: u16, rows: u16) {
        let _ = self.commands.try_send(SshCommand::Resize(columns, rows));
    }

    fn close(&self) {
        let _ = self.commands.try_send(SshCommand::Close);
    }
}

fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(path)
}

async fn try_password(
    handle: &mut client::Handle<ClientHandler>,
    user: &str,
    password: Option<&str>,
) -> Result<(), String> {
    let password =
        password.ok_or_else(|| "password authentication requires a password".to_string())?;
    let result = handle
        .authenticate_password(user, password)
        .await
        .map_err(|error| format!("password authentication failed: {error}"))?;
    if result.success() {
        Ok(())
    } else {
        Err("password authentication was rejected".into())
    }
}

async fn try_private_keys(
    handle: &mut client::Handle<ClientHandler>,
    user: &str,
    keys: &[String],
) -> Result<(), String> {
    if keys.is_empty() {
        return Err("no private key configured".into());
    }
    let mut last_error = String::from("no configured private key could be loaded");
    for path in keys {
        let path = expand_tilde(path);
        let key = match load_secret_key(&path, None) {
            Ok(key) => key,
            Err(error) => {
                last_error = format!("could not load {}: {error}", path.display());
                continue;
            }
        };
        let hash_alg = handle
            .best_supported_rsa_hash()
            .await
            .map_err(|error| format!("could not negotiate RSA hash: {error}"))?
            .flatten();
        let result = handle
            .authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key), hash_alg))
            .await
            .map_err(|error| format!("public key authentication failed: {error}"))?;
        if result.success() {
            return Ok(());
        }
        last_error = format!("key {} was rejected", path.display());
    }
    Err(last_error)
}

async fn try_agent(handle: &mut client::Handle<ClientHandler>, user: &str) -> Result<(), String> {
    let mut agent = russh::keys::agent::client::AgentClient::connect_env()
        .await
        .map_err(|error| format!("could not connect to SSH agent: {error}"))?;
    let identities = agent
        .request_identities()
        .await
        .map_err(|error| format!("SSH agent identity request failed: {error}"))?;
    if identities.is_empty() {
        return Err("SSH agent holds no identities".into());
    }
    for identity in identities {
        let key = match identity {
            AgentIdentity::PublicKey { key, .. } => key,
            AgentIdentity::Certificate { certificate, .. } => {
                PublicKey::new(certificate.public_key().clone(), "")
            }
        };
        match handle
            .authenticate_publickey_with(user, key, None, &mut agent)
            .await
        {
            Ok(result) if result.success() => return Ok(()),
            Ok(_) => continue,
            Err(error) => {
                log::debug!("SSH agent signing failed: {error}");
            }
        }
    }
    Err("SSH agent authentication was rejected".into())
}

async fn authenticate(
    handle: &mut client::Handle<ClientHandler>,
    options: &SshOptions,
    password: Option<String>,
) -> Result<(), String> {
    let requested = options.auth.as_deref();
    let mut errors = Vec::new();

    let mut attempt = |name: &str, result: Result<(), String>| -> Result<(), String> {
        match result {
            Ok(()) => Ok(()),
            Err(error) => {
                errors.push(format!("{name}: {error}"));
                Err(error)
            }
        }
    };

    // Explicit method choices fail loudly rather than silently falling back.
    match requested {
        Some("password") => {
            return attempt(
                "password",
                try_password(handle, &options.user, password.as_deref()).await,
            );
        }
        Some("publicKey") | Some("publickey") => {
            return attempt(
                "publickey",
                try_private_keys(handle, &options.user, &options.private_keys).await,
            );
        }
        Some("agent") => return attempt("agent", try_agent(handle, &options.user).await),
        _ => {}
    }

    // Automatic order: password, then keys, then the agent.
    if password.is_some() {
        if attempt(
            "password",
            try_password(handle, &options.user, password.as_deref()).await,
        )
        .is_ok()
        {
            return Ok(());
        }
    }
    if !options.private_keys.is_empty()
        && attempt(
            "publickey",
            try_private_keys(handle, &options.user, &options.private_keys).await,
        )
        .is_ok()
    {
        return Ok(());
    }
    if std::env::var_os("SSH_AUTH_SOCK").is_some()
        && attempt("agent", try_agent(handle, &options.user).await).is_ok()
    {
        return Ok(());
    }

    Err(if errors.is_empty() {
        "no usable credentials: provide a password, a private key or an SSH agent".into()
    } else {
        errors.join("; ")
    })
}

async fn run_ssh(
    options: SshOptions,
    password: Option<String>,
    columns: u16,
    rows: u16,
    policy: KnownHostsPolicy,
    events: Sender<SessionEvent>,
    commands: Receiver<SshCommand>,
) {
    let key_check = Arc::new(ServerKeyCheck {
        host: options.host.clone(),
        port: options.port,
        policy,
        error: Mutex::new(None),
    });
    let handler = ClientHandler {
        key_check: key_check.clone(),
    };

    let config = Arc::new(client::Config {
        inactivity_timeout: None,
        ..Default::default()
    });

    let mut handle =
        match client::connect(config, (options.host.as_str(), options.port), handler).await {
            Ok(handle) => handle,
            Err(error) => {
                let message = key_check.error().unwrap_or_else(|| {
                    format!(
                        "could not connect to {}:{}: {error}",
                        options.host, options.port
                    )
                });
                let _ = events.try_send(SessionEvent::Error(message));
                let _ = events.try_send(SessionEvent::Closed);
                return;
            }
        };

    if let Err(error) = authenticate(&mut handle, &options, password).await {
        let _ = events.try_send(SessionEvent::Error(format!(
            "SSH authentication for {}@{} failed: {error}",
            options.user, options.host
        )));
        let _ = events.try_send(SessionEvent::Closed);
        let _ = handle.disconnect(Disconnect::ByApplication, "", "en").await;
        return;
    }

    let session_result = async {
        let channel = handle.channel_open_session().await?;
        channel
            .request_pty(
                false,
                &options.term,
                u32::from(columns),
                u32::from(rows),
                0,
                0,
                &[],
            )
            .await?;
        channel.request_shell(true).await?;
        Ok::<_, russh::Error>(channel)
    }
    .await;

    let mut channel = match session_result {
        Ok(channel) => channel,
        Err(error) => {
            let _ = events.try_send(SessionEvent::Error(format!(
                "could not open a shell on {}: {error}",
                options.host
            )));
            let _ = events.try_send(SessionEvent::Closed);
            return;
        }
    };

    loop {
        tokio::select! {
            command = commands.recv() => {
                match command {
                    Ok(SshCommand::Write(data)) => {
                        if let Err(error) = channel.data_bytes(data).await {
                            log::debug!("SSH write failed: {error}");
                            break;
                        }
                    }
                    Ok(SshCommand::Resize(columns, rows)) => {
                        let _ = channel
                            .window_change(u32::from(columns), u32::from(rows), 0, 0)
                            .await;
                    }
                    Ok(SshCommand::Close) | Err(_) => {
                        let _ = channel.eof().await;
                        break;
                    }
                }
            }
            message = channel.wait() => {
                match message {
                    Some(ChannelMsg::Data { data }) | Some(ChannelMsg::ExtendedData { data, .. }) => {
                        let _ = events.try_send(SessionEvent::Data(data.to_vec()));
                    }
                    Some(ChannelMsg::ExitStatus { exit_status }) => {
                        let _ = events.try_send(SessionEvent::Exited(Some(exit_status as i32)));
                    }
                    Some(ChannelMsg::ExitSignal { signal_name, .. }) => {
                        let _ = events.try_send(SessionEvent::Error(format!(
                            "remote process terminated by signal {signal_name:?}"
                        )));
                    }
                    Some(ChannelMsg::Close) | Some(ChannelMsg::Eof) | None => break,
                    Some(_) => {}
                }
            }
        }
    }

    let _ = events.try_send(SessionEvent::Closed);
    let _ = handle.disconnect(Disconnect::ByApplication, "", "en").await;
}

/// Connect an SSH shell. Authentication happens on the background runtime;
/// failures arrive as `SessionEvent::Error` followed by `Closed`.
pub fn spawn_ssh(
    options: SshOptions,
    password: Option<String>,
    columns: u16,
    rows: u16,
    policy: KnownHostsPolicy,
) -> anyhow::Result<Session> {
    if options.host.trim().is_empty() {
        anyhow::bail!("SSH host is empty");
    }
    let (event_tx, event_rx) = async_channel::unbounded();
    let (command_tx, command_rx) = async_channel::unbounded();
    let kind = SessionKind::Ssh {
        host: options.host.clone(),
        user: options.user.clone(),
        port: options.port,
    };

    let thread_events = event_tx.clone();
    std::thread::Builder::new()
        .name("tabby-ssh".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build();
            match runtime {
                Ok(runtime) => {
                    runtime.block_on(run_ssh(
                        options,
                        password,
                        columns,
                        rows,
                        policy,
                        thread_events.clone(),
                        command_rx,
                    ));
                }
                Err(error) => {
                    let _ = thread_events.try_send(SessionEvent::Error(format!(
                        "could not start SSH runtime: {error}"
                    )));
                    let _ = thread_events.try_send(SessionEvent::Closed);
                }
            }
        })?;

    Ok(Session::new(
        kind,
        event_rx,
        Arc::new(SshBackend {
            commands: command_tx,
        }),
        None,
    ))
}
