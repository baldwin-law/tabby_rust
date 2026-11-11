# Tabby Rust

A Rust reimplementation of [Tabby](https://github.com/Eugeny/tabby) — the terminal
emulator, profile manager and SSH client — built on **Tabby's configuration and
profile model** with a Rust core and a **GPUI** frontend.

It is not an Electron wrapper: the terminal engine, PTY/SSH transports and the
UI all run natively in one Rust process.

```
tabby_rust/
├── crates/tabby-core     # VT engine, sessions, config, colour schemes, recovery
└── crates/tabby-gpui     # GPUI application (tabs, splits, dialogs, renderer)
```

## Features

| Area | Reimplemented |
|---|---|
| Terminal | VT parsing and screen state via `alacritty_terminal`; 256-colour palette and Tabby colour schemes; bold/italic/underline/strikethrough/dim/inverse; wide chars and combining marks; alternate screen; bracketed paste; mouse wheel scrollback |
| Sessions | Local PTY (`portable-pty`) with environment/cwd/args, SSH (`russh`) with password, private-key and agent auth, telnet/raw socket with IAC negotiation stripped |
| UI | Tab bar, recursive horizontal/vertical splits, per-pane focus, terminal grid renderer with cursor and selection, search bar, profile picker, quick connect, settings panel |
| Config | Tabby-compatible `~/.config/tabby-rust/config.yaml` (same keys; Windows/macOS paths via `dirs`), built-in shell discovery from `/etc/shells` and the login shell, unknown keys preserved on round-trip |
| Profiles | Local and SSH profiles, template + quick-connect items, shell/command/host/user/port editing, deletion |
| Recovery | Open tabs, split layout, focused pane and profile ids persisted to `recovery.yaml` and reopened on launch |
| Terminal I/O | OSC title, OSC 8 hyperlinks, OSC 52 clipboard copy, OSC 4/10/11 colour queries, device-attribute replies, bell events, selection copy/paste |

Colour schemes: Tabby Default, Tabby Default Light, Solarized Dark, Gruvbox Dark,
Dracula, plus any scheme in `terminal.customColorSchemes`.

### Not implemented (Tabby plugins)

SFTP panel, port forwarding configuration, serial ports, plugin manager, X11
forwarding, zmodem, Docker/WSL pickers, macOS/Windows shell providers, and
tab drag-reordering. The SSH core supports the transport and shell channel; the
extra channels are intentionally out of scope for this reimplementation.

## Build

System packages (Ubuntu/Debian):

```bash
sudo apt install build-essential pkg-config libfontconfig1-dev libfreetype6-dev \
  libxkbcommon-dev libxcb1-dev libwayland-dev libx11-dev libvulkan1 mesa-vulkan-drivers
```

Then:

```bash
cargo build -p tabby-gpui
cargo run -p tabby-gpui
```

Run the core tests:

```bash
cargo test -p tabby-core
```

The suite covers VT emulation, colour resolution, Tabby config/profile
round-trips, recovery layout, local PTY output/exit, and an in-process SSH server
that exercises password authentication, PTY/shell requests and byte flow.

GPUI needs Vulkan 1.1 with dynamic rendering and inline uniform blocks. In a
headless environment run under Xvfb. Mesa's lavapipe works when it is new enough
(Mesa 22+); on older distributions a Chromium-bundled SwiftShader ICD also works:

```bash
xvfb-run -a -s "-screen 0 1280x800x24" \
  env VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json \
  cargo run -p tabby-gpui
```

Note that on some Xvfb/SwiftShader setups the first frame is presented only after
an input event; moving the pointer into the window is enough.

## Packaging

`.github/workflows/release.yml` builds and publishes:

| Platform | Artifacts |
|---|---|
| Linux x86_64 | portable `.tar.gz` (binary, desktop entry, SVG icon, docs), `.deb` |
| macOS arm64 / x86_64 | `.app` inside `.zip`, `.dmg` |
| Windows x86_64 | portable `.zip` with `tabby-rust.exe`, README, LICENSE |

Push a tag to publish:

```bash
git tag v0.1.0
git push origin v0.1.0
```

Manual `workflow_dispatch` runs build the artifacts without creating a release.

Build locally on the target platform (the packaging scripts assume native
artifacts and macOS/Windows tooling):

```bash
cargo build --release -p tabby-gpui

# Linux
bash packaging/linux/package.sh target/release/tabby-rust 0.1.0 dist x86_64

# macOS (run on macOS; arch = arm64 or x86_64)
bash packaging/macos/package.sh target/release/tabby-rust 0.1.0 arm64 dist

# Windows (PowerShell)
pwsh -File packaging/windows/package.ps1 `
  -Binary target/release/tabby-rust.exe -Version 0.1.0 -OutDir dist
```

macOS signing: without secrets the bundle is ad-hoc signed. A GitHub release
with a proper signature needs repository secrets `APPLE_SIGNING_IDENTITY`,
`APPLE_ID`, `APPLE_TEAM_ID` and `APPLE_APP_PASSWORD`; the script signs with the
Developer ID, submits to `notarytool`, staples the ticket, then writes the
zip/dmg. Local downloads from an unsigned/ad-hoc build may require the standard
macOS quarantine override (`xattr -dr com.apple.quarantine "Tabby Rust.app"`).

Runtime requirements: Linux needs `libfontconfig1`, `libfreetype6`,
`libxkbcommon0`, `libxkbcommon-x11-0`, `libxcb1`, `libwayland-client0`, and a
Vulkan 1.1+ loader/driver (`libvulkan1`); macOS 11+ has Metal; Windows needs a
graphics driver exposing Vulkan 1.1+.


## Configuration

`Config::default_path()` resolves to `$TABBY_RUST_CONFIG` when set, otherwise
`~/.config/tabby-rust/config.yaml`. The schema mirrors Tabby:

```yaml
terminal:
  font: monospace
  fontSize: 14
  cursor: block
  cursorBlink: true
  scrollbackLines: 25000
  colorScheme:
    name: Tabby Default
    foreground: "#cacaca"
    background: "#171717"
    cursor: "#bbbbbb"
    colors: ["#000000", "... sixteen entries ..."]
profiles:
  - id: local:default
    type: local
    name: User default
    options:
      command: /bin/bash
      args: ["--login"]
  - id: ssh:prod
    type: ssh
    name: prod
    options:
      host: example.com
      port: 22
      user: deploy
      auth: publicKey
      privateKeys: ["~/.ssh/id_ed25519"]
recoverTabs: true
```

A `config.yaml` copied from Tabby keeps working for local/SSH profile fields
that this implementation understands. Unknown plugin fields and top-level keys
are kept verbatim when the app saves the file. Key spelling matches Tabby's
camelCase (`recoverTabs`, `enableWelcomeTab`, `behaviorOnSessionEnd`, ...).

### Credentials

Passwords typed into the UI are held only for that session and are **not**
written to `config.yaml`. A password can be supplied permanently through
`options.password`, but that stores it in plain text — prefer keys or an SSH
agent. Host keys are checked against `~/.config/tabby-rust/known_hosts`; an
unseen server is trusted once (TOFU) and a changed key is rejected.

## Key bindings

| Binding | Action |
|---|---|
| `Ctrl-Shift-T` | New tab (default profile) |
| `Ctrl-Shift-W` | Close tab |
| `Ctrl-Tab` / `Ctrl-Shift-Tab` | Next / previous tab |
| `Ctrl-Shift-Right` / `Left` | Next / previous tab (Tabby default) |
| `Ctrl-Shift-S` / `Ctrl-Shift-D` | Split right / down |
| `Ctrl-Shift-E` / `Ctrl-Alt-T` | Profile picker |
| `Ctrl-,` | Settings |
| `Ctrl-Alt-Arrows` | Pane focus |
| `Ctrl-Shift-F` | Search (`Enter` next, `Shift-Enter` previous, `Esc` close) |
| `Ctrl-Shift-C` / `Ctrl-Shift-V` | Copy / paste |
| `Ctrl-Shift-A` | Select all |
| `Ctrl-=` / `Ctrl--` / `Ctrl-0` | Zoom in / out / reset |
| `Alt-PageUp` / `Alt-PageDown` | Scroll page |
| `Ctrl-Shift-Up` / `Down` | Scroll line |
| `Ctrl-PageUp` / `PageDown` | Scroll to top / bottom |
| `Ctrl-Q` | Quit |

Inside terminal input, the app intercepts only the shortcuts above; everything
else (including `Ctrl-C`) is sent to the shell using standard xterm encodings,
with application-cursor mode, bracketed paste and modifier encodings honoured.

## Architecture notes

* **VT state is not reimplemented.** Tabby uses xterm.js; the Rust equivalent is
  `alacritty_terminal`. `tabby-core::terminal::Terminal` wraps it with a small
  render/selection/search API and colour resolution.
* **Rendering is cell-oriented.** The GPUI pane builds shaped text runs per
  visible row, paints per-cell backgrounds, then paints glyphs and the cursor.
  Each row is shaped by GPUI's text system, so fallback fonts and font features
  work without a custom glyph atlas.
* **Sessions are byte transports.** The UI owns VT state and receives
  `Data`/`Exited`/`Closed`/`Error` events. Terminal-originated events (title,
  clipboard, colour queries) are drained after every feed and answers are
  written back to the session.
* **Config is the contract.** Profile parsing is shared by the UI and recovery;
  the core can be used headlessly.

## Licensing

The Rust code in this repository is Apache-2.0. It depends on
`alacritty_terminal` (Apache-2.0), `gpui` (Apache-2.0), `russh` (Apache-2.0),
`portable-pty` (MIT) and the Rust ecosystem; see `cargo metadata` for the full
dependency licences. Tabby itself is MIT/Apache-2.0 and is referenced only for
configuration/profile compatibility and behaviour.
