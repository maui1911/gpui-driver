//! Spike A: can we read back rendered frames from a GPUI window on Windows while the
//! window is (a) focused, (b) occluded, (c) minimized, (d) the session is locked?
//!
//! Renders a window with a ticking counter and captures `window.render_to_image()`
//! once per second into `tmp-shots/`, switching scenarios on a timeline. Each PNG is
//! named after the scenario and tick so freshness is verifiable by eye.
//!
//! Run with `SPIKE_LOCK=1` to also lock the workstation mid-run (scenario d). The
//! session must be unlocked manually afterwards.

use std::time::{Duration, Instant};

use gpui::{
    App, AppContext, Bounds, Context, Window, WindowBounds, WindowOptions, div, point, prelude::*,
    px, rgb, size,
};
use gpui_platform::application;

struct Ticker {
    started: Instant,
    ticks: u64,
}

impl Render for Ticker {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .bg(rgb(0x202060))
            .size_full()
            .justify_center()
            .items_center()
            .text_xl()
            .text_color(rgb(0xffffff))
            .child(format!("tick {}", self.ticks))
            .child(format!(
                "elapsed {:.1}s",
                self.started.elapsed().as_secs_f32()
            ))
    }
}

struct Occluder;

impl Render for Occluder {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(0x802020))
            .flex()
            .justify_center()
            .items_center()
            .text_color(rgb(0xffffff))
            .child("OCCLUDER — this window covers the target")
    }
}

fn scenario(elapsed: Duration, lock_mode: bool) -> &'static str {
    let s = elapsed.as_secs();
    if lock_mode {
        match s {
            0..=4 => "focused",
            5..=29 => "locked",
            _ => "unlocked-tail",
        }
    } else {
        match s {
            0..=4 => "focused",
            5..=9 => "occluded",
            10..=14 => "minimized",
            _ => "restored",
        }
    }
}

fn main() {
    let lock_mode = std::env::var("SPIKE_LOCK").is_ok_and(|v| v == "1");
    std::fs::create_dir_all("tmp-shots").expect("create tmp-shots");

    application().run(move |cx: &mut App| {
        let target_bounds = Bounds {
            origin: point(px(200.0), px(200.0)),
            size: size(px(500.0), px(300.0)),
        };
        let target = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(target_bounds)),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|_| Ticker {
                        started: Instant::now(),
                        ticks: 0,
                    })
                },
            )
            .expect("open target window");
        cx.activate(true);

        // Tick the counter at 10 Hz so every capture shows fresh content.
        cx.spawn({
            async move |cx| {
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(100))
                        .await;
                    let result = target.update(cx, |ticker, _, cx| {
                        ticker.ticks += 1;
                        cx.notify();
                    });
                    if result.is_err() {
                        return; // window closed
                    }
                }
            }
        })
        .detach();

        // Scenario timeline + capture loop, 1 capture/second.
        cx.spawn(async move |cx| {
            let started = Instant::now();
            let total = if lock_mode { 35 } else { 18 };
            let mut occluder = None;

            for tick in 0..total {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let elapsed = started.elapsed();
                let name = scenario(elapsed, lock_mode);

                // Scenario transitions.
                if !lock_mode {
                    if elapsed.as_secs() == 5 && occluder.is_none() {
                        occluder = cx
                            .open_window(
                                WindowOptions {
                                    window_bounds: Some(WindowBounds::Windowed(Bounds {
                                        origin: point(px(150.0), px(150.0)),
                                        size: size(px(700.0), px(500.0)),
                                    })),
                                    ..Default::default()
                                },
                                |_, cx| cx.new(|_| Occluder),
                            )
                            .ok();
                        println!("[spike] opened occluder window");
                    }
                    if elapsed.as_secs() == 10 {
                        let _ = target.update(cx, |_, window, _| window.minimize_window());
                        println!("[spike] minimized target window");
                    }
                    if elapsed.as_secs() == 15 {
                        let _ = target.update(cx, |_, window, cx| {
                            window.activate_window();
                            cx.notify();
                        });
                        println!("[spike] restored target window");
                    }
                } else if elapsed.as_secs() == 5 {
                    println!("[spike] locking workstation NOW");
                    let _ = std::process::Command::new("rundll32")
                        .args(["user32.dll,LockWorkStation"])
                        .spawn();
                }

                // Capture.
                let result = target.update(cx, |_, window, _| window.render_to_image());
                match result {
                    Ok(Ok(img)) => {
                        let path = format!("tmp-shots/shot_{tick:02}_{name}.png");
                        match img.save(&path) {
                            Ok(()) => println!(
                                "[spike] {name} t={:.1}s captured {}x{} -> {path}",
                                elapsed.as_secs_f32(),
                                img.width(),
                                img.height()
                            ),
                            Err(e) => println!("[spike] {name} SAVE FAILED: {e}"),
                        }
                    }
                    Ok(Err(e)) => println!("[spike] {name} CAPTURE FAILED: {e:#}"),
                    Err(e) => println!("[spike] {name} WINDOW GONE: {e:#}"),
                }
            }

            println!("[spike] done");
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
}
