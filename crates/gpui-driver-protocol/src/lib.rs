//! Shared request/response types for the gpui-driver JSON-RPC protocol.
//!
//! The wire format is newline-delimited JSON-RPC 2.0: one request or response
//! object per line over a localhost TCP connection. Every request carries an
//! auth `token` (from the discovery file) inside its `params`.

use serde::{Deserialize, Serialize};

/// Version of the RPC protocol described by this crate.
pub const PROTOCOL_VERSION: u32 = 1;

/// Directory under the OS temp dir where discovery files are written.
pub const DISCOVERY_DIR_NAME: &str = "gpui-driver";

/// A single JSON-RPC 2.0 request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

impl RpcRequest {
    pub fn new(id: u64, method: impl Into<String>, params: impl Serialize) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            method: method.into(),
            params: serde_json::to_value(params).expect("params must serialize"),
        }
    }
}

/// A single JSON-RPC 2.0 response. Exactly one of `result`/`error` is set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcResponse {
    pub jsonrpc: String,
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl RpcResponse {
    pub fn success(id: u64, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: u64, kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(RpcError {
                code: kind.code(),
                message: message.into(),
                data: Some(RpcErrorData { kind }),
            }),
        }
    }
}

/// JSON-RPC error object with structured `data.kind`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<RpcErrorData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcErrorData {
    pub kind: ErrorKind,
}

/// Machine-readable failure categories (design §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    ElementNotFound,
    ElementNotVisible,
    WindowNotFound,
    Timeout,
    Unsupported,
    AuthFailed,
    Internal,
}

impl ErrorKind {
    /// JSON-RPC `error.code`. Application-defined range.
    pub fn code(self) -> i64 {
        match self {
            ErrorKind::ElementNotFound => -32000,
            ErrorKind::ElementNotVisible => -32001,
            ErrorKind::WindowNotFound => -32002,
            ErrorKind::Timeout => -32003,
            ErrorKind::Unsupported => -32004,
            ErrorKind::AuthFailed => -32005,
            ErrorKind::Internal => -32099,
        }
    }
}

/// Discovery file written to `<temp>/gpui-driver/<pid>.json` on init.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryFile {
    pub app_name: String,
    pub pid: u32,
    pub port: u16,
    pub token: String,
    pub protocol_version: u32,
    pub started_at: String,
}

/// Pixel-space rectangle. Logical pixels, window-relative unless stated otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

// ---- info ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfoParams {
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfoResult {
    pub app_name: String,
    pub app_version: String,
    pub protocol_version: u32,
    pub gpui_driver_version: String,
}

// ---- windows ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowsParams {
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowsResult {
    pub windows: Vec<WindowInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowInfo {
    pub window_id: u64,
    pub title: String,
    /// Screen coordinates.
    pub bounds: Bounds,
    pub active: bool,
}

// ---- tree ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeParams {
    pub token: String,
    #[serde(default)]
    pub window_id: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeResult {
    pub tree: TreeNode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeNode {
    /// `driver_id` if set; `None` for non-addressable structural nodes.
    pub id: Option<String>,
    pub kind: String,
    /// Visible text content, best effort.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub bounds: Bounds,
    pub visible: bool,
    pub enabled: bool,
    pub focused: bool,
    /// Whether the element has click/key handlers.
    pub interactive: bool,
    #[serde(default)]
    pub children: Vec<TreeNode>,
}

// ---- click ----

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    #[default]
    Left,
    Right,
    Middle,
}

/// Keyboard modifiers, as accepted in `click.modifiers`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Modifier {
    Ctrl,
    Alt,
    Shift,
    Cmd,
    Fn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClickParams {
    pub token: String,
    #[serde(default)]
    pub window_id: u64,
    pub id: String,
    #[serde(default)]
    pub button: MouseButton,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClickResult {
    pub clicked: bool,
    pub resolved_bounds: Bounds,
}

// ---- screenshot ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenshotParams {
    pub token: String,
    #[serde(default)]
    pub window_id: u64,
}

fn default_screenshot_method() -> String {
    "renderer".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenshotResult {
    pub format: String,
    pub data_base64: String,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    /// How the image was captured: `"renderer"` (offscreen scene readback via the
    /// vendored gpui_windows patch; reliable while occluded/minimized/locked) or
    /// `"printwindow"` (Win32 PrintWindow fallback; best effort, the window should
    /// be visible). Older servers omit the field; it defaults to `"renderer"`.
    #[serde(default = "default_screenshot_method")]
    pub method: String,
}

// ---- wait_idle ----

fn default_timeout_ms() -> u64 {
    5000
}

fn default_quiet_ms() -> u64 {
    150
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaitIdleParams {
    pub token: String,
    #[serde(default)]
    pub window_id: u64,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_quiet_ms")]
    pub quiet_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaitIdleResult {
    /// Milliseconds spent waiting until the window went quiet.
    pub waited_ms: u64,
}

// ---- phase 2: type_text / key / scroll / focus / query ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeTextParams {
    pub token: String,
    #[serde(default)]
    pub window_id: u64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeTextResult {
    pub typed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyParams {
    pub token: String,
    #[serde(default)]
    pub window_id: u64,
    /// GPUI-style keystroke, e.g. `ctrl-s`.
    pub combo: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyResult {
    pub dispatched: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrollParams {
    pub token: String,
    #[serde(default)]
    pub window_id: u64,
    pub id: String,
    #[serde(default)]
    pub delta_x: f32,
    #[serde(default)]
    pub delta_y: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrollResult {
    pub scrolled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FocusParams {
    pub token: String,
    #[serde(default)]
    pub window_id: u64,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FocusResult {
    pub focused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryParams {
    pub token: String,
    #[serde(default)]
    pub window_id: u64,
    #[serde(default)]
    pub text_contains: Option<String>,
    #[serde(default)]
    pub id_contains: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    /// Matching nodes, without children.
    pub matches: Vec<TreeNode>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rpc_request_round_trip() {
        let line = r#"{"jsonrpc":"2.0","id":7,"method":"click","params":{"token":"abc","window_id":0,"id":"save_button","button":"left","modifiers":[]}}"#;
        let req: RpcRequest = serde_json::from_str(line).unwrap();
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.id, 7);
        assert_eq!(req.method, "click");
        let params: ClickParams = serde_json::from_value(req.params.clone()).unwrap();
        assert_eq!(params.token, "abc");
        assert_eq!(params.window_id, 0);
        assert_eq!(params.id, "save_button");
        assert_eq!(params.button, MouseButton::Left);
        assert!(params.modifiers.is_empty());
        let back = serde_json::to_value(&req).unwrap();
        assert_eq!(back["method"], "click");
        assert_eq!(back["params"]["id"], "save_button");
    }

    #[test]
    fn response_success_shape() {
        let resp = RpcResponse::success(3, json!({"clicked": true}));
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["jsonrpc"], "2.0");
        assert_eq!(v["id"], 3);
        assert_eq!(v["result"]["clicked"], true);
        assert!(v.get("error").is_none());
    }

    #[test]
    fn response_error_shape_and_kind() {
        let resp = RpcResponse::error(4, ErrorKind::ElementNotFound, "no such element: foo");
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["id"], 4);
        assert!(v.get("result").is_none());
        assert_eq!(v["error"]["data"]["kind"], "element_not_found");
        assert_eq!(v["error"]["message"], "no such element: foo");
        // and it parses back
        let parsed: RpcResponse = serde_json::from_value(v).unwrap();
        let err = parsed.error.unwrap();
        assert_eq!(err.data.unwrap().kind, ErrorKind::ElementNotFound);
    }

    #[test]
    fn error_kinds_serialize_snake_case() {
        for (kind, expected) in [
            (ErrorKind::ElementNotFound, "element_not_found"),
            (ErrorKind::ElementNotVisible, "element_not_visible"),
            (ErrorKind::WindowNotFound, "window_not_found"),
            (ErrorKind::Timeout, "timeout"),
            (ErrorKind::Unsupported, "unsupported"),
            (ErrorKind::AuthFailed, "auth_failed"),
            (ErrorKind::Internal, "internal"),
        ] {
            assert_eq!(
                serde_json::to_value(kind).unwrap(),
                json!(expected),
                "kind {kind:?}"
            );
        }
    }

    #[test]
    fn tree_node_nested_round_trip() {
        let node = TreeNode {
            id: Some("save_button".into()),
            kind: "div".into(),
            text: Some("Save".into()),
            bounds: Bounds {
                x: 412.0,
                y: 88.0,
                w: 96.0,
                h: 32.0,
            },
            visible: true,
            enabled: true,
            focused: false,
            interactive: true,
            children: vec![TreeNode {
                id: None,
                kind: "text".into(),
                text: Some("Save".into()),
                bounds: Bounds {
                    x: 420.0,
                    y: 92.0,
                    w: 80.0,
                    h: 24.0,
                },
                visible: true,
                enabled: true,
                focused: false,
                interactive: false,
                children: vec![],
            }],
        };
        let v = serde_json::to_value(&node).unwrap();
        assert_eq!(v["bounds"]["w"], 96.0);
        assert_eq!(v["children"][0]["kind"], "text");
        let back: TreeNode = serde_json::from_value(v).unwrap();
        assert_eq!(back.children.len(), 1);
        assert_eq!(back.children[0].text.as_deref(), Some("Save"));
    }

    #[test]
    fn discovery_file_round_trip() {
        let line = r#"{
            "app_name": "codescope",
            "pid": 31337,
            "port": 49213,
            "token": "deadbeef",
            "protocol_version": 1,
            "started_at": "2026-06-10T14:03:11Z"
        }"#;
        let d: DiscoveryFile = serde_json::from_str(line).unwrap();
        assert_eq!(d.app_name, "codescope");
        assert_eq!(d.pid, 31337);
        assert_eq!(d.port, 49213);
        assert_eq!(d.protocol_version, 1);
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["started_at"], "2026-06-10T14:03:11Z");
    }

    #[test]
    fn info_and_windows_results() {
        let info = InfoResult {
            app_name: "demo".into(),
            app_version: "1.0.0".into(),
            protocol_version: PROTOCOL_VERSION,
            gpui_driver_version: "0.1.0".into(),
        };
        let v = serde_json::to_value(&info).unwrap();
        assert_eq!(v["protocol_version"], 1);

        let w = WindowsResult {
            windows: vec![WindowInfo {
                window_id: 0,
                title: "CodeScope".into(),
                bounds: Bounds {
                    x: 0.0,
                    y: 0.0,
                    w: 1280.0,
                    h: 800.0,
                },
                active: true,
            }],
        };
        let v = serde_json::to_value(&w).unwrap();
        assert_eq!(v["windows"][0]["window_id"], 0);
        assert_eq!(v["windows"][0]["active"], true);
    }

    #[test]
    fn screenshot_and_wait_idle() {
        let p: ScreenshotParams =
            serde_json::from_value(json!({"token":"t","window_id":0})).unwrap();
        assert_eq!(p.window_id, 0);
        let r = ScreenshotResult {
            format: "png".into(),
            data_base64: "AAAA".into(),
            width: 1280,
            height: 800,
            scale: 1.0,
            method: "printwindow".into(),
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["data_base64"], "AAAA");
        assert_eq!(v["method"], "printwindow");

        // method defaults to "renderer" when an older server omits it
        let r: ScreenshotResult = serde_json::from_value(json!({
            "format":"png","data_base64":"AAAA","width":1,"height":1,"scale":1.0
        }))
        .unwrap();
        assert_eq!(r.method, "renderer");

        // defaults: timeout_ms 5000, quiet_ms 150
        let p: WaitIdleParams = serde_json::from_value(json!({"token":"t","window_id":0})).unwrap();
        assert_eq!(p.timeout_ms, 5000);
        assert_eq!(p.quiet_ms, 150);
    }

    #[test]
    fn click_params_defaults() {
        let p: ClickParams =
            serde_json::from_value(json!({"token":"t","window_id":0,"id":"x"})).unwrap();
        assert_eq!(p.button, MouseButton::Left);
        assert!(p.modifiers.is_empty());
    }

    #[test]
    fn phase2_params() {
        let p: TypeTextParams =
            serde_json::from_value(json!({"token":"t","window_id":0,"text":"hello"})).unwrap();
        assert_eq!(p.text, "hello");
        let p: KeyParams =
            serde_json::from_value(json!({"token":"t","window_id":0,"combo":"ctrl-s"})).unwrap();
        assert_eq!(p.combo, "ctrl-s");
        let p: ScrollParams =
            serde_json::from_value(json!({"token":"t","window_id":0,"id":"list","delta_y":-120.0}))
                .unwrap();
        assert_eq!(p.delta_y, -120.0);
        assert_eq!(p.delta_x, 0.0);
        let p: QueryParams =
            serde_json::from_value(json!({"token":"t","window_id":0,"text_contains":"Save"}))
                .unwrap();
        assert_eq!(p.text_contains.as_deref(), Some("Save"));
        assert!(p.id_contains.is_none());
    }
}
