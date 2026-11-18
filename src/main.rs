mod android;
mod app;

use gpui_kit::*;

fn main() {
    application()
        .with_assets(assets::Assets)
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(|cx| {
            init(cx);

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(120.), px(80.)),
                    size(px(1180.), px(760.)),
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Android CLI".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };

            open_window(options, cx, |window, cx| {
                cx.new(|cx| app::App::new(window, cx))
            })
            .expect("Failed to open window");
        });
}
