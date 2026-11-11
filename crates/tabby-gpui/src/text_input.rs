//! A small single-line text field.
//!
//! GPUI does not ship a text input; Tabby's dialogs need editable host, user,
//! port and search fields, so this implements the minimum: caret movement,
//! selection-free editing, paste, masking for passwords and a placeholder.
//! Layout is deliberately simple (one line, no scrolling) because every field
//! in the UI is short.

use gpui::{
    Context, FocusHandle, KeyDownEvent, MouseDownEvent, Render, SharedString, Window, div,
    prelude::*, px,
};

use crate::theme::Theme;

pub struct TextInput {
    focus_handle: FocusHandle,
    value: SharedString,
    placeholder: SharedString,
    secret: bool,
    /// Byte offset in `value`.
    caret: usize,
    theme: Theme,
}

impl TextInput {
    pub fn new(
        placeholder: impl Into<SharedString>,
        secret: bool,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            value: "".into(),
            placeholder: placeholder.into(),
            secret,
            caret: 0,
            theme,
        }
    }

    pub fn set_theme(&mut self, theme: Theme, cx: &mut Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    fn insert(&mut self, text: &str, cx: &mut Context<Self>) {
        let mut value = self.value.to_string();
        value.insert_str(self.caret, text);
        self.caret += text.len();
        self.value = value.into();
        cx.notify();
    }

    fn backspace(&mut self, cx: &mut Context<Self>) {
        if self.caret == 0 {
            return;
        }
        let mut value = self.value.to_string();
        let previous = value[..self.caret]
            .char_indices()
            .next_back()
            .map(|(index, _)| index)
            .unwrap_or(0);
        value.replace_range(previous..self.caret, "");
        self.caret = previous;
        self.value = value.into();
        cx.notify();
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        if self.caret >= self.value.len() {
            return;
        }
        let mut value = self.value.to_string();
        let next = value[self.caret..]
            .char_indices()
            .nth(1)
            .map(|(index, _)| self.caret + index)
            .unwrap_or(value.len());
        value.replace_range(self.caret..next, "");
        self.value = value.into();
        cx.notify();
    }

    fn move_left(&mut self, cx: &mut Context<Self>) {
        if self.caret == 0 {
            return;
        }
        self.caret = self.value[..self.caret]
            .char_indices()
            .next_back()
            .map(|(index, _)| index)
            .unwrap_or(0);
        cx.notify();
    }

    fn move_right(&mut self, cx: &mut Context<Self>) {
        if self.caret >= self.value.len() {
            return;
        }
        self.caret = self.value[self.caret..]
            .char_indices()
            .nth(1)
            .map(|(index, _)| self.caret + index)
            .unwrap_or(self.value.len());
        cx.notify();
    }

    fn handle_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;

        // Clipboard stays available even though there is no selection model.
        if modifiers.control && keystroke.key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                self.insert(text.trim_end_matches(['\r', '\n']), cx);
            }
            return;
        }
        if modifiers.control {
            return;
        }

        match keystroke.key.as_str() {
            "backspace" => self.backspace(cx),
            "delete" => self.delete(cx),
            "left" => self.move_left(cx),
            "right" => self.move_right(cx),
            "home" => {
                self.caret = 0;
                cx.notify();
            }
            "end" => {
                self.caret = self.value.len();
                cx.notify();
            }
            "enter" | "tab" | "escape" | "up" | "down" | "shift-tab" => {}
            _ => {
                if let Some(text) = &keystroke.key_char {
                    if !text.is_empty() && !modifiers.alt {
                        self.insert(text, cx);
                    }
                }
            }
        }
    }

    fn handle_mouse_down(
        &mut self,
        _event: &MouseDownEvent,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window);
    }

    fn display_text(&self) -> String {
        if self.secret {
            "\u{2022}".repeat(self.value.chars().count())
        } else {
            self.value.to_string()
        }
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.theme;
        let focused = self.focus_handle.is_focused(window);
        let empty = self.value.is_empty();
        let text = if empty {
            self.placeholder.to_string()
        } else {
            self.display_text()
        };
        let key_listener = cx
            .listener(|this, event: &KeyDownEvent, window, cx| this.handle_key(event, window, cx));
        let mouse_listener = cx.listener(|this, event: &MouseDownEvent, window, cx| {
            this.handle_mouse_down(event, window, cx)
        });
        let focus_handle = self.focus_handle.clone();

        div()
            .flex()
            .items_center()
            .h(px(28.0))
            .px_2()
            .rounded(px(3.0))
            .bg(if focused {
                theme.surface_active
            } else {
                theme.surface
            })
            .border_1()
            .border_color(if focused { theme.accent } else { theme.border })
            .text_sm()
            .text_color(if empty { theme.muted } else { theme.foreground })
            .track_focus(&focus_handle)
            .on_key_down(key_listener)
            .on_mouse_down(gpui::MouseButton::Left, mouse_listener)
            .child(text)
            .when(focused, |this| {
                this.child(
                    div()
                        .w(px(1.0))
                        .h(px(14.0))
                        .ml(px(1.0))
                        .bg(theme.foreground),
                )
            })
            .child(div().flex_1())
    }
}
