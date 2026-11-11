//! Tabby-compatible configuration.
//!
//! The file lives at `~/.config/tabby-rust/config.yaml` (override with
//! `TABBY_RUST_CONFIG`) and deliberately uses the same key names as Tabby, so a
//! Tabby `config.yaml` can be dropped in and the local/SSH profiles keep
//! working. Keys this reimplementation does not understand are preserved
//! verbatim through `#[serde(flatten)]`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};

use crate::colors::ColorScheme;

pub const CONFIG_FILE_NAME: &str = "config.yaml";
pub const CONFIG_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not parse {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("could not write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not serialize configuration: {0}")]
    Serialize(#[from] serde_yaml::Error),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SearchOptions {
    pub regex: bool,
    pub whole_word: bool,
    pub case_sensitive: bool,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            regex: false,
            whole_word: false,
            case_sensitive: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TerminalSettings {
    /// Font family, matching Tabby's `terminal.font` key.
    pub font: String,
    pub font_size: f32,
    pub font_weight: u16,
    pub font_weight_bold: u16,
    pub line_padding: f32,
    pub fallback_font: Option<String>,
    /// `block`, `beam` or `underline`.
    pub cursor: String,
    pub cursor_blink: bool,
    pub scrollback_lines: usize,
    /// `off`, `visual` or `sound` — we flash the pane for `visual`/`sound`.
    pub bell: String,
    pub bracketed_paste: bool,
    pub copy_on_select: bool,
    pub scroll_on_input: bool,
    pub alt_is_meta: bool,
    pub right_click: String,
    pub paste_on_middle_click: bool,
    pub draw_bold_text_in_bright_colors: bool,
    pub minimum_contrast_ratio: f32,
    pub ligatures: bool,
    pub word_separator: String,
    pub warn_on_multiline_paste: bool,
    pub replace_newlines_with_spaces_on_paste: bool,
    pub trim_whitespace_on_paste: bool,
    pub search_regex_always_enabled: bool,
    pub search_options: SearchOptions,
    pub color_scheme: ColorScheme,
    pub light_color_scheme: ColorScheme,
    pub custom_color_schemes: Vec<ColorScheme>,
    #[serde(flatten)]
    pub extra: Mapping,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            font: default_terminal_font().into(),
            font_size: 14.0,
            font_weight: 400,
            font_weight_bold: 700,
            line_padding: 0.0,
            fallback_font: None,
            cursor: "block".into(),
            cursor_blink: true,
            scrollback_lines: 25_000,
            bell: "off".into(),
            bracketed_paste: true,
            copy_on_select: false,
            scroll_on_input: true,
            alt_is_meta: false,
            right_click: "menu".into(),
            paste_on_middle_click: true,
            draw_bold_text_in_bright_colors: true,
            minimum_contrast_ratio: 4.0,
            ligatures: false,
            word_separator: " ()[]{}\"'".into(),
            warn_on_multiline_paste: true,
            replace_newlines_with_spaces_on_paste: false,
            trim_whitespace_on_paste: true,
            search_regex_always_enabled: false,
            search_options: SearchOptions::default(),
            color_scheme: ColorScheme::tabby_default(),
            light_color_scheme: ColorScheme::tabby_light(),
            custom_color_schemes: Vec::new(),
            extra: Mapping::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AppearanceSettings {
    pub theme: String,
    /// `dark` or `light`; selects which of the two schemes is active.
    pub color_scheme_mode: String,
    pub tabs_location: String,
    pub opacity: f32,
    pub spaciness: f32,
    pub flex_tabs: bool,
    pub cycle_tabs: bool,
    pub last_tab_closes_window: bool,
    #[serde(flatten)]
    pub extra: Mapping,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: "Follow the color scheme".into(),
            color_scheme_mode: "dark".into(),
            tabs_location: "top".into(),
            opacity: 1.0,
            spaciness: 1.0,
            flex_tabs: false,
            cycle_tabs: true,
            last_tab_closes_window: false,
            extra: Mapping::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Provider-specific options, kept as YAML so unknown plugins' profiles
    /// survive a round-trip.
    pub options: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub behavior_on_session_end: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub is_builtin: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub is_template: bool,
    pub weight: i32,
    #[serde(flatten)]
    pub extra: Mapping,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            id: String::new(),
            kind: String::new(),
            name: String::new(),
            group: None,
            icon: None,
            color: None,
            options: Value::Mapping(Mapping::new()),
            behavior_on_session_end: None,
            is_builtin: false,
            is_template: false,
            weight: 0,
            extra: Mapping::new(),
        }
    }
}

impl Profile {
    pub fn new_local(name: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            id: format!("local:custom:{}", uuid::Uuid::new_v4()),
            kind: "local".into(),
            name: name.into(),
            options: serde_yaml::to_value(LocalOptions {
                command: command.into(),
                ..Default::default()
            })
            .unwrap_or(Value::Null),
            ..Default::default()
        }
    }

    pub fn new_ssh(
        name: impl Into<String>,
        host: impl Into<String>,
        user: impl Into<String>,
        port: u16,
    ) -> Self {
        Self {
            id: format!("ssh:custom:{}", uuid::Uuid::new_v4()),
            kind: "ssh".into(),
            name: name.into(),
            options: serde_yaml::to_value(SshOptions {
                host: host.into(),
                user: user.into(),
                port,
                ..Default::default()
            })
            .unwrap_or(Value::Null),
            ..Default::default()
        }
    }

    pub fn option(&self, key: &str) -> Option<&Value> {
        self.options
            .as_mapping()?
            .get(Value::String(key.to_string()))
    }

    pub fn option_str(&self, key: &str) -> Option<&str> {
        self.option(key).and_then(Value::as_str)
    }

    pub fn option_bool(&self, key: &str) -> Option<bool> {
        self.option(key).and_then(Value::as_bool)
    }

    pub fn option_u64(&self, key: &str) -> Option<u64> {
        self.option(key).and_then(Value::as_u64)
    }

    pub fn option_string_list(&self, key: &str) -> Vec<String> {
        self.option(key)
            .and_then(Value::as_sequence)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn has_credentials(&self) -> bool {
        self.option_str("password").is_some_and(|p| !p.is_empty())
            || !self.option_string_list("privateKeys").is_empty()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct LocalOptions {
    pub command: String,
    pub args: Vec<String>,
    pub home_dir_args: Vec<String>,
    pub cwd: Option<String>,
    pub env: Mapping,
    pub shell_type: Option<String>,
    pub pause_after_exit: bool,
}

impl Default for LocalOptions {
    fn default() -> Self {
        Self {
            command: String::new(),
            args: Vec::new(),
            home_dir_args: Vec::new(),
            cwd: None,
            env: Mapping::new(),
            shell_type: Some("unix".into()),
            pause_after_exit: false,
        }
    }
}

impl LocalOptions {
    pub fn from_profile(profile: &Profile) -> Self {
        serde_yaml::from_value(profile.options.clone()).unwrap_or_else(|_| Self {
            command: profile
                .option_str("command")
                .unwrap_or_default()
                .to_string(),
            args: profile.option_string_list("args"),
            cwd: profile.option_str("cwd").map(ToOwned::to_owned),
            ..Default::default()
        })
    }

    pub fn command_and_args(&self) -> (String, Vec<String>) {
        let command = if self.command.is_empty() {
            default_shell_command()
        } else {
            self.command.clone()
        };
        (command, self.args.clone())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SshOptions {
    pub host: String,
    pub port: u16,
    pub user: String,
    /// `password`, `publicKey`, `agent` or `keyboard-interactive`; `None` means
    /// "use whatever credentials are present".
    pub auth: Option<String>,
    pub password: Option<String>,
    pub private_keys: Vec<String>,
    pub keepalive_interval: u64,
    pub keepalive_count_max: u32,
    pub ready_timeout: Option<u64>,
    pub x11: bool,
    pub skip_banner: bool,
    pub jump_host: Option<String>,
    pub agent_forward: bool,
    pub proxy_command: Option<String>,
    pub term: String,
    pub cwd: Option<String>,
    pub remember_cwd: bool,
    pub reuse_session: bool,
    pub scripts: Vec<Value>,
    pub forwarded_ports: Vec<Value>,
}

impl Default for SshOptions {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 22,
            user: "root".into(),
            auth: None,
            password: None,
            private_keys: Vec::new(),
            keepalive_interval: 5000,
            keepalive_count_max: 10,
            ready_timeout: None,
            x11: false,
            skip_banner: false,
            jump_host: None,
            agent_forward: false,
            proxy_command: None,
            term: "xterm-256color".into(),
            cwd: None,
            remember_cwd: false,
            reuse_session: true,
            scripts: Vec::new(),
            forwarded_ports: Vec::new(),
        }
    }
}

impl SshOptions {
    pub fn from_profile(profile: &Profile) -> Self {
        let mut options: Self = serde_yaml::from_value(profile.options.clone()).unwrap_or_default();
        // Providers commonly leave `auth` out entirely.
        if options.user.is_empty() {
            options.user = "root".into();
        }
        if options.port == 0 {
            options.port = 22;
        }
        options
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TelnetOptions {
    pub host: String,
    pub port: u16,
    pub input_mode: Option<String>,
    pub output_mode: Option<String>,
    pub output_newlines: Option<String>,
}

impl Default for TelnetOptions {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 23,
            input_mode: Some("local-echo".into()),
            output_mode: None,
            output_newlines: Some("crlf".into()),
        }
    }
}

impl TelnetOptions {
    pub fn from_profile(profile: &Profile) -> Self {
        serde_yaml::from_value(profile.options.clone()).unwrap_or_default()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProfileGroup {
    pub id: String,
    pub name: String,
    pub parent_group_id: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub editable: bool,
    #[serde(flatten)]
    pub extra: Mapping,
}

impl Default for ProfileGroup {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            parent_group_id: None,
            icon: None,
            color: None,
            editable: true,
            extra: Mapping::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub version: u32,
    pub terminal: TerminalSettings,
    pub appearance: AppearanceSettings,
    pub profiles: Vec<Profile>,
    pub groups: Vec<ProfileGroup>,
    /// Reopen the tabs that were open when the app last closed.
    pub recover_tabs: bool,
    pub enable_welcome_tab: bool,
    pub default_quick_connect_provider: String,
    pub language: Option<String>,
    pub hotkeys: Value,
    #[serde(flatten)]
    pub extra: Mapping,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            terminal: TerminalSettings::default(),
            appearance: AppearanceSettings::default(),
            profiles: Vec::new(),
            groups: Vec::new(),
            recover_tabs: true,
            enable_welcome_tab: false,
            default_quick_connect_provider: "ssh".into(),
            language: None,
            hotkeys: Value::Mapping(Mapping::new()),
            extra: Mapping::new(),
        }
    }
}

impl Config {
    pub fn default_path() -> PathBuf {
        if let Ok(path) = std::env::var("TABBY_RUST_CONFIG") {
            return PathBuf::from(path);
        }
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("tabby-rust")
            .join(CONFIG_FILE_NAME)
    }

    pub fn state_dir() -> PathBuf {
        Self::default_path()
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    pub fn load() -> Result<Self, ConfigError> {
        Self::load_from(&Self::default_path())
    }

    pub fn load_from(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let config: Self = serde_yaml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(config)
    }

    /// Load the user's config, falling back to defaults when the file is
    /// missing or malformed. A malformed file is *not* overwritten until the
    /// caller saves explicitly.
    pub fn load_or_default() -> Self {
        match Self::load() {
            Ok(config) => config,
            Err(ConfigError::Read { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                Self::default()
            }
            Err(error) => {
                log::warn!("using default configuration: {error}");
                Self::default()
            }
        }
    }

    pub fn save(&self) -> Result<(), ConfigError> {
        self.save_to(&Self::default_path())
    }

    pub fn save_to(&self, path: &Path) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let text = serde_yaml::to_string(self)?;
        let temp = path.with_extension(format!("yaml.{}.tmp", std::process::id()));
        std::fs::write(&temp, text).map_err(|source| ConfigError::Write {
            path: temp.clone(),
            source,
        })?;
        std::fs::rename(&temp, path).map_err(|source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(())
    }

    /// The scheme selected by `appearance.colorSchemeMode`.
    pub fn active_color_scheme(&self) -> &ColorScheme {
        if self
            .appearance
            .color_scheme_mode
            .eq_ignore_ascii_case("light")
        {
            &self.terminal.light_color_scheme
        } else {
            &self.terminal.color_scheme
        }
    }

    pub fn scheme_by_name(&self, name: &str) -> Option<ColorScheme> {
        ColorScheme::builtin_schemes()
            .into_iter()
            .chain(self.terminal.custom_color_schemes.iter().cloned())
            .chain([
                self.terminal.color_scheme.clone(),
                self.terminal.light_color_scheme.clone(),
            ])
            .find(|scheme| scheme.name == name)
    }

    pub fn find_profile(&self, id: &str) -> Option<Profile> {
        self.profiles
            .iter()
            .cloned()
            .chain(builtin_profiles())
            .find(|profile| profile.id == id)
    }

    /// Profiles shown in the new-tab picker, custom profiles first. Built-in
    /// templates are included but hidden shells are not.
    pub fn selectable_profiles(&self) -> Vec<Profile> {
        let mut profiles: Vec<Profile> = self.profiles.clone();
        profiles.extend(builtin_profiles().into_iter().filter(|p| !p.is_template));
        profiles
    }

    pub fn upsert_profile(&mut self, profile: Profile) {
        if let Some(existing) = self.profiles.iter_mut().find(|p| p.id == profile.id) {
            *existing = profile;
        } else {
            self.profiles.push(profile);
        }
    }

    pub fn remove_profile(&mut self, id: &str) {
        self.profiles.retain(|profile| profile.id != id);
    }
}

/// A shell Tabby would expose as a built-in local profile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredShell {
    pub id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub hidden: bool,
}

fn slugify(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut last_dash = true;
    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            result.push(ch);
            last_dash = false;
        } else if !last_dash {
            result.push('-');
            last_dash = true;
        }
    }
    while result.ends_with('-') {
        result.pop();
    }
    result
}

/// The platform's default shell, used when a local profile has no command.
pub fn default_shell_command() -> String {
    #[cfg(windows)]
    {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
    }
}

fn default_terminal_font() -> &'static str {
    if cfg!(target_os = "macos") {
        "Menlo"
    } else if cfg!(target_os = "windows") {
        "Consolas"
    } else {
        "Liberation Mono"
    }
}

#[cfg(not(windows))]
fn login_shell() -> Option<String> {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .ok()?;
    let passwd = std::fs::read_to_string("/etc/passwd").ok()?;
    passwd.lines().find_map(|line| {
        let mut fields = line.split(':');
        if fields.next()? != user {
            return None;
        }
        fields.nth(5).map(ToOwned::to_owned)
    })
}

fn shell_display_name(command: &str) -> String {
    let base = command.rsplit(['/', '\\']).next().unwrap_or(command);
    match base.to_ascii_lowercase().as_str() {
        "bash" => "Bash".into(),
        "zsh" => "Zsh".into(),
        "fish" => "Fish".into(),
        "sh" | "dash" | "ash" => "POSIX shell".into(),
        "pwsh" | "pwsh.exe" => "PowerShell 7".into(),
        "powershell" | "powershell.exe" => "Windows PowerShell".into(),
        "cmd" | "cmd.exe" => "Command Prompt".into(),
        "nu" | "nu.exe" => "Nushell".into(),
        "elvish" => "Elvish".into(),
        "ksh" | "mksh" | "oksh" => "Korn shell".into(),
        "tcsh" | "csh" => "C shell".into(),
        _ => base.to_string(),
    }
}

/// Mirror Tabby's Unix shell discovery: the login shell as a hidden
/// `local:default` profile plus every entry from `/etc/shells` with `-l`.
#[cfg(not(windows))]
pub fn discover_shells() -> Vec<DiscoveredShell> {
    let mut shells = Vec::new();
    let login = login_shell()
        .or_else(|| std::env::var("SHELL").ok())
        .unwrap_or_else(|| {
            if cfg!(target_os = "macos") {
                "/bin/zsh".into()
            } else {
                "/bin/sh".into()
            }
        });
    let login_args = if cfg!(target_os = "macos") {
        vec!["-l".into()]
    } else {
        vec!["--login".into()]
    };
    shells.push(DiscoveredShell {
        id: "default".into(),
        name: "User default".into(),
        command: login,
        args: login_args,
        hidden: true,
    });

    let shells_path = ["/etc/shells", "/usr/share/defaults/etc/shells"]
        .into_iter()
        .find(|path| Path::new(path).exists());
    if let Some(path) = shells_path {
        if let Ok(text) = std::fs::read_to_string(path) {
            let mut seen = std::collections::HashSet::new();
            for command in text
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
            {
                if !seen.insert(command.to_string()) {
                    continue;
                }
                shells.push(DiscoveredShell {
                    id: slugify(command),
                    name: shell_display_name(command),
                    command: command.to_string(),
                    args: vec!["-l".into()],
                    hidden: false,
                });
            }
        }
    }
    shells
}

/// Windows discovery: cmd.exe, Windows PowerShell and PowerShell 7 when
/// installed. Tabby gets these from its Windows-specific providers; the Rust
/// port keeps the same built-in profile ids so `local:default` etc. still work.
#[cfg(windows)]
pub fn discover_shells() -> Vec<DiscoveredShell> {
    let mut shells = Vec::new();
    let comspec = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into());
    shells.push(DiscoveredShell {
        id: "default".into(),
        name: "User default".into(),
        command: comspec.clone(),
        args: Vec::new(),
        hidden: true,
    });
    shells.push(DiscoveredShell {
        id: "cmd".into(),
        name: "Command Prompt".into(),
        command: comspec,
        args: Vec::new(),
        hidden: false,
    });

    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    let windows_powershell = Path::new(&system_root)
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    if windows_powershell.is_file() {
        shells.push(DiscoveredShell {
            id: "powershell".into(),
            name: "Windows PowerShell".into(),
            command: windows_powershell.to_string_lossy().into_owned(),
            args: Vec::new(),
            hidden: false,
        });
    }
    if let Some(pwsh) = find_in_path("pwsh.exe") {
        shells.push(DiscoveredShell {
            id: "pwsh".into(),
            name: "PowerShell 7".into(),
            command: pwsh.to_string_lossy().into_owned(),
            args: Vec::new(),
            hidden: false,
        });
    }
    shells
}

#[cfg(windows)]
fn find_in_path(executable: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(executable))
        .find(|candidate| candidate.is_file())
}

pub fn builtin_profiles() -> Vec<Profile> {
    let mut profiles = Vec::new();
    for shell in discover_shells() {
        let shell_type = match shell.id.as_str() {
            "cmd" => "cmd",
            "pwsh" | "powershell" => "powershell",
            _ => "unix",
        };
        let options = LocalOptions {
            command: shell.command.clone(),
            args: shell.args.clone(),
            shell_type: Some(shell_type.into()),
            ..Default::default()
        };
        profiles.push(Profile {
            id: format!("local:{}", shell.id),
            kind: "local".into(),
            name: shell.name,
            options: serde_yaml::to_value(options).unwrap_or(Value::Null),
            is_builtin: true,
            weight: if shell.hidden { -1 } else { 0 },
            ..Default::default()
        });
    }

    profiles.push(Profile {
        id: "ssh:template".into(),
        kind: "ssh".into(),
        name: "SSH connection".into(),
        options: serde_yaml::to_value(SshOptions::default()).unwrap_or(Value::Null),
        is_builtin: true,
        is_template: true,
        weight: -1,
        ..Default::default()
    });
    profiles.push(Profile {
        id: "telnet:template".into(),
        kind: "telnet".into(),
        name: "Telnet session".into(),
        options: serde_yaml::to_value(TelnetOptions::default()).unwrap_or(Value::Null),
        is_builtin: true,
        is_template: true,
        weight: -1,
        ..Default::default()
    });
    profiles
}

/// Parse the SSH quick-connect grammar Tabby accepts:
/// `host`, `user@host`, `host:port`, `user@[v6]:port`.
pub fn parse_ssh_quick_connect(query: &str) -> SshOptions {
    let mut rest = query.trim();
    let mut user = None;
    if let Some((candidate, host)) = rest.rsplit_once('@') {
        if !candidate.is_empty() {
            user = Some(candidate.to_string());
            rest = host;
        }
    }

    let mut host = rest.to_string();
    let mut port = 22u16;
    if let Some(stripped) = rest.strip_prefix('[') {
        if let Some((inside, after)) = stripped.split_once(']') {
            host = inside.to_string();
            if let Some(port_text) = after.strip_prefix(':') {
                if let Ok(parsed) = port_text.parse() {
                    port = parsed;
                }
            }
        }
    } else if let Some((candidate_host, port_text)) = rest.rsplit_once(':') {
        if let Ok(parsed) = port_text.parse::<u16>() {
            host = candidate_host.to_string();
            port = parsed;
        }
    }

    SshOptions {
        host,
        port,
        user: user.unwrap_or_else(|| "root".into()),
        ..Default::default()
    }
}

/// Parse `host` or `host:port` for telnet/raw sockets.
pub fn parse_telnet_quick_connect(query: &str, default_port: u16) -> TelnetOptions {
    let mut host = query.trim().to_string();
    let mut port = default_port;
    if let Some((candidate_host, port_text)) = host.rsplit_once(':') {
        if let Ok(parsed) = port_text.parse::<u16>() {
            host = candidate_host.to_string();
            port = parsed;
        }
    }
    TelnetOptions {
        host,
        port,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_roundtrips_and_preserves_unknown_keys() {
        let yaml = r##"
version: 1
appearance:
  theme: "Follow the color scheme"
  dock: off
  futureSetting: 42
terminal:
  fontSize: 15
  colorScheme:
    name: Custom
    foreground: "#ffffff"
    background: "#000000"
    cursor: "#ff0000"
    colors: ["#000000","#111111","#222222","#333333","#444444","#555555","#666666","#777777","#888888","#999999","#aaaaaa","#bbbbbb","#cccccc","#dddddd","#eeeeee","#ffffff"]
profiles:
  - id: local:test
    type: local
    name: Test
    options:
      command: /bin/sh
      args: ["-l"]
      unknownPluginField: true
futureTopLevel: hello
"##;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.terminal.font_size, 15.0);
        assert_eq!(config.profiles.len(), 1);
        assert_eq!(config.terminal.color_scheme.name, "Custom");
        let out = serde_yaml::to_string(&config).unwrap();
        let again: Config = serde_yaml::from_str(&out).unwrap();
        assert_eq!(
            again.extra.get("futureTopLevel").unwrap().as_str(),
            Some("hello")
        );
        assert_eq!(
            again.terminal.extra.get("fontSize"),
            None,
            "fontSize is a typed field, not a flattened one"
        );
        assert_eq!(
            again.profiles[0].option_bool("unknownPluginField"),
            Some(true)
        );
        let saved = serde_yaml::to_string(&again).unwrap();
        assert!(saved.contains("futureTopLevel: hello"));
        assert!(saved.contains("unknownPluginField: true"));
    }

    #[test]
    fn missing_colorscheme_fields_get_tabby_defaults() {
        let yaml = r#"
terminal:
  colorScheme:
    name: Only name
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.terminal.color_scheme.foreground, "#cacaca");
        assert_eq!(config.terminal.color_scheme.colors.len(), 16);
    }

    #[test]
    fn quick_connect_parses_users_ports_and_ipv6() {
        let options = parse_ssh_quick_connect("alice@example.com:2222");
        assert_eq!(options.user, "alice");
        assert_eq!(options.host, "example.com");
        assert_eq!(options.port, 2222);

        let options = parse_ssh_quick_connect("root@[::1]:2200");
        assert_eq!(options.host, "::1");
        assert_eq!(options.port, 2200);

        let options = parse_ssh_quick_connect("example.com");
        assert_eq!(options.user, "root");
        assert_eq!(options.port, 22);
    }

    #[test]
    fn save_writes_atomically_and_loads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/config.yaml");
        let mut config = Config::default();
        config.terminal.font_size = 18.0;
        config.save_to(&path).unwrap();
        let loaded = Config::load_from(&path).unwrap();
        assert_eq!(loaded.terminal.font_size, 18.0);
        assert_eq!(loaded.terminal.color_scheme, ColorScheme::tabby_default());
    }

    #[test]
    fn tabby_ssh_profile_options_are_understood() {
        let yaml = r#"
profiles:
  - id: ssh:prod
    type: ssh
    name: prod
    options:
      host: example.com
      port: 2200
      user: deploy
      auth: publicKey
      privateKeys: ["~/.ssh/id_ed25519"]
      keepaliveInterval: 15000
      term: screen-256color
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let profile = &config.profiles[0];
        assert_eq!(profile.kind, "ssh");
        assert!(profile.has_credentials());
        let options = SshOptions::from_profile(profile);
        assert_eq!(options.host, "example.com");
        assert_eq!(options.port, 2200);
        assert_eq!(options.user, "deploy");
        assert_eq!(options.private_keys, vec!["~/.ssh/id_ed25519"]);
        assert_eq!(options.keepalive_interval, 15000);
        assert_eq!(options.term, "screen-256color");
    }

    #[test]
    fn tabby_local_profile_options_are_understood() {
        let yaml = r#"
profiles:
  - id: local:custom
    type: local
    name: Project shell
    options:
      command: /usr/bin/fish
      args: ["-l"]
      cwd: /tmp
      env:
        FOO: bar
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let options = LocalOptions::from_profile(&config.profiles[0]);
        assert_eq!(options.command, "/usr/bin/fish");
        assert_eq!(options.args, vec!["-l"]);
        assert_eq!(options.cwd.as_deref(), Some("/tmp"));
        assert!(
            options
                .env
                .values()
                .any(|value| value.as_str() == Some("bar"))
        );
    }

    #[test]
    fn top_level_and_profile_keys_use_tabby_casing() {
        let yaml = r#"
version: 1
recoverTabs: false
enableWelcomeTab: false
defaultQuickConnectProvider: ssh
profiles:
  - id: ssh:one
    type: ssh
    name: one
    behaviorOnSessionEnd: close
    isBuiltin: true
    options: {}
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert!(!config.recover_tabs);
        assert_eq!(
            config.profiles[0].behavior_on_session_end.as_deref(),
            Some("close")
        );
        assert!(config.profiles[0].is_builtin);
        let saved = serde_yaml::to_string(&config).unwrap();
        assert!(saved.contains("recoverTabs: false"), "{saved}");
        assert!(saved.contains("behaviorOnSessionEnd: close"), "{saved}");
        assert!(!saved.contains("recover_tabs"), "{saved}");
    }

    #[test]
    fn platform_shell_names_and_defaults_are_sane() {
        assert_eq!(
            shell_display_name("C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"),
            "Windows PowerShell"
        );
        assert_eq!(shell_display_name("/usr/bin/fish"), "Fish");
        assert_eq!(shell_display_name("/opt/bin/MyShell"), "MyShell");
        assert!(!default_shell_command().is_empty());
        assert!(!default_terminal_font().is_empty());
    }

    #[test]
    fn builtin_profiles_have_login_shell() {
        let profiles = builtin_profiles();
        assert!(profiles.iter().any(|p| p.id == "local:default"));
        assert!(profiles.iter().any(|p| p.id == "ssh:template"));
    }
}
