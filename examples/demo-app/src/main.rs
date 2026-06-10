//! Demo app for gpui-driver: a counter button, a modal dialog (occlusion test target),
//! and a minimal text field (synthetic keyboard test target).
//!
//! Run with `cargo run -p demo-app` (the `driver` feature is on by default here; real
//! apps should keep it opt-in and debug-only).

use gpui::{
    App, Bounds, Context, FocusHandle, KeyDownEvent, TitlebarOptions, Window, WindowBounds,
    WindowOptions, div, point, prelude::*, px, rgb, size,
};
use gpui_platform::application;

#[cfg(feature = "driver")]
use gpui_driver::DriverExt;

/// No-op stand-ins so the same render code compiles without the driver feature.
#[cfg(not(feature = "driver"))]
trait DriverExtStub: Sized {
    fn driver_id(self, _id: &'static str) -> Self {
        self
    }
    fn driver_text(self, _text: String) -> Self {
        self
    }
}
#[cfg(not(feature = "driver"))]
impl<T> DriverExtStub for T {}

struct DemoApp {
    count: usize,
    dialog_open: bool,
    input_text: String,
    input_focus: FocusHandle,
}

impl DemoApp {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            count: 0,
            dialog_open: false,
            input_text: String::new(),
            input_focus: cx.focus_handle(),
        }
    }

    fn on_input_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if event.keystroke.key == "backspace" {
            self.input_text.pop();
            cx.notify();
        } else if let Some(ch) = &event.keystroke.key_char {
            self.input_text.push_str(ch);
            cx.notify();
        }
    }
}

impl Render for DemoApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let button = |id: &'static str, label: String| {
            div()
                .id(id)
                .px_4()
                .py_2()
                .bg(rgb(0x2563eb))
                .text_color(gpui::white())
                .rounded_md()
                .cursor_pointer()
                .hover(|style| style.bg(rgb(0x1d4ed8)))
                .child(label)
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .p_6()
            .bg(rgb(0xf3f4f6))
            .text_color(rgb(0x111827))
            .child(
                div()
                    .text_xl()
                    .child(format!("Count: {}", self.count))
                    .driver_id("counter_label")
                    .driver_text(format!("Count: {}", self.count)),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(
                        button("increment", "Increment".into())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.count += 1;
                                cx.notify();
                            }))
                            .driver_id("increment_button")
                            .driver_text("Increment"),
                    )
                    .child(
                        button("open_dialog", "Open dialog".into())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.dialog_open = true;
                                cx.notify();
                            }))
                            .driver_id("open_dialog_button")
                            .driver_text("Open dialog"),
                    ),
            )
            .child(
                div()
                    .id("name_input")
                    .track_focus(&self.input_focus)
                    .on_key_down(cx.listener(|this, event, _, cx| {
                        this.on_input_key(event, cx);
                    }))
                    .w_80()
                    .px_3()
                    .py_2()
                    .bg(gpui::white())
                    .border_1()
                    .border_color(rgb(0x9ca3af))
                    .rounded_md()
                    .child(if self.input_text.is_empty() {
                        "(click and type)".to_string()
                    } else {
                        self.input_text.clone()
                    })
                    .driver_id("name_input")
                    .driver_text(self.input_text.clone()),
            )
            .when(self.dialog_open, |el| {
                el.child(
                    div()
                        .id("dialog_overlay")
                        .occlude()
                        .absolute()
                        .inset_0()
                        .bg(gpui::rgba(0x00000080))
                        .flex()
                        .justify_center()
                        .items_center()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_3()
                                .p_6()
                                .bg(gpui::white())
                                .rounded_lg()
                                .child("A modal dialog. It blocks the buttons behind it.")
                                .child(
                                    button("close_dialog", "Close".into())
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.dialog_open = false;
                                            cx.notify();
                                        }))
                                        .driver_id("close_dialog_button")
                                        .driver_text("Close"),
                                )
                                .driver_id("dialog"),
                        ),
                )
            })
    }
}

fn main() {
    application().run(|cx: &mut App| {
        #[cfg(feature = "driver")]
        gpui_driver::init_with_options(
            cx,
            gpui_driver::DriverOptions {
                app_name: Some("demo-app".into()),
                app_version: Some(env!("CARGO_PKG_VERSION").into()),
            },
        );

        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(300.0), px(300.0)),
                    size: size(px(640.0), px(420.0)),
                })),
                titlebar: Some(TitlebarOptions {
                    title: Some("gpui-driver demo".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                window.set_window_title("gpui-driver demo");
                cx.new(DemoApp::new)
            },
        )
        .expect("open window");
        cx.activate(true);
    });
}
