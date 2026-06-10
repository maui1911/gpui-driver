//! Foreground handler loop: receives jobs from the TCP server threads and executes
//! them on the GPUI main thread via `AsyncApp`.

use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    AnyWindowHandle, AsyncApp, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PlatformInput, Window,
};
use gpui_driver_protocol as proto;
use gpui_driver_protocol::{ErrorKind, RpcRequest, RpcResponse};
use serde::de::DeserializeOwned;
use serde_json::json;

use crate::element::convert_bounds;
use crate::registry;
use crate::server::Job;

pub(crate) struct DriverMeta {
    pub app_name: String,
    pub app_version: String,
}

type HandlerResult = Result<serde_json::Value, (ErrorKind, String)>;

pub(crate) fn spawn(cx: &mut gpui::App, meta: DriverMeta, mut rx: mpsc::UnboundedReceiver<Job>) {
    cx.spawn(async move |cx| {
        while let Some((req, reply)) = rx.next().await {
            let response = dispatch(&meta, cx, req).await;
            let _ = reply.send(response);
        }
    })
    .detach();
}

async fn dispatch(meta: &DriverMeta, cx: &mut AsyncApp, req: RpcRequest) -> RpcResponse {
    let id = req.id;
    let result = match req.method.as_str() {
        "info" => handle_info(meta),
        "windows" => handle_windows(cx),
        "tree" => params(req.params).and_then(|p| handle_tree(cx, p)),
        "click" => params(req.params).and_then(|p| handle_click(cx, p)),
        "screenshot" => params(req.params).and_then(|p| handle_screenshot(cx, p)),
        "wait_idle" => match params(req.params) {
            Ok(p) => handle_wait_idle(cx, p).await,
            Err(e) => Err(e),
        },
        "type_text" => params(req.params).and_then(|p| handle_type_text(cx, p)),
        "key" => params(req.params).and_then(|p| handle_key(cx, p)),
        "scroll" => params(req.params).and_then(|p| handle_scroll(cx, p)),
        "focus" => params(req.params).and_then(|p| handle_focus(cx, p)),
        "query" => params(req.params).and_then(|p| handle_query(cx, p)),
        other => Err((ErrorKind::Unsupported, format!("unknown method: {other}"))),
    };
    match result {
        Ok(value) => RpcResponse::success(id, value),
        Err((kind, message)) => RpcResponse::error(id, kind, message),
    }
}

fn params<T: DeserializeOwned>(value: serde_json::Value) -> Result<T, (ErrorKind, String)> {
    serde_json::from_value(value).map_err(|e| (ErrorKind::Internal, format!("invalid params: {e}")))
}

fn internal<E: std::fmt::Display>(e: E) -> (ErrorKind, String) {
    (ErrorKind::Internal, e.to_string())
}

fn handle_info(meta: &DriverMeta) -> HandlerResult {
    Ok(json!(proto::InfoResult {
        app_name: meta.app_name.clone(),
        app_version: meta.app_version.clone(),
        protocol_version: proto::PROTOCOL_VERSION,
        gpui_driver_version: env!("CARGO_PKG_VERSION").to_string(),
    }))
}

fn handle_windows(cx: &mut AsyncApp) -> HandlerResult {
    let handles = cx.update(|cx| cx.windows());
    let mut windows = Vec::new();
    for handle in handles {
        let info = handle.update(cx, |_, window, _| proto::WindowInfo {
            window_id: handle.window_id().as_u64(),
            title: window.window_title(),
            bounds: convert_bounds(window.bounds()),
            active: window.is_window_active(),
        });
        if let Ok(info) = info {
            windows.push(info);
        }
    }
    Ok(json!(proto::WindowsResult { windows }))
}

fn handle_tree(cx: &mut AsyncApp, p: proto::TreeParams) -> HandlerResult {
    let handle = resolve_window(cx, p.window_id)?;
    let (records, viewport) = collect_fresh(cx, handle)?;
    let tree = registry::assemble_tree(&records, viewport);
    Ok(json!(proto::TreeResult { tree }))
}

/// `window_id == 0` selects the first (usually only) window; anything else must match
/// a `window_id` as returned by the `windows` method.
pub(crate) fn resolve_window(
    cx: &mut AsyncApp,
    window_id: u64,
) -> Result<AnyWindowHandle, (ErrorKind, String)> {
    let handles = cx.update(|cx| cx.windows());
    let found = if window_id == 0 {
        handles.into_iter().next()
    } else {
        handles
            .into_iter()
            .find(|h| h.window_id().as_u64() == window_id)
    };
    found.ok_or_else(|| {
        (
            ErrorKind::WindowNotFound,
            format!("no window with id {window_id}"),
        )
    })
}

/// Forces a fresh, cache-bypassing draw while the registry collects, so the returned
/// records reflect the UI as of *now* — even if the platform frame loop is parked
/// (minimized window) or the app was idle.
pub(crate) fn collect_fresh(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
) -> Result<(Vec<registry::NodeRecord>, proto::Bounds), (ErrorKind, String)> {
    handle
        .update(cx, |_, window, cx| {
            let window_id = window.window_handle().window_id().as_u64();
            registry::global().begin_collect(window_id);
            window.refresh();
            window.draw(cx).clear();
            let records = registry::global().end_collect(window_id);
            let size = window.viewport_size();
            let viewport = proto::Bounds {
                x: 0.0,
                y: 0.0,
                w: f32::from(size.width),
                h: f32::from(size.height),
            };
            (records, viewport)
        })
        .map_err(internal)
}

/// Window-relative center point of a record's bounds.
pub(crate) fn center_of(bounds: proto::Bounds) -> gpui::Point<gpui::Pixels> {
    gpui::point(
        gpui::px(bounds.x + bounds.w / 2.0),
        gpui::px(bounds.y + bounds.h / 2.0),
    )
}

pub(crate) fn find_record<'a>(
    records: &'a [registry::NodeRecord],
    id: &str,
) -> Option<&'a registry::NodeRecord> {
    // Last match wins: if an id is (incorrectly) duplicated, prefer the one drawn last,
    // i.e. topmost.
    records.iter().rev().find(|r| r.id == id)
}

fn convert_button(button: proto::MouseButton) -> MouseButton {
    match button {
        proto::MouseButton::Left => MouseButton::Left,
        proto::MouseButton::Right => MouseButton::Right,
        proto::MouseButton::Middle => MouseButton::Middle,
    }
}

fn convert_modifiers(modifiers: &[proto::Modifier]) -> Modifiers {
    let mut result = Modifiers::default();
    for modifier in modifiers {
        match modifier {
            proto::Modifier::Ctrl => result.control = true,
            proto::Modifier::Alt => result.alt = true,
            proto::Modifier::Shift => result.shift = true,
            proto::Modifier::Cmd => result.platform = true,
            proto::Modifier::Fn => result.function = true,
        }
    }
    result
}

fn handle_click(cx: &mut AsyncApp, p: proto::ClickParams) -> HandlerResult {
    let handle = resolve_window(cx, p.window_id)?;
    let (records, _viewport) = collect_fresh(cx, handle)?;
    let record = find_record(&records, &p.id)
        .ok_or_else(|| {
            (
                ErrorKind::ElementNotFound,
                format!("no element with driver id {:?}", p.id),
            )
        })?
        .clone();

    if record.bounds.w <= 0.0 || record.bounds.h <= 0.0 {
        return Err((
            ErrorKind::ElementNotVisible,
            format!("element {:?} has zero size", p.id),
        ));
    }

    let position = center_of(record.bounds);
    let button = convert_button(p.button);
    let modifiers = convert_modifiers(&p.modifiers);
    let hitbox = record.hitbox.clone();

    let clicked = handle
        .update(cx, |_, window, cx| {
            // Move first so GPUI's hit test reflects the click position.
            window.dispatch_event(
                PlatformInput::MouseMove(MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers,
                }),
                cx,
            );

            // Occlusion check through the real hit-test path: if a blocking overlay
            // (modal, menu) covers the element's center, its hitbox won't be hovered.
            if let Some(hitbox) = &hitbox
                && !hitbox.is_hovered(window)
            {
                return Err((
                    ErrorKind::ElementNotVisible,
                    format!(
                        "element {:?} is not hit-testable at its center ({}, {}) — occluded?",
                        p.id,
                        f32::from(position.x),
                        f32::from(position.y)
                    ),
                ));
            }

            window.dispatch_event(
                PlatformInput::MouseDown(MouseDownEvent {
                    button,
                    position,
                    modifiers,
                    click_count: 1,
                    first_mouse: false,
                }),
                cx,
            );
            window.dispatch_event(
                PlatformInput::MouseUp(MouseUpEvent {
                    button,
                    position,
                    modifiers,
                    click_count: 1,
                }),
                cx,
            );
            Ok(())
        })
        .map_err(internal)?;
    clicked?;

    Ok(json!(proto::ClickResult {
        clicked: true,
        resolved_bounds: record.bounds,
    }))
}

/// Forces a fresh draw and captures the rendered scene, reporting how: `"renderer"`
/// (offscreen readback via the vendored gpui_windows patch; reliable while occluded,
/// minimized or locked) or `"printwindow"` (Win32 best-effort fallback on stock
/// gpui_windows). Fails with `unsupported` only when neither path is available
/// (see docs/spike-a.md).
///
/// `GPUI_DRIVER_FORCE_PRINTWINDOW=1` skips the renderer path, so the fallback can be
/// exercised from a patched build.
fn capture_image(
    window: &mut Window,
    cx: &mut gpui::App,
) -> Result<(image::RgbaImage, &'static str), (ErrorKind, String)> {
    window.refresh();
    window.draw(cx).clear();

    let forced = std::env::var("GPUI_DRIVER_FORCE_PRINTWINDOW").is_ok_and(|v| v == "1");
    let renderer_err = if forced {
        "renderer capture skipped (GPUI_DRIVER_FORCE_PRINTWINDOW=1)".to_string()
    } else {
        match window.render_to_image() {
            Ok(image) => return Ok((image, "renderer")),
            Err(e) => format!("{e:#}"),
        }
    };

    // Renderer errors other than "not implemented" mean the patched path exists but
    // broke; surface those instead of papering over them with a fallback capture.
    if !forced && !renderer_err.contains("not implemented") {
        return Err((
            ErrorKind::Internal,
            format!("screenshot failed: {renderer_err}"),
        ));
    }

    match capture_printwindow(window) {
        Ok(image) => Ok((image, "printwindow")),
        Err(fallback_err) => Err((
            ErrorKind::Unsupported,
            format!("screenshot failed: {renderer_err}; PrintWindow fallback: {fallback_err:#}"),
        )),
    }
}

/// Best-effort capture without the vendored patch, via the window's HWND (reachable
/// through gpui's public `HasWindowHandle` impl).
#[cfg(target_os = "windows")]
fn capture_printwindow(window: &Window) -> anyhow::Result<image::RgbaImage> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    // Fully qualified: gpui's inherent `Window::window_handle` (returning its own
    // AnyWindowHandle) shadows the raw-window-handle trait method.
    let handle = HasWindowHandle::window_handle(window)
        .map_err(|e| anyhow::anyhow!("window_handle: {e}"))?;
    match handle.as_raw() {
        RawWindowHandle::Win32(handle) => crate::printwindow::capture_hwnd(handle.hwnd.get()),
        other => anyhow::bail!("unexpected window handle kind: {other:?}"),
    }
}

#[cfg(not(target_os = "windows"))]
fn capture_printwindow(_window: &Window) -> anyhow::Result<image::RgbaImage> {
    anyhow::bail!("no fallback capture on this platform")
}

fn handle_screenshot(cx: &mut AsyncApp, p: proto::ScreenshotParams) -> HandlerResult {
    let handle = resolve_window(cx, p.window_id)?;
    let (image, method, scale) = handle
        .update(cx, |_, window, cx| {
            let (image, method) = capture_image(window, cx)?;
            Ok((image, method, window.scale_factor()))
        })
        .map_err(internal)??;

    let (width, height) = (image.width(), image.height());
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(image)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(internal)?;

    use base64::Engine as _;
    Ok(json!(proto::ScreenshotResult {
        format: "png".into(),
        data_base64: base64::engine::general_purpose::STANDARD.encode(&png),
        width,
        height,
        scale,
        method: method.to_string(),
    }))
}

/// Maps a plain character to GPUI keystroke syntax (`' '` is named `space`, etc.).
fn keystroke_for_char(ch: char) -> String {
    match ch {
        ' ' => "space".to_string(),
        '\n' => "enter".to_string(),
        '\t' => "tab".to_string(),
        '-' => "minus".to_string(),
        other => other.to_string(),
    }
}

fn handle_type_text(cx: &mut AsyncApp, p: proto::TypeTextParams) -> HandlerResult {
    let handle = resolve_window(cx, p.window_id)?;
    handle
        .update(cx, |_, window, cx| {
            for ch in p.text.chars() {
                // Mirrors gpui's TestAppContext::simulate_input: parse each character
                // as a keystroke; dispatch_keystroke simulates IME so key_char is set.
                match gpui::Keystroke::parse(&keystroke_for_char(ch)) {
                    Ok(keystroke) => {
                        window.dispatch_keystroke(keystroke, cx);
                    }
                    Err(_) => {
                        log::debug!("gpui-driver: skipping untypeable character {ch:?}");
                    }
                }
            }
        })
        .map_err(internal)?;
    Ok(json!(proto::TypeTextResult { typed: true }))
}

fn handle_key(cx: &mut AsyncApp, p: proto::KeyParams) -> HandlerResult {
    let keystroke = gpui::Keystroke::parse(&p.combo).map_err(|_| {
        (
            ErrorKind::Internal,
            format!("invalid keystroke combo: {:?}", p.combo),
        )
    })?;
    let handle = resolve_window(cx, p.window_id)?;
    let dispatched = handle
        .update(cx, |_, window, cx| window.dispatch_keystroke(keystroke, cx))
        .map_err(internal)?;
    Ok(json!(proto::KeyResult { dispatched }))
}

fn handle_scroll(cx: &mut AsyncApp, p: proto::ScrollParams) -> HandlerResult {
    let handle = resolve_window(cx, p.window_id)?;
    let (records, _) = collect_fresh(cx, handle)?;
    let record = find_record(&records, &p.id)
        .ok_or_else(|| {
            (
                ErrorKind::ElementNotFound,
                format!("no element with driver id {:?}", p.id),
            )
        })?
        .clone();
    let position = center_of(record.bounds);

    handle
        .update(cx, |_, window, cx| {
            window.dispatch_event(
                PlatformInput::MouseMove(MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers: Modifiers::default(),
                }),
                cx,
            );
            window.dispatch_event(
                PlatformInput::ScrollWheel(gpui::ScrollWheelEvent {
                    position,
                    delta: gpui::ScrollDelta::Pixels(gpui::point(
                        gpui::px(p.delta_x),
                        gpui::px(p.delta_y),
                    )),
                    modifiers: Modifiers::default(),
                    touch_phase: gpui::TouchPhase::Moved,
                }),
                cx,
            );
        })
        .map_err(internal)?;
    Ok(json!(proto::ScrollResult { scrolled: true }))
}

/// v0 semantics: focus by synthesizing a left click on the element's center, which is
/// how a user would focus it. A future version may use focus handles directly.
fn handle_focus(cx: &mut AsyncApp, p: proto::FocusParams) -> HandlerResult {
    let click = proto::ClickParams {
        token: p.token,
        window_id: p.window_id,
        id: p.id,
        button: proto::MouseButton::Left,
        modifiers: Vec::new(),
    };
    handle_click(cx, click)?;
    Ok(json!(proto::FocusResult { focused: true }))
}

fn handle_query(cx: &mut AsyncApp, p: proto::QueryParams) -> HandlerResult {
    if p.text_contains.is_none() && p.id_contains.is_none() {
        return Err((
            ErrorKind::Internal,
            "query needs text_contains and/or id_contains".into(),
        ));
    }
    let handle = resolve_window(cx, p.window_id)?;
    let (records, viewport) = collect_fresh(cx, handle)?;
    let text_needle = p.text_contains.as_deref().map(str::to_lowercase);
    let id_needle = p.id_contains.as_deref().map(str::to_lowercase);

    let matches: Vec<proto::TreeNode> = records
        .iter()
        .filter(|r| {
            let text_ok = text_needle.as_deref().is_none_or(|needle| {
                r.text
                    .as_deref()
                    .is_some_and(|t| t.to_lowercase().contains(needle))
            });
            let id_ok = id_needle
                .as_deref()
                .is_none_or(|needle| r.id.to_lowercase().contains(needle));
            text_ok && id_ok
        })
        .map(|r| proto::TreeNode {
            id: Some(r.id.clone()),
            kind: r.kind.clone(),
            text: r.text.clone(),
            bounds: r.bounds,
            visible: r.bounds.w > 0.0
                && r.bounds.h > 0.0
                && r.bounds.x < viewport.w
                && r.bounds.y < viewport.h,
            enabled: true,
            focused: false,
            interactive: r.interactive,
            children: Vec::new(),
        })
        .collect();

    Ok(json!(proto::QueryResult { matches }))
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Samples a fingerprint of the window's current visual state. Prefers rendered
/// pixels; falls back to the registry (ids + bounds + text) where capture is
/// unsupported.
fn sample_window_state(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
) -> Result<u64, (ErrorKind, String)> {
    let pixel_hash = handle
        .update(cx, |_, window, cx| {
            window.refresh();
            window.draw(cx).clear();
            window.render_to_image().ok().map(|img| fnv1a(img.as_raw()))
        })
        .map_err(internal)?;
    if let Some(hash) = pixel_hash {
        return Ok(hash);
    }

    let (records, _) = collect_fresh(cx, handle)?;
    let mut repr = String::new();
    for r in &records {
        use std::fmt::Write as _;
        let _ = write!(
            repr,
            "{}|{}|{:?}|{:.1},{:.1},{:.1},{:.1};",
            r.id, r.kind, r.text, r.bounds.x, r.bounds.y, r.bounds.w, r.bounds.h
        );
    }
    Ok(fnv1a(repr.as_bytes()))
}

/// Resolves once the window's visual state has stopped changing for `quiet_ms`.
///
/// The design phrases idleness as "no frame requested for quiet_ms"; GPUI doesn't
/// expose its dirty flag publicly, so we measure the equivalent observable: the
/// rendered output no longer changes between forced draws.
async fn handle_wait_idle(cx: &mut AsyncApp, p: proto::WaitIdleParams) -> HandlerResult {
    let handle = resolve_window(cx, p.window_id)?;
    let start = Instant::now();
    let timeout = Duration::from_millis(p.timeout_ms);
    let quiet = Duration::from_millis(p.quiet_ms);
    let poll = Duration::from_millis((p.quiet_ms / 3).clamp(15, 50));

    let mut last_hash = sample_window_state(cx, handle)?;
    let mut stable_since = Instant::now();

    loop {
        if stable_since.elapsed() >= quiet {
            return Ok(json!(proto::WaitIdleResult {
                waited_ms: start.elapsed().as_millis() as u64,
            }));
        }
        if start.elapsed() >= timeout {
            return Err((
                ErrorKind::Timeout,
                format!(
                    "window did not go idle within {} ms (quiet window: {} ms)",
                    p.timeout_ms, p.quiet_ms
                ),
            ));
        }
        cx.background_executor().timer(poll).await;
        let hash = sample_window_state(cx, handle)?;
        if hash != last_hash {
            last_hash = hash;
            stable_since = Instant::now();
        }
    }
}
