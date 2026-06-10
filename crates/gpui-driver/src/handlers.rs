//! Foreground handler loop: receives jobs from the TCP server threads and executes
//! them on the GPUI main thread via `AsyncApp`.

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{AnyWindowHandle, AsyncApp};
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
        other => Err((
            ErrorKind::Unsupported,
            format!("unknown method: {other}"),
        )),
    };
    match result {
        Ok(value) => RpcResponse::success(id, value),
        Err((kind, message)) => RpcResponse::error(id, kind, message),
    }
}

fn params<T: DeserializeOwned>(value: serde_json::Value) -> Result<T, (ErrorKind, String)> {
    serde_json::from_value(value)
        .map_err(|e| (ErrorKind::Internal, format!("invalid params: {e}")))
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

#[allow(dead_code)]
pub(crate) fn find_record<'a>(
    records: &'a [registry::NodeRecord],
    id: &str,
) -> Option<&'a registry::NodeRecord> {
    // Last match wins: if an id is (incorrectly) duplicated, prefer the one drawn last,
    // i.e. topmost.
    records.iter().rev().find(|r| r.id == id)
}
