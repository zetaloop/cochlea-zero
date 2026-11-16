#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod analysis;
mod audio;
mod playback;
mod player;
mod visual;

use gpui::{
    AppContext, Bounds, WindowBounds, WindowOptions,
    component::{Theme, TitleBar},
    px, size,
};

fn main() {
    gpui::application()
        .with_assets(gpui::assets::AllAssets)
        .run(|cx| {
            gpui::init(cx);
            gpui::component::set_locale("zh-CN");
            Theme::sync_system_appearance(None, cx);
            player::init(cx);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1120.0), px(760.0)),
                    cx,
                ))),
                window_min_size: Some(size(px(760.0), px(480.0))),
                inactive_frame_interval: None,
                low_power_frame_interval: None,
                thermal_frame_interval: None,
                ..TitleBar::window_options()
            };
            gpui::open_window(options, cx, |window, cx| {
                window.set_window_title("Cochlea Zero");
                cx.new(|cx| player::Player::new(window, cx))
            })
            .expect("failed to open window");
            cx.activate(true);
        });
}
