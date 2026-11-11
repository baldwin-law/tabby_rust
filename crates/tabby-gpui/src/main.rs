#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod pane;
mod text_input;
mod theme;

use gpui::{
    App, AppContext as _, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px,
    size,
};

use tabby_core::config::Config;

use crate::app::{TabbyApp, register_key_bindings};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    Application::new().run(|cx: &mut App| {
        register_key_bindings(cx);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let config = Config::load_or_default();
        let bounds = Bounds::centered(None, size(px(1100.0), px(700.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(420.0), px(260.0))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Tabby Rust".into()),
                    ..Default::default()
                }),
                app_id: Some("tabby-rust".into()),
                ..Default::default()
            },
            |window, cx| {
                let app = cx.new(|cx| TabbyApp::empty(config, cx));
                let _ = app.update(cx, |app: &mut TabbyApp, cx| app.start(window, cx));
                app
            },
        )
        .expect("could not open the Tabby Rust window");
        cx.activate(true);
    });
}
