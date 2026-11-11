use std::cell::RefCell;
use std::ops::RangeInclusive;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    Bounds, Context, CursorStyle as GpuiCursorStyle, FocusHandle, FontStyle, FontWeight, Hsla,
    KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    ScrollDelta, ScrollWheelEvent, ShapedLine, SharedString, StrikethroughStyle, Task, TextRun,
    Timer, UnderlineStyle, Window, canvas, div, fill, point, prelude::*, px, size,
};
use tabby_core::event::Event as TerminalEvent;
use tabby_core::grid::Scroll;
use tabby_core::index::{Column, Direction, Line, Point as TermPoint, Side};
use tabby_core::selection::SelectionType;
use tabby_core::term::TermMode;
use tabby_core::term::cell::{Cell, Flags};
use tabby_core::term::search::RegexSearch;
use tabby_core::vte::ansi::{CursorShape, Rgb};
use tabby_core::{
    ColorScheme, Config, Profile, Session, SessionEvent, SessionKind, TermSize, Terminal,
};

use crate::text_input::TextInput;
use crate::theme::{Theme, hsla_from_rgb8};

const INITIAL_COLUMNS: usize = 80;
const INITIAL_ROWS: usize = 24;

struct Metrics {
    cell_width: f32,
    cell_height: f32,
    ascent: f32,
    font_size: f32,
    font_id: gpui::FontId,
    line_height: Pixels,
}

struct PaintedRow {
    backgrounds: Vec<(f32, f32, f32, f32, Hsla)>,
    line: ShapedLine,
    top: f32,
}

struct Frame {
    background: Hsla,
    rows: Vec<PaintedRow>,
    cursor: Option<(f32, f32, f32, f32, Hsla)>,
    line_height: Pixels,
}

impl Frame {
    fn paint(self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut gpui::App) {
        window.paint_quad(fill(bounds, self.background));
        for row in &self.rows {
            for (x, y, width, height, color) in &row.backgrounds {
                window.paint_quad(fill(
                    Bounds {
                        origin: point(bounds.left() + px(*x), bounds.top() + px(*y)),
                        size: size(px(*width), px(*height)),
                    },
                    *color,
                ));
            }
        }
        for row in &self.rows {
            let origin = point(bounds.left(), bounds.top() + px(row.top));
            if let Err(error) = row.line.paint(origin, self.line_height, window, cx) {
                log::debug!("terminal text paint failed: {error}");
            }
        }
        if let Some((x, y, width, height, color)) = self.cursor {
            window.paint_quad(fill(
                Bounds {
                    origin: point(bounds.left() + px(x), bounds.top() + px(y)),
                    size: size(px(width), px(height)),
                },
                color,
            ));
        }
    }
}

struct SearchState {
    open: bool,
    input: gpui::Entity<TextInput>,
    regex: Option<RegexSearch>,
    query: String,
    last_match: Option<RangeInclusive<TermPoint>>,
    error: Option<String>,
}

impl SearchState {
    fn new(theme: Theme, cx: &mut Context<TerminalPane>) -> Self {
        let input = cx.new(|cx| TextInput::new("Search", false, theme, cx));
        Self {
            open: false,
            input,
            regex: None,
            query: String::new(),
            last_match: None,
            error: None,
        }
    }
}

pub struct TerminalPane {
    focus_handle: FocusHandle,
    terminal: Terminal,
    session: Option<Session>,
    kind: SessionKind,
    profile: Profile,
    title: String,
    settings: tabby_core::config::TerminalSettings,
    scheme: ColorScheme,
    theme: Theme,
    grid_bounds: Rc<RefCell<Option<Bounds<Pixels>>>>,
    metrics_cache: Option<((String, f32, u16), Metrics)>,
    selection_dragging: bool,
    mouse_down_cell: Option<(usize, usize)>,
    mouse_moved_while_down: bool,
    scroll_remainder: f32,
    search: SearchState,
    exited: Option<Option<i32>>,
    error: Option<String>,
    cursor_blink_on: bool,
    cursor_blink_phase: bool,
    _session_task: Option<Task<()>>,
    _blink_task: Option<Task<()>>,
}

impl TerminalPane {
    pub fn new(
        profile: Profile,
        kind: SessionKind,
        session: anyhow::Result<Session>,
        config: &Config,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = config.terminal.clone();
        let cursor_blink_on = settings.cursor_blink;
        let scheme = config.active_color_scheme().clone();
        let theme = Theme::from_scheme(&scheme);
        let cursor_shape = match settings.cursor.as_str() {
            "beam" => CursorShape::Beam,
            "underline" => CursorShape::Underline,
            _ => CursorShape::Block,
        };
        let mut terminal = Terminal::new(
            TermSize::new(INITIAL_COLUMNS, INITIAL_ROWS),
            settings.scrollback_lines,
            cursor_shape,
        );
        let mut startup_error = None;
        let (session, session_task) = match session {
            Ok(session) => {
                let events = session.events();
                let task = cx.spawn(async move |this, cx| {
                    while let Ok(event) = events.recv().await {
                        if this
                            .update(cx, |pane, cx| {
                                pane.on_session_event(event, cx);
                                cx.notify();
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                });
                (Some(session), Some(task))
            }
            Err(spawn_error) => {
                let message = format!("{spawn_error:#}");
                terminal.feed(format!("\r\n\x1b[31m{message}\x1b[0m\r\n").as_bytes());
                startup_error = Some(message);
                (None, None)
            }
        };

        let blink_task = cx.spawn(async move |this, cx| {
            loop {
                Timer::after(Duration::from_millis(530)).await;
                if this
                    .update(cx, |pane, cx| {
                        pane.cursor_blink_phase = !pane.cursor_blink_phase;
                        if pane.cursor_blink_on {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        Self {
            focus_handle: cx.focus_handle(),
            terminal,
            session,
            kind,
            title: if profile.name.is_empty() {
                profile.id.clone()
            } else {
                profile.name.clone()
            },
            profile,
            settings,
            scheme,
            theme,
            grid_bounds: Rc::new(RefCell::new(None)),
            metrics_cache: None,
            selection_dragging: false,
            mouse_down_cell: None,
            mouse_moved_while_down: false,
            scroll_remainder: 0.0,
            search: SearchState::new(theme, cx),
            exited: None,
            error: startup_error,
            cursor_blink_on,
            cursor_blink_phase: true,
            _session_task: session_task,
            _blink_task: Some(blink_task),
        }
    }

    pub fn title(&self) -> String {
        self.title.clone()
    }

    pub fn kind(&self) -> &SessionKind {
        &self.kind
    }

    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    pub fn apply_config(&mut self, config: &Config, cx: &mut Context<Self>) {
        self.settings = config.terminal.clone();
        self.scheme = config.active_color_scheme().clone();
        self.theme = Theme::from_scheme(&self.scheme);
        let theme = self.theme;
        self.cursor_blink_on = self.settings.cursor_blink;
        let shape = match self.settings.cursor.as_str() {
            "beam" => CursorShape::Beam,
            "underline" => CursorShape::Underline,
            _ => CursorShape::Block,
        };
        self.terminal.set_cursor_shape(shape);
        self.search
            .input
            .update(cx, |input, cx| input.set_theme(theme, cx));
        cx.notify();
    }

    pub fn working_directory(&self) -> Option<std::path::PathBuf> {
        self.session
            .as_ref()
            .and_then(|session| session.working_directory())
    }

    pub fn scroll_command(&mut self, command: ScrollCommand) {
        match command {
            ScrollCommand::PageUp => self.terminal.scroll_display(Scroll::PageUp),
            ScrollCommand::PageDown => self.terminal.scroll_display(Scroll::PageDown),
            ScrollCommand::LineUp => self.terminal.scroll_display(Scroll::Delta(1)),
            ScrollCommand::LineDown => self.terminal.scroll_display(Scroll::Delta(-1)),
            ScrollCommand::Top => self.terminal.scroll_display(Scroll::Top),
            ScrollCommand::Bottom => self.terminal.scroll_display(Scroll::Bottom),
        }
    }

    fn write(&self, data: &[u8]) {
        if let Some(session) = &self.session {
            session.write(data);
        }
    }

    fn on_session_event(&mut self, event: SessionEvent, cx: &mut Context<Self>) {
        match event {
            SessionEvent::Data(data) => {
                self.terminal.feed(&data);
                self.process_terminal_events(cx);
            }
            SessionEvent::Exited(code) => {
                self.exited = Some(code);
                self.process_terminal_events(cx);
            }
            SessionEvent::Closed => {
                if self.exited.is_none() {
                    self.exited = Some(None);
                }
            }
            SessionEvent::Error(message) => {
                self.error = Some(message.clone());
                self.terminal
                    .feed(format!("\r\n\x1b[31m{message}\x1b[0m\r\n").as_bytes());
                self.process_terminal_events(cx);
            }
        }
    }

    fn process_terminal_events(&mut self, cx: &mut Context<Self>) {
        for event in self.terminal.take_events() {
            match event {
                TerminalEvent::Title(title) => {
                    self.title = title;
                }
                TerminalEvent::ResetTitle => {
                    self.title = if self.profile.name.is_empty() {
                        self.kind.title()
                    } else {
                        self.profile.name.clone()
                    };
                }
                TerminalEvent::ClipboardStore(_, text) => {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
                }
                TerminalEvent::ClipboardLoad(_, formatter) => {
                    let text = cx
                        .read_from_clipboard()
                        .and_then(|item| item.text())
                        .unwrap_or_default();
                    self.write(formatter(&text).as_bytes());
                }
                TerminalEvent::PtyWrite(text) => {
                    self.write(text.as_bytes());
                }
                TerminalEvent::ColorRequest(index, formatter) => {
                    let rgb = self.scheme.palette(index as u8);
                    let response = formatter(Rgb {
                        r: rgb.r,
                        g: rgb.g,
                        b: rgb.b,
                    });
                    self.write(response.as_bytes());
                }
                TerminalEvent::TextAreaSizeRequest(formatter) => {
                    let metrics = self.metrics_cache.as_ref().map(|(_, metrics)| metrics);
                    let response = formatter(tabby_core::event::WindowSize {
                        num_lines: self.terminal.screen_lines() as u16,
                        num_cols: self.terminal.columns() as u16,
                        cell_width: metrics.map(|m| m.cell_width as u16).unwrap_or(8),
                        cell_height: metrics.map(|m| m.cell_height as u16).unwrap_or(16),
                    });
                    self.write(response.as_bytes());
                }
                _ => {}
            }
        }
    }

    fn base_font(&self) -> gpui::Font {
        let mut font = gpui::font(self.settings.font.clone());
        font.weight = FontWeight(f32::from(self.settings.font_weight));
        font
    }

    fn metrics(&mut self, window: &Window) -> Metrics {
        let key = (
            self.settings.font.clone(),
            self.settings.font_size,
            self.settings.font_weight,
        );
        if let Some((cached_key, metrics)) = &self.metrics_cache {
            if *cached_key == key {
                return Metrics {
                    cell_width: metrics.cell_width,
                    cell_height: metrics.cell_height,
                    ascent: metrics.ascent,
                    font_size: metrics.font_size,
                    font_id: metrics.font_id,
                    line_height: metrics.line_height,
                };
            }
        }

        let font = self.base_font();
        let font_id = window.text_system().resolve_font(&font);
        let font_size = px(self.settings.font_size);
        let probe = "MMMMMMMMMM";
        let run = TextRun {
            len: probe.len(),
            font: font.clone(),
            color: self.theme.foreground,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line =
            window
                .text_system()
                .shape_line(SharedString::from(probe), font_size, &[run], None);
        let cell_width = (f32::from(line.width) / probe.len() as f32).max(1.0);
        // Use the shaped line's metrics: GPUI's FontMetrics::descent is stored
        // negated on Linux while LineLayout stores the positive distance.
        let ascent = f32::from(line.ascent);
        let descent = f32::from(line.descent);
        let cell_height = (ascent + descent + self.settings.line_padding).max(1.0);
        let metrics = Metrics {
            cell_width,
            cell_height,
            ascent,
            font_size: self.settings.font_size,
            font_id,
            line_height: px(cell_height),
        };
        self.metrics_cache = Some((
            key,
            Metrics {
                cell_width: metrics.cell_width,
                cell_height: metrics.cell_height,
                ascent: metrics.ascent,
                font_size: metrics.font_size,
                font_id: metrics.font_id,
                line_height: metrics.line_height,
            },
        ));
        metrics
    }

    fn grid_size(&self, metrics: &Metrics) -> (usize, usize) {
        let bounds = *self.grid_bounds.borrow();
        match bounds {
            Some(bounds) => {
                let columns = (f32::from(bounds.size.width) / metrics.cell_width)
                    .floor()
                    .max(2.0) as usize;
                let rows = (f32::from(bounds.size.height) / metrics.cell_height)
                    .floor()
                    .max(1.0) as usize;
                (columns, rows)
            }
            None => (INITIAL_COLUMNS, INITIAL_ROWS),
        }
    }

    fn sync_size(&mut self, columns: usize, rows: usize) {
        let size = TermSize::new(columns, rows);
        if size != self.terminal.size() {
            self.terminal.resize(size.columns, size.screen_lines);
            if let Some(session) = &self.session {
                session.resize(size.columns as u16, size.screen_lines as u16);
            }
        }
    }

    fn build_frame(
        &mut self,
        metrics: &Metrics,
        columns: usize,
        rows: usize,
        window: &Window,
    ) -> Frame {
        let scheme = self.scheme.clone();
        let bold_bright = self.settings.draw_bold_text_in_bright_colors;
        let base_font = self.base_font();
        let _ = window;
        let mut frame_rows = Vec::with_capacity(rows);
        let selection = self.terminal.selection_range();
        let cursor = self.terminal.renderable().cursor;
        let search_match = self.search.last_match.clone();

        for row in 0..rows {
            let mut text = String::with_capacity(columns);
            let mut runs: Vec<TextRun> = Vec::with_capacity(columns);
            let mut backgrounds = Vec::new();
            for column in 0..columns {
                let point = match self.terminal.viewport_point(row, column) {
                    Some(point) => point,
                    None => break,
                };
                let cell = match self.terminal.cell(row, column) {
                    Some(cell) => cell,
                    None => break,
                };
                let resolved = self.terminal.resolve_cell(cell, &scheme, bold_bright);
                if resolved.leading_wide_spacer || resolved.wide_spacer {
                    continue;
                }

                let mut glyph = cell.c.to_string();
                if let Some(extra) = cell.zerowidth() {
                    for ch in extra {
                        glyph.push(*ch);
                    }
                }
                let len = glyph.len();
                let mut fg = resolved.fg;
                if resolved.hidden {
                    fg = resolved.bg;
                }
                let mut bg = resolved.bg;
                let mut is_selected = false;
                if let Some(selection) = &selection {
                    if selection.contains(point) {
                        is_selected = true;
                        bg = scheme.selection_rgb();
                    }
                }
                if let Some(range) = &search_match {
                    if range.contains(&point) {
                        bg = scheme.ansi(3).mix(scheme.background_rgb(), 0.35);
                        fg = scheme.background_rgb();
                        is_selected = true;
                    }
                }
                let width = if resolved.wide {
                    metrics.cell_width * 2.0
                } else {
                    metrics.cell_width
                };
                if bg != scheme.background_rgb() || is_selected {
                    backgrounds.push((
                        column as f32 * metrics.cell_width,
                        row as f32 * metrics.cell_height,
                        width,
                        metrics.cell_height,
                        hsla_from_rgb8(bg),
                    ));
                }

                let mut font = base_font.clone();
                font.weight = FontWeight(if resolved.bold {
                    f32::from(self.settings.font_weight_bold)
                } else {
                    f32::from(self.settings.font_weight)
                });
                if resolved.italic {
                    font.style = FontStyle::Italic;
                }
                runs.push(TextRun {
                    len,
                    font,
                    color: hsla_from_rgb8(fg),
                    background_color: None,
                    underline: resolved.underline.then(|| UnderlineStyle {
                        thickness: px(1.0),
                        color: Some(hsla_from_rgb8(fg)),
                        wavy: false,
                    }),
                    strikethrough: resolved.strikeout.then(|| StrikethroughStyle {
                        thickness: px(1.0),
                        color: Some(hsla_from_rgb8(fg)),
                    }),
                });
                text.push_str(&glyph);
            }

            let line = window.text_system().shape_line(
                SharedString::from(text),
                px(metrics.font_size),
                &runs,
                None,
            );
            frame_rows.push(PaintedRow {
                backgrounds,
                line,
                top: row as f32 * metrics.cell_height,
            });
        }

        let mut cursor_paint = None;
        let mode_visible = self.terminal.mode().contains(TermMode::SHOW_CURSOR);
        if mode_visible
            && cursor.shape != CursorShape::Hidden
            && (self.cursor_blink_phase || !self.cursor_blink_on)
        {
            if let Some(row) = self.terminal.visible_row(cursor.point) {
                let column = cursor.point.column.0;
                let color = hsla_from_rgb8(scheme.cursor_rgb());
                let color = Hsla { a: 0.72, ..color };
                let (x, y, width, height) = match cursor.shape {
                    CursorShape::Beam => (
                        column as f32 * metrics.cell_width,
                        row as f32 * metrics.cell_height,
                        2.0,
                        metrics.cell_height,
                    ),
                    CursorShape::Underline => (
                        column as f32 * metrics.cell_width,
                        (row + 1) as f32 * metrics.cell_height - 2.0,
                        metrics.cell_width,
                        2.0,
                    ),
                    CursorShape::HollowBlock => (
                        column as f32 * metrics.cell_width,
                        row as f32 * metrics.cell_height,
                        metrics.cell_width,
                        metrics.cell_height,
                    ),
                    _ => (
                        column as f32 * metrics.cell_width,
                        row as f32 * metrics.cell_height,
                        metrics.cell_width,
                        metrics.cell_height,
                    ),
                };
                cursor_paint = Some((x, y, width, height, color));
            }
        }

        Frame {
            background: hsla_from_rgb8(scheme.background_rgb()),
            rows: frame_rows,
            cursor: cursor_paint,
            line_height: px(metrics.cell_height),
        }
    }

    fn point_for_position(
        &self,
        metrics: &Metrics,
        position: Point<Pixels>,
    ) -> Option<(TermPoint, Side)> {
        let bounds = (*self.grid_bounds.borrow())?;
        let local = bounds.localize(&position)?;
        let x = f32::from(local.x);
        let y = f32::from(local.y);
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let column = (x / metrics.cell_width).floor() as usize;
        let row = (y / metrics.cell_height).floor() as usize;
        let side = if x % metrics.cell_width < metrics.cell_width / 2.0 {
            Side::Left
        } else {
            Side::Right
        };
        let point = self
            .terminal
            .viewport_point(row, column.min(self.terminal.columns().saturating_sub(1)))?;
        Some((point, side))
    }

    fn cell_index_for_position(
        &self,
        metrics: &Metrics,
        position: Point<Pixels>,
    ) -> Option<(usize, usize)> {
        let bounds = (*self.grid_bounds.borrow())?;
        let local = bounds.localize(&position)?;
        let x = f32::from(local.x);
        let y = f32::from(local.y);
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let column = (x / metrics.cell_width).floor() as usize;
        let row = (y / metrics.cell_height).floor() as usize;
        Some((row, column))
    }

    fn handle_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // The search field and dialogs are descendants of the pane; only send
        // keys to the shell while the pane itself owns focus.
        if !self.focus_handle.is_focused(window) {
            return;
        }
        if self.is_app_shortcut(event) {
            return;
        }
        if let Some(bytes) = self.key_to_bytes(event) {
            self.write(&bytes);
            if self.settings.scroll_on_input {
                self.terminal.scroll_to_bottom();
            }
            cx.notify();
        }
    }

    /// Shortcuts handled by the app's key bindings. The pane must not also send
    /// them to the shell (otherwise Ctrl-Shift-T would type a control byte).
    fn is_app_shortcut(&self, event: &KeyDownEvent) -> bool {
        let m = event.keystroke.modifiers;
        let key = event.keystroke.key.as_str();
        if m.control && m.shift {
            return matches!(
                key,
                "t" | "w"
                    | "s"
                    | "d"
                    | "f"
                    | "c"
                    | "v"
                    | "a"
                    | "left"
                    | "right"
                    | "pageup"
                    | "pagedown"
                    | "up"
                    | "down"
                    | "e"
            );
        }
        if m.control && m.alt {
            return matches!(key, "t" | "left" | "right" | "up" | "down" | "enter");
        }
        if m.control && !m.shift && !m.alt {
            return key == "tab"
                || key == "pageup"
                || key == "pagedown"
                || key == "0"
                || key == "-"
                || key == "=";
        }
        if m.alt && !m.control {
            return key == "pageup"
                || key == "pagedown"
                || matches!(
                    key,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "0"
                );
        }
        key == "f11"
    }

    fn key_to_bytes(&self, event: &KeyDownEvent) -> Option<Vec<u8>> {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        let key = keystroke.key.as_str();
        let mode = self.terminal.mode();
        let app_cursor = mode.contains(TermMode::APP_CURSOR);
        let modifier_code = 1
            + u8::from(modifiers.shift)
            + u8::from(modifiers.alt) * 2
            + u8::from(modifiers.control) * 4;

        let modified_csi = |final_byte: char| -> Vec<u8> {
            format!("\x1b[1;{modifier_code}{final_byte}").into_bytes()
        };

        match key {
            "enter" | "return" => Some(vec![b'\r']),
            "tab" => Some(if modifiers.shift {
                b"\x1b[Z".to_vec()
            } else {
                vec![b'\t']
            }),
            "backspace" => Some(vec![0x7f]),
            "escape" => Some(vec![0x1b]),
            "up" | "down" | "right" | "left" => {
                let letter = match key {
                    "up" => 'A',
                    "down" => 'B',
                    "right" => 'C',
                    _ => 'D',
                };
                if modifier_code > 1 {
                    Some(modified_csi(letter))
                } else if app_cursor {
                    Some(format!("\x1bO{letter}").into_bytes())
                } else {
                    Some(format!("\x1b[{letter}").into_bytes())
                }
            }
            "home" | "end" => {
                let letter = if key == "home" { 'H' } else { 'F' };
                if modifier_code > 1 {
                    Some(modified_csi(letter))
                } else {
                    Some(format!("\x1b[{letter}").into_bytes())
                }
            }
            "insert" | "delete" | "pageup" | "pagedown" => {
                let code = match key {
                    "insert" => 2,
                    "delete" => 3,
                    "pageup" => 5,
                    _ => 6,
                };
                if modifier_code > 1 {
                    Some(format!("\x1b[{code};{modifier_code}~").into_bytes())
                } else {
                    Some(format!("\x1b[{code}~").into_bytes())
                }
            }
            "f1" | "f2" | "f3" | "f4" => {
                let letter = match key {
                    "f1" => 'P',
                    "f2" => 'Q',
                    "f3" => 'R',
                    _ => 'S',
                };
                if modifier_code > 1 {
                    Some(format!("\x1b[1;{modifier_code}{letter}").into_bytes())
                } else {
                    Some(format!("\x1bO{letter}").into_bytes())
                }
            }
            "f5" | "f6" | "f7" | "f8" | "f9" | "f10" | "f11" | "f12" => {
                let code = match key {
                    "f5" => 15,
                    "f6" => 17,
                    "f7" => 18,
                    "f8" => 19,
                    "f9" => 20,
                    "f10" => 21,
                    "f11" => 23,
                    _ => 24,
                };
                if modifier_code > 1 {
                    Some(format!("\x1b[{code};{modifier_code}~").into_bytes())
                } else {
                    Some(format!("\x1b[{code}~").into_bytes())
                }
            }
            _ => {
                if modifiers.control {
                    let ch = keystroke
                        .key_char
                        .as_deref()
                        .and_then(|text| text.chars().next())
                        .or_else(|| key.chars().next())?;
                    let byte = match ch {
                        'a'..='z' => (ch as u8) - b'a' + 1,
                        'A'..='Z' => (ch as u8) - b'A' + 1,
                        ' ' | '@' => 0,
                        '[' => 0x1b,
                        '\\' => 0x1c,
                        ']' => 0x1d,
                        '^' => 0x1e,
                        '_' => 0x1f,
                        _ => return None,
                    };
                    Some(vec![byte])
                } else if let Some(text) = &keystroke.key_char {
                    if text.is_empty() {
                        return None;
                    }
                    let mut bytes = Vec::new();
                    if modifiers.alt {
                        bytes.push(0x1b);
                    }
                    bytes.extend_from_slice(text.as_bytes());
                    Some(bytes)
                } else {
                    None
                }
            }
        }
    }

    fn handle_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window);
        let metrics = self.metrics(window);
        match event.button {
            MouseButton::Navigate(_) => {}
            MouseButton::Left => {
                self.mouse_moved_while_down = false;
                if let Some((row, column)) = self.cell_index_for_position(&metrics, event.position)
                {
                    self.mouse_down_cell = Some((row, column));
                }
                if let Some((point, side)) = self.point_for_position(&metrics, event.position) {
                    if event.modifiers.shift && self.terminal.has_selection() {
                        self.terminal.selection_extend(point, side);
                    } else {
                        self.terminal.selection_start(point, side);
                    }
                    self.selection_dragging = true;
                    cx.notify();
                }
            }
            MouseButton::Middle => {
                if self.settings.paste_on_middle_click {
                    self.paste(cx);
                }
            }
            MouseButton::Right => {
                if self.settings.right_click == "clipboard" {
                    self.paste(cx);
                }
            }
        }
    }

    fn handle_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if !self.selection_dragging {
            return;
        }
        let metrics = self.metrics(window);
        if let Some((point, side)) = self.point_for_position(&metrics, event.position) {
            self.mouse_moved_while_down = true;
            self.terminal.selection_update(point, side);
            cx.notify();
        }
    }

    fn handle_mouse_up(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        if event.button != MouseButton::Left {
            return;
        }
        let was_dragging = self.selection_dragging;
        self.selection_dragging = false;
        if was_dragging && self.settings.copy_on_select {
            if let Some(text) = self.terminal.selection_text() {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
            }
        }
        // A plain click on a hyperlink (OSC 8 or a URL in the text) opens it.
        if !self.mouse_moved_while_down {
            if let Some((row, column)) = self.mouse_down_cell.take() {
                if let Some(url) = self.url_at(row, column) {
                    cx.open_url(&url);
                }
            }
        } else {
            self.mouse_down_cell = None;
        }
        cx.notify();
    }

    fn handle_scroll(&mut self, event: &ScrollWheelEvent, window: &Window, cx: &mut Context<Self>) {
        let metrics = self.metrics(window);
        let lines = match event.delta {
            ScrollDelta::Lines(delta) => -f32::from(delta.y),
            ScrollDelta::Pixels(delta) => -f32::from(delta.y) / metrics.cell_height,
        };
        self.scroll_remainder += lines;
        let whole = self.scroll_remainder.trunc();
        if whole != 0.0 {
            self.scroll_remainder -= whole;
            self.terminal.scroll_display(Scroll::Delta(whole as i32));
            cx.notify();
        }
    }

    fn url_at(&self, row: usize, column: usize) -> Option<String> {
        let cell = self.terminal.cell(row, column)?;
        if let Some(url) = self.terminal.cell_hyperlink(cell) {
            return Some(url);
        }
        let mut text = String::new();
        let mut cell_to_char = Vec::new();
        for c in 0..self.terminal.columns() {
            let cell = self.terminal.cell(row, c)?;
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            cell_to_char.push((c, text.len()));
            text.push(cell.c);
        }
        let char_index = cell_to_char
            .iter()
            .find(|(cell_column, _)| *cell_column == column)
            .map(|(_, index)| *index)?;
        let mut token_start = None;
        let mut token_end = None;
        for (index, ch) in text.char_indices() {
            if ch.is_whitespace() {
                if let Some(start) = token_start {
                    if start <= char_index && char_index < index {
                        token_end = Some(index);
                        break;
                    }
                    token_start = None;
                }
            } else if token_start.is_none() {
                token_start = Some(index);
                token_end = Some(index + ch.len_utf8());
            } else {
                token_end = Some(index + ch.len_utf8());
            }
        }
        let start = token_start?;
        let end = token_end?;
        if !(start <= char_index && char_index < end) {
            return None;
        }
        let word = text[start..end].trim_matches(|c: char| "()[]{}<>,;\"'".contains(c));
        if word.starts_with("http://")
            || word.starts_with("https://")
            || word.starts_with("ftp://")
            || word.starts_with("file://")
        {
            Some(word.to_string())
        } else {
            None
        }
    }

    pub fn copy_selection(&self) -> Option<String> {
        self.terminal.selection_text()
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.terminal.select_all();
        cx.notify();
    }

    pub fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let mut text = text;
        if self.settings.replace_newlines_with_spaces_on_paste {
            text = text.replace(['\r', '\n'], " ");
        }
        if self.settings.trim_whitespace_on_paste {
            text = text.trim_end().to_string();
        }
        if text.is_empty() {
            return;
        }
        let bracketed = self.settings.bracketed_paste
            && self.terminal.mode().contains(TermMode::BRACKETED_PASTE);
        if bracketed {
            self.write(b"\x1b[200~");
        }
        self.write(text.as_bytes());
        if bracketed {
            self.write(b"\x1b[201~");
        }
        if self.settings.scroll_on_input {
            self.terminal.scroll_to_bottom();
        }
        cx.notify();
    }

    pub fn begin_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.open = true;
        let handle = self.search.input.read(cx).focus_handle();
        handle.focus(window);
        cx.notify();
    }

    pub fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.open = false;
        self.search.last_match = None;
        self.focus_handle.focus(window);
        cx.notify();
    }

    fn rebuild_search(&mut self, cx: &mut Context<Self>) {
        let query = self.search.input.read(cx).value().to_string();
        if query.is_empty() {
            self.search.regex = None;
            self.search.query.clear();
            self.search.last_match = None;
            return;
        }
        if query != self.search.query {
            self.search.query = query;
            match tabby_core::terminal::build_search(
                &self.search.query,
                &self.settings.search_options,
            ) {
                Some(regex) => {
                    self.search.regex = Some(regex);
                    self.search.error = None;
                }
                None => {
                    self.search.regex = None;
                    self.search.error = Some("invalid search expression".into());
                }
            }
        }
    }

    pub fn find(&mut self, forward: bool, cx: &mut Context<Self>) {
        self.rebuild_search(cx);
        if self.search.regex.is_none() {
            cx.notify();
            return;
        }
        let origin = self
            .search
            .last_match
            .as_ref()
            .map(|range| {
                if forward {
                    *range.end()
                } else {
                    *range.start()
                }
            })
            .unwrap_or_else(|| self.terminal.renderable().cursor.point);
        let (direction, side) = if forward {
            (Direction::Right, Side::Right)
        } else {
            (Direction::Left, Side::Left)
        };
        let found = {
            let regex = self.search.regex.as_mut().expect("checked above");
            self.terminal.search_next(regex, origin, direction, side)
        };
        match found {
            Some(range) => {
                self.terminal.scroll_to_point(*range.start());
                self.search.last_match = Some(range);
                self.search.error = None;
            }
            None => {
                self.search.error = Some("no match".into());
            }
        }
        cx.notify();
    }

    fn render_search_bar(&self, theme: Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let input = self.search.input.clone();
        let close = {
            let this = this.clone();
            move |_: &gpui::MouseDownEvent, window: &mut Window, cx: &mut gpui::App| {
                let _ = this.update(cx, |pane, cx| pane.close_search(window, cx));
            }
        };
        let prev = {
            let this = this.clone();
            move |_: &gpui::MouseDownEvent, _window: &mut Window, cx: &mut gpui::App| {
                let _ = this.update(cx, |pane, cx| pane.find(false, cx));
            }
        };
        let next = {
            let this = this.clone();
            move |_: &gpui::MouseDownEvent, _window: &mut Window, cx: &mut gpui::App| {
                let _ = this.update(cx, |pane, cx| pane.find(true, cx));
            }
        };
        div()
            .key_context("search")
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .bg(theme.surface)
            .border_b_1()
            .border_color(theme.border)
            .child(div().w(px(260.0)).child(input))
            .child(search_button("↑", prev, theme))
            .child(search_button("↓", next, theme))
            .when_some(self.search.error.clone(), |this, error| {
                this.child(div().text_xs().text_color(theme.error).child(error))
            })
            .child(search_button("×", close, theme))
    }
}

fn search_button(
    label: &'static str,
    listener: impl Fn(&gpui::MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
    theme: Theme,
) -> impl IntoElement {
    div()
        .w(px(22.0))
        .h(px(22.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.0))
        .text_sm()
        .text_color(theme.foreground)
        .hover(|style| style.bg(theme.surface_active))
        .on_mouse_down(gpui::MouseButton::Left, listener)
        .child(label)
}

impl Render for TerminalPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let metrics = self.metrics(window);
        let (columns, rows) = self.grid_size(&metrics);
        self.sync_size(columns, rows);
        let frame = self.build_frame(&metrics, columns, rows, window);
        let theme = self.theme;

        let grid_bounds = self.grid_bounds.clone();
        let this_for_frame = cx.entity().downgrade();
        let grid = canvas(
            move |bounds, window, _cx| {
                let previous = *grid_bounds.borrow();
                grid_bounds.replace(Some(bounds));
                if previous.map(|b| (b.size.width, b.size.height))
                    != Some((bounds.size.width, bounds.size.height))
                {
                    // The frame was built from the previous bounds; ask for one
                    // more frame so the grid matches the new size.
                    let this = this_for_frame.clone();
                    window.on_next_frame(move |_, cx| {
                        let _ = this.update(cx, |_, cx| cx.notify());
                    });
                }
            },
            move |bounds, _, window, cx| frame.paint(bounds, window, cx),
        )
        .size_full();

        let focus_handle = self.focus_handle.clone();
        let key_listener = cx
            .listener(|this, event: &KeyDownEvent, window, cx| this.handle_key(event, window, cx));
        let mouse_down = cx.listener(|this, event: &MouseDownEvent, window, cx| {
            this.handle_mouse_down(event, window, cx)
        });
        let mouse_move = cx.listener(|this, event: &MouseMoveEvent, window, cx| {
            this.handle_mouse_move(event, window, cx)
        });
        let mouse_up =
            cx.listener(|this, event: &MouseUpEvent, _window, cx| this.handle_mouse_up(event, cx));
        let scroll_listener = cx.listener(|this, event: &ScrollWheelEvent, window, cx| {
            this.handle_scroll(event, window, cx)
        });

        let mut container = div()
            .key_context("terminal")
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.background)
            .track_focus(&focus_handle)
            .on_key_down(key_listener)
            .on_mouse_down(MouseButton::Left, mouse_down)
            .on_mouse_up(MouseButton::Left, mouse_up)
            .on_mouse_move(mouse_move)
            .on_scroll_wheel(scroll_listener)
            .cursor(GpuiCursorStyle::IBeam)
            .child(div().relative().flex_1().child(grid));

        if self.search.open {
            container = container.child(self.render_search_bar(theme, cx));
        }

        if let Some(code) = self.exited {
            let label = match code {
                Some(code) => format!("Process exited ({code})"),
                None => "Process exited".to_string(),
            };
            container = container.child(
                div()
                    .absolute()
                    .top_1()
                    .right_2()
                    .px_2()
                    .py_1()
                    .rounded(px(3.0))
                    .bg(theme.surface_active)
                    .text_color(theme.muted)
                    .text_xs()
                    .child(label),
            );
        }

        if let Some(error) = &self.error {
            let error = error.clone();
            container = container.child(
                div()
                    .absolute()
                    .bottom_2()
                    .left_2()
                    .px_2()
                    .py_1()
                    .rounded(px(3.0))
                    .bg(theme.error)
                    .text_color(theme.background)
                    .text_xs()
                    .child(error),
            );
        }

        container
    }
}

#[derive(Clone, Copy)]
pub enum ScrollCommand {
    PageUp,
    PageDown,
    LineUp,
    LineDown,
    Top,
    Bottom,
}

// Keep commonly used terminal types referenced from this module.
#[allow(dead_code)]
fn _type_anchors(_: SelectionType, _: Cell, _: Column, _: Line) {}
