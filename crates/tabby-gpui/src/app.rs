use std::sync::atomic::{AtomicU64, Ordering};

use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, IntoElement, KeyBinding, MouseButton,
    SharedString, Subscription, Window, actions, div, prelude::*, px, relative, rgba,
};

use tabby_core::config::{
    Config, LocalOptions, Profile, SshOptions, TelnetOptions, builtin_profiles,
    default_shell_command, parse_ssh_quick_connect, parse_telnet_quick_connect,
};
use tabby_core::recovery::{RecoveredAxis, RecoveredLayout, RecoveredPane, RecoveryState};
use tabby_core::{ColorScheme, KnownHostsPolicy, NewSession, SessionKind};

use crate::pane::{ScrollCommand, TerminalPane};
use crate::text_input::TextInput;
use crate::theme::Theme;

static NEXT_PANE_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_TAB_ID: AtomicU64 = AtomicU64::new(1);

fn next_pane_id() -> u64 {
    NEXT_PANE_ID.fetch_add(1, Ordering::Relaxed)
}

actions!(
    tabby,
    [
        NewTab,
        CloseTab,
        NextTab,
        PrevTab,
        SplitRight,
        SplitDown,
        ClosePane,
        FocusPaneLeft,
        FocusPaneRight,
        FocusPaneUp,
        FocusPaneDown,
        OpenProfilePicker,
        ToggleSettings,
        Search,
        FindNext,
        FindPrev,
        CloseSearch,
        Copy,
        Paste,
        SelectAll,
        ZoomIn,
        ZoomOut,
        ResetZoom,
        ScrollPageUp,
        ScrollPageDown,
        ScrollLineUp,
        ScrollLineDown,
        ScrollTop,
        ScrollBottom,
        SubmitDialog,
        CancelDialog,
        DialogUp,
        DialogDown,
        DialogTab,
        DialogBackTab,
        QuitApp,
    ]
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Clone)]
enum SplitNode {
    Pane {
        id: u64,
        entity: Entity<TerminalPane>,
    },
    Split {
        axis: Axis,
        ratio: f32,
        first: Box<SplitNode>,
        second: Box<SplitNode>,
    },
}

impl SplitNode {
    fn first_pane_id(&self) -> u64 {
        match self {
            Self::Pane { id, .. } => *id,
            Self::Split { first, .. } => first.first_pane_id(),
        }
    }

    fn find_pane(&self, id: u64) -> Option<Entity<TerminalPane>> {
        match self {
            Self::Pane {
                id: pane_id,
                entity,
            } if *pane_id == id => Some(entity.clone()),
            Self::Split { first, second, .. } => {
                first.find_pane(id).or_else(|| second.find_pane(id))
            }
            _ => None,
        }
    }

    fn replace_pane(&mut self, id: u64, replacement: SplitNode) -> bool {
        match self {
            Self::Pane { id: pane_id, .. } if *pane_id == id => {
                *self = replacement;
                true
            }
            Self::Split { first, second, .. } => {
                first.replace_pane(id, replacement.clone()) || second.replace_pane(id, replacement)
            }
            _ => false,
        }
    }

    fn pane_ids(&self, output: &mut Vec<u64>) {
        match self {
            Self::Pane { id, .. } => output.push(*id),
            Self::Split { first, second, .. } => {
                first.pane_ids(output);
                second.pane_ids(output);
            }
        }
    }

    fn entities(&self, output: &mut Vec<Entity<TerminalPane>>) {
        match self {
            Self::Pane { entity, .. } => output.push(entity.clone()),
            Self::Split { first, second, .. } => {
                first.entities(output);
                second.entities(output);
            }
        }
    }

    fn remove(self, id: u64) -> Option<SplitNode> {
        match self {
            Self::Pane { id: pane_id, .. } if pane_id == id => None,
            Self::Pane { .. } => Some(self),
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => match (first.remove(id), second.remove(id)) {
                (Some(first), Some(second)) => Some(Self::Split {
                    axis,
                    ratio,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
                (Some(node), None) | (None, Some(node)) => Some(node),
                (None, None) => None,
            },
        }
    }

    fn to_recovery(&self, cx: &App) -> RecoveredLayout {
        match self {
            Self::Pane { entity, .. } => {
                let pane = entity.read(cx);
                RecoveredLayout::Pane(RecoveredPane {
                    profile_id: pane.profile().id.clone(),
                    title: Some(pane.title()),
                    cwd: pane
                        .working_directory()
                        .map(|path| path.to_string_lossy().to_string()),
                })
            }
            Self::Split {
                axis,
                ratio,
                first,
                second,
            } => RecoveredLayout::Split {
                axis: match axis {
                    Axis::Horizontal => RecoveredAxis::Horizontal,
                    Axis::Vertical => RecoveredAxis::Vertical,
                },
                ratio: *ratio,
                first: Box::new(first.to_recovery(cx)),
                second: Box::new(second.to_recovery(cx)),
            },
        }
    }
}

struct Tab {
    id: u64,
    layout: SplitNode,
    focused_pane: u64,
    title_override: Option<String>,
}

struct ProfilePickerDialog {
    input: Entity<TextInput>,
    selected: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsSection {
    Terminal,
    Appearance,
    Profiles,
    About,
}

struct SettingsDialog {
    section: SettingsSection,
}

enum Dialog {
    Profiles(ProfilePickerDialog),
    Settings(SettingsDialog),
    SshPassword {
        profile: Profile,
        password: Entity<TextInput>,
        error: Option<String>,
    },
    QuickConnect {
        input: Entity<TextInput>,
        error: Option<String>,
    },
    NewProfile(NewProfileDialog),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NewProfileKind {
    Local,
    Ssh,
}

struct NewProfileDialog {
    kind: NewProfileKind,
    name: Entity<TextInput>,
    command: Entity<TextInput>,
    host: Entity<TextInput>,
    user: Entity<TextInput>,
    port: Entity<TextInput>,
    password: Entity<TextInput>,
    focus: usize,
    error: Option<String>,
}

impl NewProfileDialog {
    fn fields(&self) -> Vec<Entity<TextInput>> {
        match self.kind {
            NewProfileKind::Local => vec![self.name.clone(), self.command.clone()],
            NewProfileKind::Ssh => vec![
                self.name.clone(),
                self.host.clone(),
                self.user.clone(),
                self.port.clone(),
                self.password.clone(),
            ],
        }
    }
}

enum PickerItem {
    QuickSsh(String),
    QuickTelnet(String),
    Profile(Profile),
}

pub struct TabbyApp {
    config: Config,
    tabs: Vec<Tab>,
    active_tab: usize,
    dialog: Option<Dialog>,
    dialog_focus: FocusHandle,
    subscriptions: Vec<Subscription>,
    schemes: Vec<ColorScheme>,
}

impl TabbyApp {
    pub fn empty(config: Config, cx: &mut Context<Self>) -> Self {
        let _ = cx;
        let mut schemes = ColorScheme::builtin_schemes();
        schemes.extend(config.terminal.custom_color_schemes.iter().cloned());
        schemes.push(config.terminal.color_scheme.clone());
        schemes.push(config.terminal.light_color_scheme.clone());
        let mut seen = std::collections::HashSet::new();
        schemes.retain(|scheme| seen.insert(scheme.name.clone()));
        Self {
            config,
            tabs: Vec::new(),
            active_tab: 0,
            dialog: None,
            dialog_focus: cx.focus_handle(),
            subscriptions: Vec::new(),
            schemes,
        }
    }

    pub fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.config.recover_tabs {
            if let Some(state) = RecoveryState::load() {
                self.restore_recovery(state, window, cx);
            }
        }
        if self.tabs.is_empty() {
            let profile = self.default_profile();
            self.open_profile(profile, window, cx);
        }
        self.focus_active(window, cx);
        self.save_recovery(cx);
    }

    fn theme(&self) -> Theme {
        Theme::from_scheme(self.config.active_color_scheme())
    }

    fn active_pane(&self, _cx: &App) -> Option<Entity<TerminalPane>> {
        let tab = self.tabs.get(self.active_tab)?;
        tab.layout.find_pane(tab.focused_pane)
    }

    fn all_panes(&self) -> Vec<Entity<TerminalPane>> {
        let mut entities = Vec::new();
        for tab in &self.tabs {
            tab.layout.entities(&mut entities);
        }
        entities
    }

    fn save_config(&self) {
        if let Err(error) = self.config.save() {
            log::error!("could not save configuration: {error}");
        }
    }

    fn default_profile(&self) -> Profile {
        self.config
            .find_profile("local:default")
            .or_else(|| {
                builtin_profiles()
                    .into_iter()
                    .find(|profile| profile.kind == "local" && !profile.is_template)
            })
            .unwrap_or_else(|| Profile::new_local("Shell", default_shell_command()))
    }

    fn open_default_profile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let profile = self.default_profile();
        self.open_profile(profile, window, cx);
    }

    fn session_for_profile(
        &self,
        profile: &Profile,
        password: Option<String>,
        cwd_override: Option<std::path::PathBuf>,
    ) -> anyhow::Result<NewSession> {
        match profile.kind.as_str() {
            "local" => {
                let mut options = LocalOptions::from_profile(profile);
                if options.cwd.is_none() {
                    if let Some(cwd) = cwd_override {
                        options.cwd = Some(cwd.to_string_lossy().to_string());
                    }
                }
                Ok(NewSession::Local(options))
            }
            "ssh" => {
                let options = SshOptions::from_profile(profile);
                if options.host.trim().is_empty() {
                    anyhow::bail!("SSH profile '{}' has no host", profile.name);
                }
                let password = password.or_else(|| options.password.clone());
                Ok(NewSession::Ssh { options, password })
            }
            "telnet" => Ok(NewSession::Telnet(TelnetOptions::from_profile(profile))),
            other => anyhow::bail!("profile type '{other}' is not supported by tabby-rust"),
        }
    }

    fn spawn_pane(
        &mut self,
        profile: Profile,
        kind: SessionKind,
        session: anyhow::Result<NewSession>,
        cx: &mut Context<Self>,
    ) -> Entity<TerminalPane> {
        let result = match session {
            Ok(session) => {
                tabby_core::session::spawn(session, 80, 24, KnownHostsPolicy::tabby_default())
            }
            Err(error) => Err(error),
        };
        let config = self.config.clone();
        let entity = cx.new(|cx| TerminalPane::new(profile, kind, result, &config, cx));
        let subscription = cx.observe(&entity, |_, _, cx| cx.notify());
        self.subscriptions.push(subscription);
        entity
    }

    fn open_profile(&mut self, profile: Profile, window: &mut Window, cx: &mut Context<Self>) {
        if profile.kind == "ssh" && profile.is_template {
            self.open_quick_connect(window, cx);
            return;
        }
        if profile.kind == "telnet" && profile.is_template {
            self.open_quick_connect(window, cx);
            return;
        }

        let needs_password = profile.kind == "ssh"
            && !profile.has_credentials()
            && std::env::var_os("SSH_AUTH_SOCK").is_none();
        if needs_password {
            self.open_password_dialog(profile, window, cx);
            return;
        }

        let cwd = self
            .active_pane(cx)
            .and_then(|pane| pane.read(cx).working_directory());
        self.spawn_profile(profile, None, cwd, window, cx);
    }

    fn spawn_profile(
        &mut self,
        profile: Profile,
        password: Option<String>,
        cwd: Option<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session = self.session_for_profile(&profile, password, cwd);
        let kind = match &session {
            Ok(NewSession::Local(_)) => SessionKind::Local,
            Ok(NewSession::Ssh { options, .. }) => SessionKind::Ssh {
                host: options.host.clone(),
                user: options.user.clone(),
                port: options.port,
            },
            Ok(NewSession::Telnet(options)) => SessionKind::Telnet {
                host: options.host.clone(),
                port: options.port,
            },
            Err(_) => kind_for_profile(&profile),
        };
        let entity = self.spawn_pane(profile, kind, session, cx);
        let pane_id = next_pane_id();
        self.tabs.push(Tab {
            id: NEXT_TAB_ID.fetch_add(1, Ordering::Relaxed),
            layout: SplitNode::Pane {
                id: pane_id,
                entity: entity.clone(),
            },
            focused_pane: pane_id,
            title_override: None,
        });
        self.active_tab = self.tabs.len() - 1;
        entity.read(cx).focus_handle().focus(window);
        self.save_recovery(cx);
        cx.notify();
    }

    fn open_password_dialog(
        &mut self,
        profile: Profile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let theme = self.theme();
        let password = cx.new(|cx| TextInput::new("Password", true, theme, cx));
        password.read(cx).focus(window);
        self.dialog = Some(Dialog::SshPassword {
            profile,
            password,
            error: None,
        });
        cx.notify();
    }

    fn open_quick_connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let theme = self.theme();
        let input = cx.new(|cx| TextInput::new("user@host:port", false, theme, cx));
        input.read(cx).focus(window);
        self.dialog = Some(Dialog::QuickConnect { input, error: None });
        cx.notify();
    }

    fn open_profile_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let theme = self.theme();
        let input =
            cx.new(|cx| TextInput::new("Search profiles or type user@host", false, theme, cx));
        input.read(cx).focus(window);
        self.dialog = Some(Dialog::Profiles(ProfilePickerDialog { input, selected: 0 }));
        cx.notify();
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = window;
        self.dialog = Some(Dialog::Settings(SettingsDialog {
            section: SettingsSection::Terminal,
        }));
        self.dialog_focus.focus(window);
        cx.notify();
    }

    fn open_new_profile(
        &mut self,
        kind: NewProfileKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let theme = self.theme();
        let dialog = NewProfileDialog {
            kind,
            name: cx.new(|cx| TextInput::new("Name", false, theme, cx)),
            command: cx.new(|cx| TextInput::new("/bin/bash", false, theme, cx)),
            host: cx.new(|cx| TextInput::new("example.com", false, theme, cx)),
            user: cx.new(|cx| TextInput::new("root", false, theme, cx)),
            port: cx.new(|cx| TextInput::new("22", false, theme, cx)),
            password: cx.new(|cx| TextInput::new("Password (optional)", true, theme, cx)),
            focus: 0,
            error: None,
        };
        dialog.name.read(cx).focus(window);
        self.dialog = Some(Dialog::NewProfile(dialog));
        cx.notify();
    }

    fn picker_items(&self, query: &str) -> Vec<PickerItem> {
        let query = query.trim();
        let lower = query.to_lowercase();
        let mut items = Vec::new();
        if !query.is_empty() && !query.contains(' ') {
            items.push(PickerItem::QuickSsh(query.to_string()));
        }
        if !query.is_empty() && query.contains(':') {
            items.push(PickerItem::QuickTelnet(query.to_string()));
        }
        for profile in self.config.selectable_profiles() {
            if lower.is_empty()
                || profile.name.to_lowercase().contains(&lower)
                || profile.id.to_lowercase().contains(&lower)
                || profile
                    .option_str("host")
                    .is_some_and(|host| host.to_lowercase().contains(&lower))
                || profile
                    .option_str("command")
                    .is_some_and(|command| command.to_lowercase().contains(&lower))
            {
                items.push(PickerItem::Profile(profile));
            }
        }
        items
    }

    fn submit_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        match dialog {
            Dialog::Profiles(picker) => {
                let query = picker.input.read(cx).value().to_string();
                let items = self.picker_items(&query);
                if items.is_empty() {
                    self.dialog = Some(Dialog::Profiles(picker));
                    return;
                }
                let index = picker.selected.min(items.len() - 1);
                match items.into_iter().nth(index) {
                    Some(PickerItem::QuickSsh(query)) => {
                        let options = parse_ssh_quick_connect(&query);
                        let mut profile = Profile::new_ssh(
                            query.clone(),
                            options.host.clone(),
                            options.user.clone(),
                            options.port,
                        );
                        profile.options =
                            serde_yaml::to_value(options).unwrap_or(serde_yaml::Value::Null);
                        self.open_profile(profile, window, cx);
                    }
                    Some(PickerItem::QuickTelnet(query)) => {
                        let options = parse_telnet_quick_connect(&query, 23);
                        let mut profile = Profile::new_local(query.clone(), "");
                        profile.kind = "telnet".into();
                        profile.name = query;
                        profile.options =
                            serde_yaml::to_value(options).unwrap_or(serde_yaml::Value::Null);
                        self.open_profile(profile, window, cx);
                    }
                    Some(PickerItem::Profile(profile)) => self.open_profile(profile, window, cx),
                    None => {}
                }
            }
            Dialog::Settings(settings) => {
                self.dialog = Some(Dialog::Settings(settings));
            }
            Dialog::SshPassword {
                profile,
                password,
                error,
            } => {
                let value = password.read(cx).value().to_string();
                if value.is_empty() {
                    self.dialog = Some(Dialog::SshPassword {
                        profile,
                        password,
                        error: Some("Enter a password".into()),
                    });
                    if let Some(handle) = self.dialog_input(cx) {
                        handle.focus(window);
                    }
                    return;
                }
                let _ = error;
                self.spawn_profile(profile, Some(value), None, window, cx);
            }
            Dialog::QuickConnect { input, error } => {
                let query = input.read(cx).value().trim().to_string();
                if query.is_empty() {
                    let handle = input.read(cx).focus_handle();
                    self.dialog = Some(Dialog::QuickConnect {
                        input,
                        error: Some("Enter a host".into()),
                    });
                    handle.focus(window);
                    return;
                }
                let _ = error;
                let options = parse_ssh_quick_connect(&query);
                let mut profile = Profile::new_ssh(
                    query.clone(),
                    options.host.clone(),
                    options.user.clone(),
                    options.port,
                );
                profile.options = serde_yaml::to_value(options).unwrap_or(serde_yaml::Value::Null);
                self.open_profile(profile, window, cx);
            }
            Dialog::NewProfile(new_profile) => {
                self.submit_new_profile(new_profile, window, cx);
            }
        }
        cx.notify();
    }

    fn submit_new_profile(
        &mut self,
        dialog: NewProfileDialog,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = dialog.name.read(cx).value().trim().to_string();
        match dialog.kind {
            NewProfileKind::Local => {
                let command = dialog.command.read(cx).value().trim().to_string();
                if command.is_empty() {
                    self.dialog = Some(Dialog::NewProfile(NewProfileDialog {
                        error: Some("Command is required".into()),
                        ..dialog
                    }));
                    return;
                }
                let name = if name.is_empty() {
                    command.clone()
                } else {
                    name
                };
                let profile = Profile::new_local(name, command);
                self.config.upsert_profile(profile.clone());
                self.save_config();
                self.spawn_profile(profile, None, None, window, cx);
            }
            NewProfileKind::Ssh => {
                let host = dialog.host.read(cx).value().trim().to_string();
                let user = dialog.user.read(cx).value().trim().to_string();
                let port_text = dialog.port.read(cx).value().trim().to_string();
                let password = dialog.password.read(cx).value().to_string();
                if host.is_empty() {
                    self.dialog = Some(Dialog::NewProfile(NewProfileDialog {
                        error: Some("Host is required".into()),
                        ..dialog
                    }));
                    return;
                }
                let port: u16 = port_text.parse().unwrap_or(22);
                let user = if user.is_empty() { "root".into() } else { user };
                let name = if name.is_empty() {
                    format!("{user}@{host}")
                } else {
                    name
                };
                let mut profile = Profile::new_ssh(name, &host, &user, port);
                let mut options = SshOptions {
                    host,
                    port,
                    user,
                    ..Default::default()
                };
                if !password.is_empty() {
                    options.auth = Some("password".into());
                }
                profile.options = serde_yaml::to_value(options).unwrap_or(serde_yaml::Value::Null);
                self.config.upsert_profile(profile.clone());
                self.save_config();
                self.spawn_profile(
                    profile,
                    (!password.is_empty()).then_some(password),
                    None,
                    window,
                    cx,
                );
            }
        }
    }

    fn cancel_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dialog = None;
        if let Some(pane) = self.active_pane(cx) {
            pane.read(cx).focus_handle().focus(window);
        }
        cx.notify();
    }

    fn dialog_input(&self, cx: &App) -> Option<FocusHandle> {
        match self.dialog.as_ref()? {
            Dialog::Profiles(picker) => Some(picker.input.read(cx).focus_handle()),
            Dialog::SshPassword { password, .. } => Some(password.read(cx).focus_handle()),
            Dialog::QuickConnect { input, .. } => Some(input.read(cx).focus_handle()),
            Dialog::NewProfile(dialog) => dialog
                .fields()
                .get(dialog.focus)
                .map(|input| input.read(cx).focus_handle()),
            Dialog::Settings(_) => None,
        }
    }

    fn dialog_move_focus(&mut self, backwards: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(Dialog::NewProfile(dialog)) = self.dialog.as_mut() {
            let fields = dialog.fields();
            if fields.is_empty() {
                return;
            }
            let count = fields.len();
            dialog.focus = if backwards {
                (dialog.focus + count - 1) % count
            } else {
                (dialog.focus + 1) % count
            };
            fields[dialog.focus].read(cx).focus(window);
        }
    }

    fn dialog_move(&mut self, delta: i32, cx: &mut Context<Self>) {
        let query = match self.dialog.as_ref() {
            Some(Dialog::Profiles(picker)) => Some(picker.input.read(cx).value().to_string()),
            _ => None,
        };
        let Some(query) = query else {
            return;
        };
        let count = self.picker_items(&query).len();
        if let Some(Dialog::Profiles(picker)) = self.dialog.as_mut() {
            if count == 0 {
                picker.selected = 0;
            } else {
                let next = picker.selected as i32 + delta;
                picker.selected = next.rem_euclid(count as i32) as usize;
            }
            cx.notify();
        }
    }

    fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let profile = self.default_profile();
        self.open_profile(profile, window, cx);
    }

    fn close_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        let index = self.active_tab.min(self.tabs.len() - 1);
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            if self.config.appearance.last_tab_closes_window {
                window.remove_window();
                return;
            }
            self.open_default_profile(window, cx);
            return;
        }
        self.active_tab = index.min(self.tabs.len() - 1);
        self.focus_active(window, cx);
        self.save_recovery(cx);
        cx.notify();
    }

    fn focus_active(&self, window: &mut Window, cx: &App) {
        if let Some(pane) = self.active_pane(cx) {
            pane.read(cx).focus_handle().focus(window);
        }
    }

    fn set_active_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        self.active_tab = index;
        self.focus_active(window, cx);
        cx.notify();
    }

    fn cycle_tab(&mut self, delta: i32, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        let count = self.tabs.len() as i32;
        let next = (self.active_tab as i32 + delta).rem_euclid(count) as usize;
        self.set_active_tab(next, window, cx);
    }

    fn split(&mut self, axis: Axis, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane) = self.active_pane(cx) else {
            return;
        };
        let profile = pane.read(cx).profile().clone();
        let cwd = pane.read(cx).working_directory();
        if profile.kind == "ssh"
            && !profile.has_credentials()
            && std::env::var_os("SSH_AUTH_SOCK").is_none()
        {
            self.open_password_dialog(profile, window, cx);
            return;
        }
        let session = self.session_for_profile(&profile, None, cwd);
        let kind = pane.read(cx).kind().clone();
        let entity = self.spawn_pane(profile, kind, session, cx);
        let new_id = next_pane_id();
        let old_id = self.tabs[self.active_tab].focused_pane;
        let replacement = SplitNode::Split {
            axis,
            ratio: 0.5,
            first: Box::new(SplitNode::Pane {
                id: new_id,
                entity: entity.clone(),
            }),
            second: Box::new(SplitNode::Pane {
                id: old_id,
                entity: pane.clone(),
            }),
        };
        if self.tabs[self.active_tab]
            .layout
            .replace_pane(old_id, replacement)
        {
            self.tabs[self.active_tab].focused_pane = new_id;
            entity.read(cx).focus_handle().focus(window);
            self.save_recovery(cx);
            cx.notify();
        }
    }

    fn close_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(_) = self.tabs.get(self.active_tab) else {
            return;
        };
        let tab_index = self.active_tab;
        let focused = self.tabs[tab_index].focused_pane;
        let mut ids = Vec::new();
        self.tabs[tab_index].layout.pane_ids(&mut ids);
        if ids.len() <= 1 {
            self.close_tab(window, cx);
            return;
        }
        let layout = self.tabs[tab_index].layout.clone();
        if let Some(layout) = layout.remove(focused) {
            self.tabs[tab_index].layout = layout;
            let mut remaining = Vec::new();
            self.tabs[tab_index].layout.pane_ids(&mut remaining);
            if let Some(next) = remaining.first() {
                self.tabs[tab_index].focused_pane = *next;
            }
            self.focus_active(window, cx);
            self.save_recovery(cx);
            cx.notify();
        }
    }

    fn focus_pane_step(&mut self, delta: i32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return;
        };
        let mut ids = Vec::new();
        tab.layout.pane_ids(&mut ids);
        if ids.is_empty() {
            return;
        }
        let current = ids
            .iter()
            .position(|id| *id == tab.focused_pane)
            .unwrap_or(0);
        let next = (current as i32 + delta).rem_euclid(ids.len() as i32) as usize;
        tab.focused_pane = ids[next];
        self.focus_active(window, cx);
        cx.notify();
    }

    fn with_focused_pane(
        &mut self,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut TerminalPane, &mut Context<TerminalPane>),
    ) {
        if let Some(pane) = self.active_pane(cx) {
            pane.update(cx, |pane, cx| update(pane, cx));
        }
    }

    fn copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = window;
        if let Some(pane) = self.active_pane(cx) {
            if let Some(text) = pane.read(cx).copy_selection() {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
            }
        }
    }

    fn paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = window;
        self.with_focused_pane(cx, |pane, cx| pane.paste(cx));
    }

    fn select_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = window;
        self.with_focused_pane(cx, |pane, cx| pane.select_all(cx));
    }

    fn search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pane) = self.active_pane(cx) {
            pane.update(cx, |pane, cx| pane.begin_search(window, cx));
        }
    }

    fn find(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let _ = window;
        if let Some(pane) = self.active_pane(cx) {
            pane.update(cx, |pane, cx| pane.find(forward, cx));
        }
    }

    fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(pane) = self.active_pane(cx) {
            pane.update(cx, |pane, cx| pane.close_search(window, cx));
        }
    }

    fn scroll(&mut self, kind: ScrollCommand, window: &mut Window, cx: &mut Context<Self>) {
        let _ = window;
        if let Some(pane) = self.active_pane(cx) {
            pane.update(cx, |pane, cx| {
                pane.scroll_command(kind);
                cx.notify();
            });
        }
    }

    fn zoom(&mut self, delta: f32, cx: &mut Context<Self>) {
        self.config.terminal.font_size = (self.config.terminal.font_size + delta).clamp(6.0, 48.0);
        self.apply_config(cx);
    }

    fn reset_zoom(&mut self, cx: &mut Context<Self>) {
        self.config.terminal.font_size = 14.0;
        self.apply_config(cx);
    }

    fn apply_config(&mut self, cx: &mut Context<Self>) {
        let config = self.config.clone();
        for pane in self.all_panes() {
            pane.update(cx, |pane, cx| pane.apply_config(&config, cx));
        }
        self.schemes = ColorScheme::builtin_schemes();
        self.schemes
            .extend(config.terminal.custom_color_schemes.iter().cloned());
        self.schemes.push(config.terminal.color_scheme.clone());
        self.schemes
            .push(config.terminal.light_color_scheme.clone());
        let mut seen = std::collections::HashSet::new();
        self.schemes
            .retain(|scheme| seen.insert(scheme.name.clone()));
        self.save_config();
        cx.notify();
    }

    fn save_recovery(&self, cx: &App) {
        if !self.config.recover_tabs {
            return;
        }
        let tabs = self
            .tabs
            .iter()
            .map(|tab| {
                let mut ids = Vec::new();
                tab.layout.pane_ids(&mut ids);
                tabby_core::recovery::RecoveredTab {
                    title: tab.title_override.clone(),
                    focused_pane: ids
                        .iter()
                        .position(|id| *id == tab.focused_pane)
                        .unwrap_or(0),
                    layout: Some(tab.layout.to_recovery(cx)),
                }
            })
            .collect();
        let state = RecoveryState { version: 1, tabs };
        if let Err(error) = state.save() {
            log::warn!("could not save tab recovery state: {error}");
        }
    }

    fn restore_recovery(
        &mut self,
        state: RecoveryState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = window;
        for recovered in state.tabs {
            let Some(layout) = recovered.layout else {
                continue;
            };
            if let Some((layout, _)) = self.restore_layout(layout, cx) {
                let mut ids = Vec::new();
                layout.pane_ids(&mut ids);
                let focused_pane = ids
                    .get(recovered.focused_pane.min(ids.len().saturating_sub(1)))
                    .copied()
                    .unwrap_or_else(|| layout.first_pane_id());
                self.tabs.push(Tab {
                    id: NEXT_TAB_ID.fetch_add(1, Ordering::Relaxed),
                    layout,
                    focused_pane,
                    title_override: recovered.title,
                });
            }
        }
        if !self.tabs.is_empty() {
            self.active_tab = 0;
        }
    }

    fn restore_layout(
        &mut self,
        layout: RecoveredLayout,
        cx: &mut Context<Self>,
    ) -> Option<(SplitNode, usize)> {
        match layout {
            RecoveredLayout::Pane(pane) => {
                let profile = self.config.find_profile(&pane.profile_id)?;
                if profile.kind == "ssh"
                    && !profile.has_credentials()
                    && std::env::var_os("SSH_AUTH_SOCK").is_none()
                {
                    return None;
                }
                let cwd = pane.cwd.map(std::path::PathBuf::from);
                let session = self.session_for_profile(&profile, None, cwd);
                let kind = match &session {
                    Ok(session) => session.kind(),
                    Err(_) => kind_for_profile(&profile),
                };
                let entity = self.spawn_pane(profile, kind, session, cx);
                let id = next_pane_id();
                Some((SplitNode::Pane { id, entity }, 0))
            }
            RecoveredLayout::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let first = self.restore_layout(*first, cx);
                let second = self.restore_layout(*second, cx);
                match (first, second) {
                    (Some((first, _)), Some((second, _))) => Some((
                        SplitNode::Split {
                            axis: match axis {
                                RecoveredAxis::Horizontal => Axis::Horizontal,
                                RecoveredAxis::Vertical => Axis::Vertical,
                            },
                            ratio: ratio.clamp(0.1, 0.9),
                            first: Box::new(first),
                            second: Box::new(second),
                        },
                        0,
                    )),
                    (Some(node), None) | (None, Some(node)) => Some(node),
                    (None, None) => None,
                }
            }
        }
    }
}

fn kind_for_profile(profile: &Profile) -> SessionKind {
    match profile.kind.as_str() {
        "ssh" => SessionKind::Ssh {
            host: profile.option_str("host").unwrap_or("").to_string(),
            user: profile.option_str("user").unwrap_or("root").to_string(),
            port: profile.option_u64("port").unwrap_or(22) as u16,
        },
        "telnet" => SessionKind::Telnet {
            host: profile.option_str("host").unwrap_or("").to_string(),
            port: profile.option_u64("port").unwrap_or(23) as u16,
        },
        _ => SessionKind::Local,
    }
}

fn field_row(
    label: &'static str,
    input: Entity<TextInput>,
    focused: bool,
    theme: Theme,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(div().text_xs().text_color(theme.muted).child(label))
        .child(input)
        .when(focused, |this| this.border_1().border_color(theme.accent))
}

fn button(
    label: impl Into<SharedString>,
    listener: impl Fn(&gpui::MouseDownEvent, &mut Window, &mut App) + 'static,
    theme: Theme,
) -> impl IntoElement {
    div()
        .px_3()
        .py_1()
        .rounded(px(3.0))
        .bg(theme.surface_active)
        .text_sm()
        .text_color(theme.foreground)
        .hover(|style| style.bg(theme.border))
        .on_mouse_down(gpui::MouseButton::Left, listener)
        .child(label.into())
}

impl TabbyApp {
    fn render_tab_bar(&self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let mut tabs = div().flex().items_center().overflow_hidden();
        for (index, tab) in self.tabs.iter().enumerate() {
            let title = tab
                .title_override
                .clone()
                .or_else(|| {
                    tab.layout
                        .find_pane(tab.focused_pane)
                        .map(|pane| pane.read(cx).title())
                })
                .unwrap_or_else(|| "Terminal".into());
            let active = index == self.active_tab;
            let select_this = this.clone();
            let close_this = this.clone();
            tabs = tabs.child(
                div()
                    .id(gpui::ElementId::Name(SharedString::from(format!(
                        "tab-{}",
                        tab.id
                    ))))
                    .h_full()
                    .px_3()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_r_1()
                    .border_color(theme.border)
                    .when(active, |this| {
                        this.bg(theme.background)
                            .border_b_2()
                            .border_color(theme.accent)
                    })
                    .when(!active, |this| {
                        this.hover(|style| style.bg(theme.surface_active))
                    })
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        let _ =
                            select_this.update(cx, |app, cx| app.set_active_tab(index, window, cx));
                    })
                    .child(
                        div()
                            .text_sm()
                            .text_color(if active {
                                theme.foreground
                            } else {
                                theme.muted
                            })
                            .child(title),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted)
                            .hover(|style| style.text_color(theme.error))
                            .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                                let _ = close_this.update(cx, |app, cx| {
                                    app.active_tab = index;
                                    app.close_tab(window, cx);
                                });
                            })
                            .child("×"),
                    ),
            );
        }

        let add_this = this.clone();
        let settings_this = this.clone();
        div()
            .h(px(36.0))
            .flex_shrink_0()
            .flex()
            .items_center()
            .bg(theme.surface)
            .border_b_1()
            .border_color(theme.border)
            .child(tabs)
            .child(
                div()
                    .w(px(32.0))
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.muted)
                    .hover(|style| style.bg(theme.surface_active).text_color(theme.foreground))
                    .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                        let _ = add_this.update(cx, |app, cx| app.open_profile_picker(window, cx));
                    })
                    .child("+"),
            )
            .child(div().flex_1())
            .child(
                div()
                    .w(px(32.0))
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.muted)
                    .hover(|style| style.bg(theme.surface_active).text_color(theme.foreground))
                    .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                        let _ = settings_this.update(cx, |app, cx| app.open_settings(window, cx));
                    })
                    .child("⚙"),
            )
    }

    fn render_node(&self, node: &SplitNode, theme: Theme) -> AnyElement {
        match node {
            SplitNode::Pane { entity, .. } => entity.clone().into_any_element(),
            SplitNode::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let first = self.render_node(first, theme);
                let second = self.render_node(second, theme);
                let divider = match axis {
                    Axis::Horizontal => div().w(px(1.0)).h_full().bg(theme.border),
                    Axis::Vertical => div().w_full().h(px(1.0)).bg(theme.border),
                };
                div()
                    .flex()
                    .when(*axis == Axis::Horizontal, |this| this.flex_row())
                    .when(*axis == Axis::Vertical, |this| this.flex_col())
                    .size_full()
                    .child(
                        div()
                            .flex_basis(relative(*ratio))
                            .min_w(px(0.0))
                            .min_h(px(0.0))
                            .overflow_hidden()
                            .size_full()
                            .child(first),
                    )
                    .child(divider)
                    .child(
                        div()
                            .flex_basis(relative(1.0 - *ratio))
                            .min_w(px(0.0))
                            .min_h(px(0.0))
                            .overflow_hidden()
                            .size_full()
                            .child(second),
                    )
                    .into_any_element()
            }
        }
    }

    fn render_dialog(&mut self, theme: Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let mut dialog = self.dialog.take()?;
        let result = self.render_dialog_inner(&mut dialog, theme, cx);
        self.dialog = Some(dialog);
        result
    }

    fn render_dialog_inner(
        &self,
        dialog: &mut Dialog,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let this = cx.entity().downgrade();
        let panel = match dialog {
            Dialog::Profiles(picker) => {
                let query = picker.input.read(cx).value().to_string();
                let items = self.picker_items(&query);
                let mut list = div()
                    .id(gpui::ElementId::Name("profile-picker-list".into()))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .max_h(px(320.0))
                    .overflow_y_scroll();
                for (index, item) in items.iter().enumerate() {
                    let (title, subtitle) = match item {
                        PickerItem::QuickSsh(query) => (
                            format!("Connect via SSH: {query}"),
                            "Type a host and press Enter".to_string(),
                        ),
                        PickerItem::QuickTelnet(query) => (
                            format!("Connect via Telnet: {query}"),
                            "Raw TCP with telnet negotiation".to_string(),
                        ),
                        PickerItem::Profile(profile) => (
                            profile.name.clone(),
                            match profile.kind.as_str() {
                                "local" => {
                                    profile.option_str("command").unwrap_or("Local").to_string()
                                }
                                "ssh" => format!(
                                    "{}@{}:{}",
                                    profile.option_str("user").unwrap_or("root"),
                                    profile.option_str("host").unwrap_or(""),
                                    profile.option_u64("port").unwrap_or(22)
                                ),
                                other => other.to_string(),
                            },
                        ),
                    };
                    let selected = index == picker.selected.min(items.len().saturating_sub(1));
                    let select_this = this.clone();
                    list = list.child(
                        div()
                            .flex()
                            .flex_col()
                            .px_2()
                            .py_1()
                            .rounded(px(3.0))
                            .when(selected, |this| this.bg(theme.surface_active))
                            .hover(|style| style.bg(theme.surface_active))
                            .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                                let _ = select_this.update(cx, |app: &mut TabbyApp, cx| {
                                    if let Some(Dialog::Profiles(picker)) = app.dialog.as_mut() {
                                        picker.selected = index;
                                    }
                                    app.submit_dialog(window, cx);
                                });
                            })
                            .child(div().text_sm().text_color(theme.foreground).child(title))
                            .child(div().text_xs().text_color(theme.muted).child(subtitle)),
                    );
                }
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .text_lg()
                            .text_color(theme.foreground)
                            .child("New tab"),
                    )
                    .child(picker.input.clone())
                    .child(list)
            }
            Dialog::Settings(settings) => {
                let mut sections = div().flex().gap_1();
                for (section, label) in [
                    (SettingsSection::Terminal, "Terminal"),
                    (SettingsSection::Appearance, "Appearance"),
                    (SettingsSection::Profiles, "Profiles"),
                    (SettingsSection::About, "About"),
                ] {
                    let select_this = this.clone();
                    sections = sections.child(
                        div()
                            .px_3()
                            .py_1()
                            .rounded(px(3.0))
                            .text_sm()
                            .when(settings.section == section, |this| {
                                this.bg(theme.surface_active).text_color(theme.foreground)
                            })
                            .when(settings.section != section, |this| {
                                this.text_color(theme.muted)
                            })
                            .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                                let _ = select_this.update(cx, |app, cx| {
                                    if let Some(Dialog::Settings(settings)) = app.dialog.as_mut() {
                                        settings.section = section;
                                    }
                                    cx.notify();
                                });
                            })
                            .child(label),
                    );
                }
                let content: AnyElement = match settings.section {
                    SettingsSection::Terminal => self.settings_terminal(theme, cx),
                    SettingsSection::Appearance => self.settings_appearance(theme, cx),
                    SettingsSection::Profiles => self.settings_profiles(theme, cx),
                    SettingsSection::About => self.settings_about(theme),
                };
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .text_lg()
                            .text_color(theme.foreground)
                            .child("Settings"),
                    )
                    .child(sections)
                    .child(content)
            }
            Dialog::SshPassword {
                profile,
                password,
                error,
            } => {
                let title = format!(
                    "SSH password for {}@{}",
                    profile.option_str("user").unwrap_or("root"),
                    profile.option_str("host").unwrap_or("")
                );
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_lg().text_color(theme.foreground).child(title))
                    .child(password.clone())
                    .when_some(error.clone(), |this, error| {
                        this.child(div().text_xs().text_color(theme.error).child(error))
                    })
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted)
                            .child("Enter to connect · Esc to cancel"),
                    )
            }
            Dialog::QuickConnect { input, error } => div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_lg()
                        .text_color(theme.foreground)
                        .child("Quick connect"),
                )
                .child(input.clone())
                .when_some(error.clone(), |this, error| {
                    this.child(div().text_xs().text_color(theme.error).child(error))
                })
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted)
                        .child("user@host:port · Enter to connect"),
                ),
            Dialog::NewProfile(new_profile) => {
                let mut fields = div().flex().flex_col().gap_2();
                match new_profile.kind {
                    NewProfileKind::Local => {
                        fields = fields
                            .child(field_row(
                                "Name",
                                new_profile.name.clone(),
                                new_profile.focus == 0,
                                theme,
                            ))
                            .child(field_row(
                                "Command",
                                new_profile.command.clone(),
                                new_profile.focus == 1,
                                theme,
                            ));
                    }
                    NewProfileKind::Ssh => {
                        fields = fields
                            .child(field_row(
                                "Name",
                                new_profile.name.clone(),
                                new_profile.focus == 0,
                                theme,
                            ))
                            .child(field_row(
                                "Host",
                                new_profile.host.clone(),
                                new_profile.focus == 1,
                                theme,
                            ))
                            .child(field_row(
                                "User",
                                new_profile.user.clone(),
                                new_profile.focus == 2,
                                theme,
                            ))
                            .child(field_row(
                                "Port",
                                new_profile.port.clone(),
                                new_profile.focus == 3,
                                theme,
                            ))
                            .child(field_row(
                                "Password (session only)",
                                new_profile.password.clone(),
                                new_profile.focus == 4,
                                theme,
                            ));
                    }
                }
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_lg().text_color(theme.foreground).child(
                        match new_profile.kind {
                            NewProfileKind::Local => "New local profile",
                            NewProfileKind::Ssh => "New SSH profile",
                        },
                    ))
                    .child(fields)
                    .when_some(new_profile.error.clone(), |this, error| {
                        this.child(div().text_xs().text_color(theme.error).child(error))
                    })
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted)
                            .child("Tab moves between fields · Enter saves · Esc cancels"),
                    )
            }
        };

        Some(
            div()
                .key_context("dialog")
                .track_focus(&self.dialog_focus)
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgba(0x000000aa))
                .child(
                    div()
                        .w(px(560.0))
                        .id(gpui::ElementId::Name("dialog-panel".into()))
                        .max_h(px(560.0))
                        .overflow_y_scroll()
                        .p_4()
                        .rounded(px(6.0))
                        .bg(theme.surface)
                        .border_1()
                        .border_color(theme.border)
                        .child(panel),
                )
                .into_any_element(),
        )
    }

    fn settings_terminal(&self, theme: Theme, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.entity().downgrade();
        let size = self.config.terminal.font_size;
        let zoom_out = {
            let this = this.clone();
            move |_: &gpui::MouseDownEvent, _: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |app, cx| app.zoom(-1.0, cx));
            }
        };
        let zoom_in = {
            let this = this.clone();
            move |_: &gpui::MouseDownEvent, _: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |app, cx| app.zoom(1.0, cx));
            }
        };
        let cursor = self.config.terminal.cursor.clone();
        let mut cursor_buttons = div().flex().gap_1();
        for style in ["block", "beam", "underline"] {
            let this = this.clone();
            let style_owned = style.to_string();
            cursor_buttons = cursor_buttons.child(
                div()
                    .px_2()
                    .py_1()
                    .rounded(px(3.0))
                    .text_sm()
                    .when(cursor == style, |this| {
                        this.bg(theme.accent).text_color(theme.background)
                    })
                    .when(cursor != style, |this| {
                        this.bg(theme.surface_active).text_color(theme.foreground)
                    })
                    .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                        let _ = this.update(cx, |app, cx| {
                            app.config.terminal.cursor = style_owned.clone();
                            app.apply_config(cx);
                        });
                    })
                    .child(style),
            );
        }

        let schemes = self.schemes.clone();
        let current_scheme = self.config.terminal.color_scheme.name.clone();
        let mut scheme_list = div()
            .id(gpui::ElementId::Name("scheme-list".into()))
            .flex()
            .flex_col()
            .gap_1()
            .max_h(px(180.0))
            .overflow_y_scroll();
        for scheme in schemes {
            let this = this.clone();
            let scheme_clone = scheme.clone();
            let selected = scheme.name == current_scheme;
            scheme_list = scheme_list.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(px(3.0))
                    .when(selected, |this| this.bg(theme.surface_active))
                    .hover(|style| style.bg(theme.surface_active))
                    .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                        let _ = this.update(cx, |app, cx| {
                            app.config.terminal.color_scheme = scheme_clone.clone();
                            app.apply_config(cx);
                        });
                    })
                    .child(
                        div()
                            .w(px(28.0))
                            .h(px(16.0))
                            .rounded(px(2.0))
                            .bg(crate::theme::hsla_from_rgb8(scheme.background_rgb()))
                            .border_1()
                            .border_color(theme.border)
                            .child(
                                div()
                                    .w(px(8.0))
                                    .h(px(8.0))
                                    .m(px(3.0))
                                    .rounded(px(1.0))
                                    .bg(crate::theme::hsla_from_rgb8(scheme.ansi(2))),
                            ),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.foreground)
                            .child(scheme.name.clone()),
                    ),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.foreground)
                            .child("Font size"),
                    )
                    .child(button("-", zoom_out, theme))
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.foreground)
                            .child(format!("{size:.0} px")),
                    )
                    .child(button("+", zoom_in, theme)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_sm().text_color(theme.foreground).child("Cursor"))
                    .child(cursor_buttons),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.foreground)
                            .child("Color scheme"),
                    )
                    .child(scheme_list),
            )
            .into_any_element()
    }

    fn settings_appearance(&self, theme: Theme, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.entity().downgrade();
        let mode = self.config.appearance.color_scheme_mode.clone();
        let mut buttons = div().flex().gap_1();
        for value in ["dark", "light"] {
            let this = this.clone();
            let value_owned = value.to_string();
            buttons = buttons.child(
                div()
                    .px_3()
                    .py_1()
                    .rounded(px(3.0))
                    .text_sm()
                    .when(mode == value, |this| {
                        this.bg(theme.accent).text_color(theme.background)
                    })
                    .when(mode != value, |this| {
                        this.bg(theme.surface_active).text_color(theme.foreground)
                    })
                    .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                        let _ = this.update(cx, |app, cx| {
                            app.config.appearance.color_scheme_mode = value_owned.clone();
                            app.apply_config(cx);
                        });
                    })
                    .child(value),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.foreground)
                            .child("Color scheme mode"),
                    )
                    .child(buttons),
            )
            .child(div().text_xs().text_color(theme.muted).child(format!(
                "Active scheme: {}",
                self.config.active_color_scheme().name
            )))
            .into_any_element()
    }

    fn settings_profiles(&self, theme: Theme, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.entity().downgrade();
        let mut list = div()
            .id(gpui::ElementId::Name("settings-profile-list".into()))
            .flex()
            .flex_col()
            .gap_1()
            .max_h(px(240.0))
            .overflow_y_scroll();
        let profiles: Vec<Profile> = self
            .config
            .profiles
            .iter()
            .cloned()
            .chain(builtin_profiles())
            .collect();
        for profile in profiles {
            let custom = !profile.is_builtin;
            let id = profile.id.clone();
            let delete_this = this.clone();
            let subtitle = match profile.kind.as_str() {
                "local" => profile.option_str("command").unwrap_or("local").to_string(),
                "ssh" => format!(
                    "{}@{}:{}",
                    profile.option_str("user").unwrap_or("root"),
                    profile.option_str("host").unwrap_or(""),
                    profile.option_u64("port").unwrap_or(22)
                ),
                other => other.to_string(),
            };
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(theme.foreground)
                                    .child(profile.name.clone()),
                            )
                            .child(div().text_xs().text_color(theme.muted).child(subtitle)),
                    )
                    .when(custom, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(theme.error)
                                .on_mouse_down(gpui::MouseButton::Left, move |_, _, cx| {
                                    let _ = delete_this.update(cx, |app, cx| {
                                        app.config.remove_profile(&id);
                                        app.apply_config(cx);
                                    });
                                })
                                .child("Delete"),
                        )
                    }),
            );
        }
        let new_ssh = {
            let this = this.clone();
            move |_: &gpui::MouseDownEvent, window: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |app, cx| {
                    app.open_new_profile(NewProfileKind::Ssh, window, cx)
                });
            }
        };
        let new_local = {
            let this = this.clone();
            move |_: &gpui::MouseDownEvent, window: &mut Window, cx: &mut App| {
                let _ = this.update(cx, |app, cx| {
                    app.open_new_profile(NewProfileKind::Local, window, cx)
                });
            }
        };
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(list)
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(button("New SSH profile", new_ssh, theme))
                    .child(button("New local profile", new_local, theme)),
            )
            .into_any_element()
    }

    fn settings_about(&self, theme: Theme) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .text_color(theme.foreground)
                    .child("Tabby Rust — a Rust/GPUI reimplementation of Tabby"),
            )
            .child(div().text_xs().text_color(theme.muted).child(format!(
                "Core: local PTY, SSH (russh), telnet, VT (alacritty_terminal). Profile config: {}",
                Config::default_path().display()
            )))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted)
                    .child("Gaps: SFTP, port forwarding, serial, plugin manager, X11 forwarding."),
            )
            .into_any_element()
    }

    fn quit_app(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.save_recovery(cx);
        cx.quit();
    }
}

impl Render for TabbyApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme();
        let this = cx.entity().downgrade();
        let mut root = div()
            .key_context("app")
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground);

        macro_rules! bind_window_action {
            ($action:ty, $method:ident) => {{
                let weak = this.clone();
                root = root.on_action(move |_: &$action, window, cx| {
                    let _ = weak.update(cx, |app, cx| app.$method(window, cx));
                });
            }};
        }
        bind_window_action!(NewTab, new_tab);
        bind_window_action!(CloseTab, close_tab);
        bind_window_action!(ClosePane, close_pane);
        bind_window_action!(OpenProfilePicker, open_profile_picker);
        bind_window_action!(ToggleSettings, open_settings);
        bind_window_action!(SubmitDialog, submit_dialog);
        bind_window_action!(CancelDialog, cancel_dialog);
        bind_window_action!(QuitApp, quit_app);

        macro_rules! bind_simple_action {
            ($action:ty, $body:expr) => {{
                let weak = this.clone();
                root = root.on_action(move |_: &$action, window, cx| {
                    let _ = weak.update(cx, |app, cx| {
                        ($body)(app, window, cx);
                    });
                });
            }};
        }
        bind_simple_action!(
            NextTab,
            |app: &mut TabbyApp, window: &mut Window, cx: &mut Context<TabbyApp>| app
                .cycle_tab(1, window, cx)
        );
        bind_simple_action!(
            PrevTab,
            |app: &mut TabbyApp, window: &mut Window, cx: &mut Context<TabbyApp>| app
                .cycle_tab(-1, window, cx)
        );
        bind_simple_action!(
            SplitRight,
            |app: &mut TabbyApp, window: &mut Window, cx: &mut Context<TabbyApp>| app.split(
                Axis::Horizontal,
                window,
                cx
            )
        );
        bind_simple_action!(
            SplitDown,
            |app: &mut TabbyApp, window: &mut Window, cx: &mut Context<TabbyApp>| app.split(
                Axis::Vertical,
                window,
                cx
            )
        );
        bind_simple_action!(
            FocusPaneLeft,
            |app: &mut TabbyApp, window: &mut Window, cx: &mut Context<TabbyApp>| app
                .focus_pane_step(-1, window, cx)
        );
        bind_simple_action!(
            FocusPaneRight,
            |app: &mut TabbyApp, window: &mut Window, cx: &mut Context<TabbyApp>| app
                .focus_pane_step(1, window, cx)
        );
        bind_simple_action!(
            FocusPaneUp,
            |app: &mut TabbyApp, window: &mut Window, cx: &mut Context<TabbyApp>| app
                .focus_pane_step(-1, window, cx)
        );
        bind_simple_action!(
            FocusPaneDown,
            |app: &mut TabbyApp, window: &mut Window, cx: &mut Context<TabbyApp>| app
                .focus_pane_step(1, window, cx)
        );
        bind_simple_action!(
            Search,
            |app: &mut TabbyApp, window: &mut Window, cx: &mut Context<TabbyApp>| app
                .search(window, cx)
        );
        bind_simple_action!(
            FindNext,
            |app: &mut TabbyApp, window: &mut Window, cx: &mut Context<TabbyApp>| app
                .find(true, window, cx)
        );
        bind_simple_action!(
            FindPrev,
            |app: &mut TabbyApp, window: &mut Window, cx: &mut Context<TabbyApp>| app
                .find(false, window, cx)
        );
        bind_simple_action!(
            CloseSearch,
            |app: &mut TabbyApp, window: &mut Window, cx: &mut Context<TabbyApp>| app
                .close_search(window, cx)
        );
        bind_simple_action!(
            Copy,
            |app: &mut TabbyApp, window: &mut Window, cx: &mut Context<TabbyApp>| app
                .copy(window, cx)
        );
        bind_simple_action!(
            Paste,
            |app: &mut TabbyApp, window: &mut Window, cx: &mut Context<TabbyApp>| app
                .paste(window, cx)
        );
        bind_simple_action!(
            SelectAll,
            |app: &mut TabbyApp, window: &mut Window, cx: &mut Context<TabbyApp>| app
                .select_all(window, cx)
        );
        bind_simple_action!(
            ZoomIn,
            |app: &mut TabbyApp, _window: &mut Window, cx: &mut Context<TabbyApp>| app
                .zoom(1.0, cx)
        );
        bind_simple_action!(
            ZoomOut,
            |app: &mut TabbyApp, _window: &mut Window, cx: &mut Context<TabbyApp>| app
                .zoom(-1.0, cx)
        );
        bind_simple_action!(
            ResetZoom,
            |app: &mut TabbyApp, _window: &mut Window, cx: &mut Context<TabbyApp>| app
                .reset_zoom(cx)
        );
        bind_simple_action!(
            ScrollPageUp,
            |app: &mut TabbyApp, window: &mut Window, cx: &mut Context<TabbyApp>| app.scroll(
                ScrollCommand::PageUp,
                window,
                cx
            )
        );
        bind_simple_action!(
            ScrollPageDown,
            |app: &mut TabbyApp, window: &mut Window, cx: &mut Context<TabbyApp>| app.scroll(
                ScrollCommand::PageDown,
                window,
                cx
            )
        );
        bind_simple_action!(
            ScrollLineUp,
            |app: &mut TabbyApp, window: &mut Window, cx: &mut Context<TabbyApp>| app.scroll(
                ScrollCommand::LineUp,
                window,
                cx
            )
        );
        bind_simple_action!(
            ScrollLineDown,
            |app: &mut TabbyApp, window: &mut Window, cx: &mut Context<TabbyApp>| app.scroll(
                ScrollCommand::LineDown,
                window,
                cx
            )
        );
        bind_simple_action!(
            ScrollTop,
            |app: &mut TabbyApp, window: &mut Window, cx: &mut Context<TabbyApp>| app.scroll(
                ScrollCommand::Top,
                window,
                cx
            )
        );
        bind_simple_action!(
            ScrollBottom,
            |app: &mut TabbyApp, window: &mut Window, cx: &mut Context<TabbyApp>| app.scroll(
                ScrollCommand::Bottom,
                window,
                cx
            )
        );
        bind_simple_action!(
            DialogUp,
            |app: &mut TabbyApp, _window: &mut Window, cx: &mut Context<TabbyApp>| {
                app.dialog_move(-1, cx);
            }
        );
        bind_simple_action!(
            DialogDown,
            |app: &mut TabbyApp, _window: &mut Window, cx: &mut Context<TabbyApp>| {
                app.dialog_move(1, cx);
            }
        );
        bind_simple_action!(
            DialogTab,
            |app: &mut TabbyApp, window: &mut Window, cx: &mut Context<TabbyApp>| app
                .dialog_move_focus(false, window, cx)
        );
        bind_simple_action!(
            DialogBackTab,
            |app: &mut TabbyApp, window: &mut Window, cx: &mut Context<TabbyApp>| app
                .dialog_move_focus(true, window, cx)
        );

        root = root.child(self.render_tab_bar(theme, cx));
        let content = if let Some(tab) = self.tabs.get(self.active_tab) {
            self.render_node(&tab.layout, theme)
        } else {
            div()
                .flex()
                .items_center()
                .justify_center()
                .child("No tabs")
                .into_any_element()
        };
        root = root.child(div().flex_1().min_h(px(0.0)).child(content));

        if let Some(dialog) = self.render_dialog(theme, cx) {
            root = root.child(dialog);
        }

        if let Some(handle) = self.dialog_input(cx) {
            if !handle.is_focused(window) {
                handle.focus(window);
            }
        }

        root
    }
}

pub fn register_key_bindings(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-shift-t", NewTab, None),
        KeyBinding::new("ctrl-shift-w", CloseTab, None),
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PrevTab, None),
        KeyBinding::new("ctrl-shift-right", NextTab, None),
        KeyBinding::new("ctrl-shift-left", PrevTab, None),
        KeyBinding::new("ctrl-shift-s", SplitRight, None),
        KeyBinding::new("ctrl-shift-d", SplitDown, None),
        KeyBinding::new("ctrl-alt-t", OpenProfilePicker, None),
        KeyBinding::new("ctrl-shift-e", OpenProfilePicker, None),
        KeyBinding::new("ctrl-,", ToggleSettings, None),
        KeyBinding::new("ctrl-shift-f", Search, None),
        KeyBinding::new("ctrl-shift-c", Copy, None),
        KeyBinding::new("ctrl-shift-v", Paste, None),
        KeyBinding::new("ctrl-shift-a", SelectAll, None),
        KeyBinding::new("ctrl-=", ZoomIn, None),
        KeyBinding::new("ctrl-shift-=", ZoomIn, None),
        KeyBinding::new("ctrl--", ZoomOut, None),
        KeyBinding::new("ctrl-shift--", ZoomOut, None),
        KeyBinding::new("ctrl-0", ResetZoom, None),
        KeyBinding::new("alt-pageup", ScrollPageUp, None),
        KeyBinding::new("alt-pagedown", ScrollPageDown, None),
        KeyBinding::new("ctrl-shift-up", ScrollLineUp, None),
        KeyBinding::new("ctrl-shift-down", ScrollLineDown, None),
        KeyBinding::new("ctrl-pageup", ScrollTop, None),
        KeyBinding::new("ctrl-pagedown", ScrollBottom, None),
        KeyBinding::new("ctrl-alt-left", FocusPaneLeft, None),
        KeyBinding::new("ctrl-alt-right", FocusPaneRight, None),
        KeyBinding::new("ctrl-alt-up", FocusPaneUp, None),
        KeyBinding::new("ctrl-alt-down", FocusPaneDown, None),
        KeyBinding::new("enter", SubmitDialog, Some("dialog")),
        KeyBinding::new("escape", CancelDialog, Some("dialog")),
        KeyBinding::new("up", DialogUp, Some("dialog")),
        KeyBinding::new("down", DialogDown, Some("dialog")),
        KeyBinding::new("tab", DialogTab, Some("dialog")),
        KeyBinding::new("shift-tab", DialogBackTab, Some("dialog")),
        KeyBinding::new("enter", FindNext, Some("search")),
        KeyBinding::new("shift-enter", FindPrev, Some("search")),
        KeyBinding::new("escape", CloseSearch, Some("search")),
    ]);
}
