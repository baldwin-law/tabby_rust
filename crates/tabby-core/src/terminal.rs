//! VT emulation.
//!
//! We deliberately do not reimplement escape-sequence parsing: Tabby delegates
//! this to xterm.js, and for the Rust core the equivalent proven engine is
//! `alacritty_terminal`. This module wraps it in a small API the GPUI frontend
//! can render, without exposing the dozens of alacritty-specific types.

use std::ops::RangeInclusive;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Direction, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionRange, SelectionType};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::search::RegexSearch;
use alacritty_terminal::term::{Config as TermConfig, RenderableContent, Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Processor, Rgb};

use crate::colors::{ColorScheme, Rgb8};

/// Sink for terminal-originated events (clipboard, title, bell, device
/// attribute replies). The UI drains these and writes replies back to the
/// session.
#[derive(Clone)]
pub struct EventProxy {
    sender: async_channel::Sender<Event>,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        // `unbounded` send cannot block; dropping a UI-closed channel is fine.
        let _ = self.sender.try_send(event);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TermSize {
    pub columns: usize,
    pub screen_lines: usize,
}

impl TermSize {
    pub fn new(columns: usize, screen_lines: usize) -> Self {
        Self {
            columns: columns.max(2),
            screen_lines: screen_lines.max(1),
        }
    }
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

/// A cell's style after applying the scheme, OSC colour overrides and the
/// terminal's inverse/dim/bold attributes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResolvedCell {
    pub fg: Rgb8,
    pub bg: Rgb8,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikeout: bool,
    pub hidden: bool,
    pub wide: bool,
    pub wide_spacer: bool,
    pub leading_wide_spacer: bool,
}

pub struct Terminal {
    term: Term<EventProxy>,
    parser: Processor,
    events: async_channel::Receiver<Event>,
    size: TermSize,
    bell_count: u64,
}

impl Terminal {
    pub fn new(size: TermSize, scrollback_lines: usize, cursor_shape: CursorShape) -> Self {
        let (sender, events) = async_channel::unbounded();
        let config = TermConfig {
            scrolling_history: scrollback_lines,
            default_cursor_style: alacritty_terminal::vte::ansi::CursorStyle {
                shape: cursor_shape,
                blinking: true,
            },
            semantic_escape_chars: ",│`|:\"' ()[]{}<>".into(),
            ..Default::default()
        };
        let term = Term::new(config, &size, EventProxy { sender });
        Self {
            term,
            parser: Processor::new(),
            events,
            size,
            bell_count: 0,
        }
    }

    /// Feed raw PTY/SSH bytes through the VT parser.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    pub fn resize(&mut self, columns: usize, rows: usize) {
        let size = TermSize::new(columns, rows);
        if size != self.size {
            self.size = size;
            self.term.resize(size);
        }
    }

    pub fn size(&self) -> TermSize {
        self.size
    }

    pub fn columns(&self) -> usize {
        self.size.columns
    }

    pub fn screen_lines(&self) -> usize {
        self.size.screen_lines
    }

    pub fn scrollback_lines(&self) -> usize {
        self.term.grid().history_size()
    }

    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    pub fn scroll_display(&mut self, scroll: Scroll) {
        self.term.scroll_display(scroll);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll_display(Scroll::Bottom);
    }

    /// Scroll the viewport so `point` is visible (used by search).
    pub fn scroll_to_point(&mut self, point: Point) {
        self.term.scroll_to_point(point);
    }

    pub fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    pub fn cursor_style(&self) -> CursorShape {
        self.term.cursor_style().shape
    }

    pub fn set_cursor_shape(&mut self, shape: CursorShape) {
        let config = TermConfig {
            scrolling_history: self.term.grid().history_size(),
            default_cursor_style: alacritty_terminal::vte::ansi::CursorStyle {
                shape,
                blinking: true,
            },
            ..Default::default()
        };
        self.term.set_options(config);
    }

    pub fn renderable(&self) -> RenderableContent<'_> {
        self.term.renderable_content()
    }

    /// Cell at a viewport-relative position (0,0 is the top visible row).
    pub fn cell(&self, row: usize, column: usize) -> Option<&Cell> {
        if row >= self.screen_lines() || column >= self.columns() {
            return None;
        }
        let point = self.viewport_point(row, column)?;
        Some(&self.term.grid()[point.line][point.column])
    }

    /// Convert a viewport row/column into an absolute grid point, accounting
    /// for the current scrollback offset.
    pub fn viewport_point(&self, row: usize, column: usize) -> Option<Point> {
        if row >= self.screen_lines() || column >= self.columns() {
            return None;
        }
        let line = Line(row as i32 - self.display_offset() as i32);
        Some(Point::new(line, Column(column)))
    }

    /// Convert an absolute grid point to a viewport row, if visible.
    pub fn visible_row(&self, point: Point) -> Option<usize> {
        let row = point.line.0 + self.display_offset() as i32;
        (0..self.screen_lines() as i32)
            .contains(&row)
            .then_some(row as usize)
    }

    pub fn selection_start(&mut self, point: Point, side: Side) {
        self.term.selection = Some(Selection::new(SelectionType::Simple, point, side));
    }

    pub fn selection_update(&mut self, point: Point, side: Side) {
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, side);
        } else {
            self.selection_start(point, side);
        }
    }

    pub fn selection_extend(&mut self, point: Point, side: Side) {
        if let Some(selection) = self.term.selection.as_mut() {
            selection.update(point, side);
        }
    }

    pub fn clear_selection(&mut self) {
        self.term.selection = None;
    }

    pub fn has_selection(&self) -> bool {
        self.term.selection.as_ref().is_some_and(|selection| {
            selection
                .to_range(&self.term)
                .is_some_and(|range| range.start != range.end)
        })
    }

    pub fn selection_range(&self) -> Option<SelectionRange> {
        self.term
            .selection
            .as_ref()
            .and_then(|s| s.to_range(&self.term))
    }

    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string()
    }

    /// Select the whole visible screen + scrollback, matching Tabby's
    /// `select-all` command.
    pub fn select_all(&mut self) {
        let top = self.term.topmost_line();
        let bottom = self.term.bottommost_line();
        let mut selection = Selection::new(
            SelectionType::Simple,
            Point::new(top, Column(0)),
            Side::Left,
        );
        selection.update(Point::new(bottom, self.term.last_column()), Side::Right);
        selection.include_all();
        self.term.selection = Some(selection);
    }

    pub fn selection_text_range(&self, range: &RangeInclusive<Point>) -> String {
        self.term.bounds_to_string(*range.start(), *range.end())
    }

    pub fn search_next(
        &self,
        regex: &mut RegexSearch,
        origin: Point,
        direction: Direction,
        side: Side,
    ) -> Option<RangeInclusive<Point>> {
        self.term.search_next(regex, origin, direction, side, None)
    }

    /// Drain terminal events accumulated since the last call.
    pub fn take_events(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            if matches!(event, Event::Bell) {
                self.bell_count = self.bell_count.wrapping_add(1);
            }
            events.push(event);
        }
        events
    }

    pub fn bell_count(&self) -> u64 {
        self.bell_count
    }

    /// Resolve a cell for painting. `bold_bright` reproduces xterm's
    /// "draw bold text in bright colours" behaviour for the base 8 colours.
    pub fn resolve_cell(
        &self,
        cell: &Cell,
        scheme: &ColorScheme,
        bold_bright: bool,
    ) -> ResolvedCell {
        let flags = cell.flags;
        let bold = flags.intersects(Flags::BOLD) || flags.intersects(Flags::BOLD_ITALIC);
        let italic = flags.intersects(Flags::ITALIC) || flags.intersects(Flags::BOLD_ITALIC);
        let inverse = flags.contains(Flags::INVERSE);
        let dim = flags.intersects(Flags::DIM) || flags.intersects(Flags::DIM_BOLD);
        let hidden = flags.contains(Flags::HIDDEN);

        let mut fg = self.color_to_rgb(
            cell.fg,
            scheme,
            scheme.foreground_rgb(),
            bold && bold_bright,
        );
        let mut bg = self.color_to_rgb(cell.bg, scheme, scheme.background_rgb(), false);

        if inverse {
            std::mem::swap(&mut fg, &mut bg);
        }
        if dim {
            fg = fg.mix(bg, 0.5);
        }
        if hidden {
            fg = bg;
        }

        ResolvedCell {
            fg,
            bg,
            bold,
            italic,
            underline: flags.intersects(Flags::ALL_UNDERLINES),
            strikeout: flags.contains(Flags::STRIKEOUT),
            hidden,
            wide: flags.contains(Flags::WIDE_CHAR),
            wide_spacer: flags.contains(Flags::WIDE_CHAR_SPACER),
            leading_wide_spacer: flags.contains(Flags::LEADING_WIDE_CHAR_SPACER),
        }
    }

    fn color_to_rgb(&self, color: Color, scheme: &ColorScheme, default: Rgb8, bold: bool) -> Rgb8 {
        match color {
            Color::Spec(Rgb { r, g, b }) => Rgb8::new(r, g, b),
            Color::Indexed(index) => self.indexed_to_rgb(index, scheme),
            Color::Named(named) => {
                let overridden =
                    self.term.colors()[named].map(|rgb| Rgb8::new(rgb.r, rgb.g, rgb.b));
                if let Some(rgb) = overridden {
                    return rgb;
                }
                let mut rgb = match named {
                    NamedColor::Foreground | NamedColor::BrightForeground => default,
                    NamedColor::Background => scheme.background_rgb(),
                    NamedColor::Cursor => scheme.cursor_rgb(),
                    NamedColor::Black
                    | NamedColor::Red
                    | NamedColor::Green
                    | NamedColor::Yellow
                    | NamedColor::Blue
                    | NamedColor::Magenta
                    | NamedColor::Cyan
                    | NamedColor::White
                    | NamedColor::BrightBlack
                    | NamedColor::BrightRed
                    | NamedColor::BrightGreen
                    | NamedColor::BrightYellow
                    | NamedColor::BrightBlue
                    | NamedColor::BrightMagenta
                    | NamedColor::BrightCyan
                    | NamedColor::BrightWhite => scheme.ansi(named as usize),
                    NamedColor::DimBlack
                    | NamedColor::DimRed
                    | NamedColor::DimGreen
                    | NamedColor::DimYellow
                    | NamedColor::DimBlue
                    | NamedColor::DimMagenta
                    | NamedColor::DimCyan
                    | NamedColor::DimWhite => scheme
                        .ansi(named as usize - NamedColor::DimBlack as usize)
                        .mix(scheme.background_rgb(), 0.5),
                    NamedColor::DimForeground => default.mix(scheme.background_rgb(), 0.5),
                };
                if bold
                    && matches!(
                        named,
                        NamedColor::Black
                            | NamedColor::Red
                            | NamedColor::Green
                            | NamedColor::Yellow
                            | NamedColor::Blue
                            | NamedColor::Magenta
                            | NamedColor::Cyan
                            | NamedColor::White
                    )
                {
                    rgb = scheme.ansi(named as usize + 8);
                }
                rgb
            }
        }
    }

    fn indexed_to_rgb(&self, index: u8, scheme: &ColorScheme) -> Rgb8 {
        if let Some(rgb) = self.term.colors()[usize::from(index)] {
            return Rgb8::new(rgb.r, rgb.g, rgb.b);
        }
        scheme.palette(index)
    }

    pub fn cell_hyperlink(&self, cell: &Cell) -> Option<String> {
        cell.hyperlink().map(|link| link.uri().to_string())
    }
}

/// Search over the terminal grid, honouring the options Tabby exposes.
pub fn build_search(query: &str, options: &crate::config::SearchOptions) -> Option<RegexSearch> {
    let mut pattern = if options.regex {
        query.to_string()
    } else {
        regex::escape(query)
    };
    if options.whole_word {
        pattern = format!(r"\b(?:{pattern})\b");
    }
    if !options.case_sensitive {
        pattern = format!("(?i:{pattern})");
    }
    RegexSearch::new(&pattern).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TerminalSettings;

    fn feed(term: &mut Terminal, text: &str) {
        term.feed(text.as_bytes());
    }

    fn row_text(term: &Terminal, row: usize) -> String {
        (0..term.columns())
            .filter_map(|column| term.cell(row, column))
            .filter(|cell| !cell.flags.contains(Flags::WIDE_CHAR_SPACER))
            .map(|cell| cell.c)
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn parses_basic_text_crlf_and_colors() {
        let mut term = Terminal::new(TermSize::new(20, 4), 100, CursorShape::Block);
        feed(&mut term, "hello\r\n\x1b[31mred\x1b[0m");
        assert_eq!(row_text(&term, 0), "hello");
        assert_eq!(row_text(&term, 1), "red");
        let cell = term.cell(1, 0).unwrap();
        assert_eq!(cell.fg, Color::Named(NamedColor::Red));
    }

    #[test]
    fn scrollback_and_viewport_mapping() {
        let mut term = Terminal::new(TermSize::new(20, 2), 100, CursorShape::Block);
        feed(&mut term, "one\r\ntwo\r\nthree\r\nfour");
        assert_eq!(row_text(&term, 0), "three");
        assert_eq!(row_text(&term, 1), "four");
        term.scroll_display(Scroll::Delta(2));
        assert_eq!(term.display_offset(), 2);
        assert_eq!(row_text(&term, 0), "one");
        assert_eq!(row_text(&term, 1), "two");
        term.scroll_to_bottom();
        assert_eq!(term.display_offset(), 0);
    }

    #[test]
    fn selection_roundtrips_text() {
        let mut term = Terminal::new(TermSize::new(20, 4), 100, CursorShape::Block);
        feed(&mut term, "hello\r\nworld");
        let start = term.viewport_point(0, 0).unwrap();
        let end = term.viewport_point(0, 4).unwrap();
        term.selection_start(start, Side::Left);
        term.selection_update(end, Side::Right);
        assert_eq!(term.selection_text().as_deref(), Some("hello"));
        term.clear_selection();
        assert!(!term.has_selection());
    }

    #[test]
    fn alternate_screen_and_modes() {
        let mut term = Terminal::new(TermSize::new(20, 4), 100, CursorShape::Block);
        feed(&mut term, "normal");
        feed(&mut term, "\x1b[?1049h");
        feed(&mut term, "\x1b[?25l");
        assert!(term.mode().contains(TermMode::ALT_SCREEN));
        assert!(!term.mode().contains(TermMode::SHOW_CURSOR));
        feed(&mut term, "\x1b[?1049l");
        assert_eq!(row_text(&term, 0), "normal");
    }

    #[test]
    fn wide_characters_occupy_two_cells() {
        let mut term = Terminal::new(TermSize::new(20, 4), 100, CursorShape::Block);
        feed(&mut term, "你好");
        let cell = term.cell(0, 0).unwrap();
        assert!(cell.flags.contains(Flags::WIDE_CHAR));
        let spacer = term.cell(0, 1).unwrap();
        assert!(spacer.flags.contains(Flags::WIDE_CHAR_SPACER));
    }

    #[test]
    fn resolves_tabby_default_palette() {
        let mut term = Terminal::new(TermSize::new(20, 4), 100, CursorShape::Block);
        feed(&mut term, "\x1b[38;5;196mX\x1b[0m");
        let cell = term.cell(0, 0).unwrap();
        let scheme = TerminalSettings::default().color_scheme;
        let resolved = term.resolve_cell(cell, &scheme, true);
        assert_eq!(resolved.fg, scheme.palette(196));
    }

    #[test]
    fn osc_title_and_bell_events_are_forwarded() {
        let mut term = Terminal::new(TermSize::new(20, 4), 100, CursorShape::Block);
        feed(&mut term, "\x1b]2;my title\x07\x07");
        let events = term.take_events();
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::Title(title) if title == "my title"))
        );
        assert!(events.iter().any(|event| matches!(event, Event::Bell)));
        assert_eq!(term.bell_count(), 1);
    }
}
