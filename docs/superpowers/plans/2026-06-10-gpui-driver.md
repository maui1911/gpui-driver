# gpui-driver Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the gpui-driver workspace per `gpui-driver-DESIGN.md`: an in-process JSON-RPC server crate that lets a coding agent inspect, click, and screenshot a running GPUI app without window focus, plus a CLI and an agent skill.

**Architecture:** Three crates (`gpui-driver-protocol` shared types, `gpui-driver` in-app lib, `gpui-driver-cli` binary) talking newline-delimited JSON-RPC 2.0 over localhost TCP with token auth and `%TEMP%/gpui-driver/<pid>.json` discovery. Element addressing rides on an own `driver_id` registry (wrapper `Element` capturing bounds + hitboxes each frame); screenshots ride on `Window::render_to_image()` backed by a vendored one-method patch to `gpui_windows`.

**Tech Stack:** Rust 2024, gpui pinned to zed rev `20a3f7705f18a9913571d4fcdee687b76abdb213` (git dep), serde/serde_json, clap 4, image (PNG), base64, futures channels.

**Research already done (this informs the tasks below; see docs/spike-a.md / docs/spike-b.md tasks):**
- `Window::dispatch_event(PlatformInput, cx)` and `Window::dispatch_keystroke(Keystroke, cx)` are public → synthetic input through GPUI's real hit-test path needs no patch (window.rs:4456,4498 at pinned rev).
- `Window::render_to_image()` exists behind gpui feature `test-support` (window.rs:2237) but `PlatformWindow::render_to_image` is only implemented by the test platform + macOS Metal headless renderer. `gpui_windows::WindowsWindow` lacks it → vendored patch required.
- Inspector (`inspector.rs`) tracks only hovered/selected element; frame hitboxes and the AccessKit tree are `pub(crate)` → no public full-tree enumeration → own registry confirmed (design §3 fallback).
- `cx.windows()`, `window.window_title()`, `window.bounds()`, `window.is_window_active()`, `window.on_next_frame(cb)` (registers without requesting a frame), `window.draw(cx)` all public.
- gpui on crates.io is 0.2.2 (2025-10-22, pre platform-split, no render_to_image) → pin git rev per design.
- On minimize, gpui_windows parks the frame callback (`restore_from_minimized`) → frames stop; document as degraded mode, occluded/locked are the core scenarios.

---

### Task 1: Workspace scaffolding

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `crates/gpui-driver-protocol/Cargo.toml`, `crates/gpui-driver-protocol/src/lib.rs` (stub)
- Create: `crates/gpui-driver/Cargo.toml`, `crates/gpui-driver/src/lib.rs` (stub)
- Create: `crates/gpui-driver-cli/Cargo.toml`, `crates/gpui-driver-cli/src/main.rs` (stub)
- Create: `.gitignore`, `rust-toolchain.toml` (omit — use default stable), `README.md` (short stub, expanded in Task 12)

- [ ] **Step 1: Write workspace root `Cargo.toml`**

```toml
[workspace]
resolver = "2"
members = [
    "crates/gpui-driver-protocol",
    "crates/gpui-driver",
    "crates/gpui-driver-cli",
    "examples/demo-app",
]
default-members = [
    "crates/gpui-driver-protocol",
    "crates/gpui-driver-cli",
]

[workspace.package]
version = "0.1.0"
edition = "2024"
license = "MIT OR Apache-2.0"
repository = "https://github.com/mauiwind/gpui-driver"

[workspace.dependencies]
gpui-driver-protocol = { path = "crates/gpui-driver-protocol" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
base64 = "0.22"
log = "0.4"

[profile.dev]
# gpui is heavy; keep example app linkable
debug = "line-tables-only"
```

(`examples/demo-app` joins the members list in Task 9; keep it listed from the start with the directory created in this task to avoid root manifest churn.)

- [ ] **Step 2: Stub crate manifests + lib.rs/main.rs (`fn main() {}` / empty lib), `.gitignore` (`/target`, `*.png` under `/tmp-shots`), LICENSE-MIT + LICENSE-APACHE files**
- [ ] **Step 3: `cargo check` → workspace compiles**
- [ ] **Step 4: Commit "chore: workspace scaffolding"**

### Task 2: `gpui-driver-protocol` — types + serde round-trip tests

**Files:**
- Create: `crates/gpui-driver-protocol/src/lib.rs`

- [ ] **Step 1: Write failing round-trip tests** (in-file `#[cfg(test)] mod tests`): serialize/deserialize `RpcRequest`, every `Method` params struct, `RpcResponse` success + error with `ErrorKind`, `DiscoveryFile`, and a nested `TreeNode`. Assert JSON field names match the design doc exactly (e.g. `data_base64`, `window_id`, snake_case `error.data.kind`).
- [ ] **Step 2: `cargo test -p gpui-driver-protocol` → fails (types missing)**
- [ ] **Step 3: Implement types:**

```rust
// Core JSON-RPC 2.0 envelope (newline-delimited, one object per line)
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest {
    pub jsonrpc: String,            // always "2.0"
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,  // params structs flattened below; token inside
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RpcResponse { /* id, result xor error */ }

#[derive(Debug, Serialize, Deserialize)]
pub struct RpcError { pub code: i64, pub message: String, pub data: Option<RpcErrorData> }

#[derive(Debug, Serialize, Deserialize)]
pub struct RpcErrorData { pub kind: ErrorKind }

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind { ElementNotFound, ElementNotVisible, WindowNotFound, Timeout, Unsupported, AuthFailed, Internal }

// Param/result structs per method: InfoResult, WindowsResult/WindowInfo,
// TreeParams/TreeNode, ClickParams/ClickResult, ScreenshotParams/ScreenshotResult,
// WaitIdleParams/WaitIdleResult, TypeTextParams, KeyParams,
// each with a `token: String` on params via a shared `#[serde(flatten)]`-free explicit field.
// Bounds { x, y, w, h } as f32. DiscoveryFile { app_name, pid, port, token, protocol_version, started_at }.
```

Use explicit `token: String` field on every params struct (design: token on every request). `TreeNode { id, kind, text, bounds, visible, enabled, focused, interactive, children }`.

- [ ] **Step 4: `cargo test -p gpui-driver-protocol` → PASS**
- [ ] **Step 5: Commit "feat(protocol): v1 request/response types"**

### Task 3: Spike A — occluded/locked screenshot (kill the top risk)

**Files:**
- Create: `vendor/gpui_windows/` (copy of `crates/gpui_windows` from zed @ pinned rev, deps rewritten from `workspace = true` to `{ git = "https://github.com/zed-industries/zed", rev = "20a3f77..." }`)
- Modify: `vendor/gpui_windows/src/window.rs` — implement `PlatformWindow::render_to_image(&self, scene: &Scene)`
- Modify: `vendor/gpui_windows/src/directx_renderer.rs` — add `render_scene_to_image(&mut self, scene: &Scene) -> Result<RgbaImage>`: draw scene (existing `draw()` path minus `present()`), then `CopyResource` backbuffer → `D3D11_USAGE_STAGING` texture → `Map` → BGRA→RGBA convert (readback pattern exists in `direct_write.rs:1060-1152`)
- Create: `examples/spike-a/` minimal gpui app (one window, animated counter) + driver-side hotkey/timer that calls `window.render_to_image()` every 2s and writes `tmp-shots/<n>.png`
- Create: `docs/spike-a.md` — findings

- [ ] **Step 1: Vendor gpui_windows, rewrite manifest deps, add `[patch."https://github.com/zed-industries/zed"] gpui_windows = { path = "vendor/gpui_windows" }` to workspace root**
- [ ] **Step 2: Build spike app with `gpui = { git, rev, features = ["test-support"] }` → confirm `render_to_image` reachable, returns error before patch**
- [ ] **Step 3: Implement readback; run spike: capture with window (a) focused, (b) fully occluded by another window, (c) minimized, (d) session locked (`rundll32 user32.dll,LockWorkStation`, capture via timer, then unlock and inspect PNGs)**
- [ ] **Step 4: Record results + chosen approach in `docs/spike-a.md`; note upstreaming path (PR adding render_to_image to gpui_windows)**
- [ ] **Step 5: Commit "spike: windows render_to_image readback (vendored gpui_windows patch)"**

Fallback if backbuffer readback fails when locked: render to a dedicated offscreen `ID3D11Texture2D` render target instead of the swapchain buffer (same draw calls, own RTV) — presentation never involved. Document whichever works.

### Task 4: Spike B — inspector data (documentation task)

**Files:**
- Create: `docs/spike-b.md`

- [ ] **Step 1: Write up the (already completed) source analysis at pinned rev:** inspector tracks only the active element; `Frame::hitboxes`, `inspector_hitboxes`, and the AccessKit tree are `pub(crate)`; `tree` therefore uses the `driver_id` registry; future direction = upstream public a11y-tree access. Include file/line references.
- [ ] **Step 2: Commit "docs: spike B findings — tree rides on driver_id registry"**

### Task 5: `gpui-driver` lib — registry + DriverExt

**Files:**
- Create: `crates/gpui-driver/src/registry.rs` — `DriverRegistry` (global, `parking_lot::Mutex<HashMap<WindowId, WindowNodes>>`): per frame, nodes record `{ driver_id, kind, bounds, hitbox_id, parent_index, frame_count }`; thread-local parent stack during prepaint
- Create: `crates/gpui-driver/src/element.rs` — `DriverNode<E: Element>` wrapper implementing `Element`: delegates `id`/`source_location`/`request_layout`; in `prepaint` pushes onto parent stack, delegates inner prepaint, pops, records bounds + `window.insert_hitbox(bounds, HitboxBehavior::Normal)`; `paint` delegates. `DriverExt` trait: `fn driver_id(self, id: impl Into<SharedString>) -> DriverNode<Self>` blanket-implemented for `Element + Sized`, plus `.driver_text(...)` optional label
- Create: `crates/gpui-driver/src/lib.rs` — `pub fn init(cx: &mut App)` (stub until Task 6), re-export `DriverExt`

- [ ] **Step 1: Write unit test for registry tree assembly** (flat records with parent indices → nested `TreeNode` JSON; stale-frame eviction)
- [ ] **Step 2: `cargo test -p gpui-driver` → FAIL → implement → PASS**
- [ ] **Step 3: Commit "feat(driver): element registry + DriverExt"**

### Task 6: `gpui-driver` lib — server thread, discovery, info/windows/tree

**Files:**
- Create: `crates/gpui-driver/src/server.rs` — `std::thread` TCP listener on `127.0.0.1:0`; per-connection thread; newline-delimited JSON-RPC; token check; forwards `(RpcRequest, futures oneshot Sender)` over `futures::channel::mpsc::unbounded` to the foreground
- Create: `crates/gpui-driver/src/discovery.rs` — write `%TEMP%/gpui-driver/<pid>.json` on init, remove in `cx.on_app_quit`
- Create: `crates/gpui-driver/src/handlers.rs` — foreground loop: `cx.spawn(async move |cx| while let Some((req, tx)) = rx.next().await { ... })`; handlers for `info`, `windows` (via `cx.windows()` + `update_window` for title/bounds/active), `tree` (registry snapshot; force `window.draw(cx)` first if a refresh is pending so bounds are fresh)
- Modify: `crates/gpui-driver/src/lib.rs` — real `init(cx)`

- [ ] **Step 1: Integration-style test with demo app deferred to Task 9 CI; unit-test the JSON-RPC framing (feed a `TcpStream` via loopback in a plain test, mock handler thread)**
- [ ] **Step 2: Implement; `cargo check -p gpui-driver` with gpui dep compiles**
- [ ] **Step 3: Commit "feat(driver): tcp json-rpc server + discovery + info/windows/tree"**

### Task 7: synthetic `click` + `wait_idle` + `screenshot`

**Files:**
- Modify: `crates/gpui-driver/src/handlers.rs`

- [ ] **Step 1: `click`:** resolve element by id in registry → center point → `dispatch_event(PlatformInput::MouseMove{..})` → check our hitbox is hovered via stored `HitboxId` (occlusion = real hit-test) → `MouseDown`/`MouseUp` (button/modifiers from params, `click_count: 1`) → respond with resolved bounds; errors `element_not_found` / `element_not_visible`
- [ ] **Step 2: `wait_idle`:** per-window `Arc<Mutex<Instant>>` updated by a self-re-arming `window.on_next_frame` callback installed at init; handler polls every 25ms on the foreground executor until `quiet_ms` elapsed since last frame or `timeout_ms` → `ErrorKind::Timeout`
- [ ] **Step 3: `screenshot`:** force `window.draw(cx)` when dirty → `window.render_to_image()` → PNG via `image` crate → base64; map platform error to `unsupported`
- [ ] **Step 4: Commit "feat(driver): click, wait_idle, screenshot"**

### Task 8: `gpui-driver-cli`

**Files:**
- Create: `crates/gpui-driver-cli/src/main.rs` (clap), `src/client.rs` (discovery scan + liveness probe via TCP connect/info, TcpStream JSON-RPC client), `src/output.rs` (human tree rendering, `--json` passthrough)

- [ ] **Step 1: Unit tests for discovery selection (tempdir with fake discovery files: dead-pid cleanup via failed connect, `--app`/`--pid`/auto-select-when-one), tree compact rendering, exit-code mapping (0/2/3/4/5 per design §6)**
- [ ] **Step 2: Implement commands `list,info,windows,tree,click,screenshot,wait-idle` (+`type`,`key` wired once Task 10 lands); `--json` default when stdout not a TTY (`std::io::IsTerminal`)**
- [ ] **Step 3: `cargo test -p gpui-driver-cli` PASS; commit "feat(cli): gpui-driver binary"**

### Task 9: example app + end-to-end verification

**Files:**
- Create: `examples/demo-app/` — gpui app via `gpui_platform::application()`: window with `driver_id`'d buttons (one increments a counter label), a button opening a dialog overlay (tests popover tree + occlusion click error), a text field (Phase 2 target)

- [ ] **Step 1: Build & run demo app with `--features driver`**
- [ ] **Step 2: Drive it with the CLI end-to-end: `list → tree → click increment → wait-idle → screenshot → verify PNG shows counter=1` (use window-capture/Read on the PNG to visually confirm); repeat occluded; verify occluded-element click returns exit code 2**
- [ ] **Step 3: Commit "feat(examples): demo app + e2e verification notes"**

### Task 10: Phase 2 inputs — `type_text`, `key`, `scroll`, `focus`, `query`

- [ ] **Step 1: `key`: `Keystroke::parse(combo)` → `window.dispatch_keystroke`; `type_text`: per-char keystrokes falling back to `PlatformInput::KeyDown` with key_char; `scroll`: `ScrollWheelEvent` at element center; `focus`: dispatch click at center (v0 semantics, documented); `query`: registry filter by `text_contains`/`id_contains`**
- [ ] **Step 2: Wire CLI subcommands; extend demo-app e2e (type into text field, screenshot-verify)**
- [ ] **Step 3: Commit "feat: phase 2 input methods"**

### Task 11: agent skill

**Files:**
- Create: `skill/SKILL.md` — workflow per design §8 (list → tree --interactive-only → act by id only → wait-idle → screenshot → look; element_not_found → grep app source for driver_id)

- [ ] **Step 1: Write SKILL.md with frontmatter (name, description, trigger guidance), commands reference, exit-code table, anti-patterns (never coordinates, never click blind)**
- [ ] **Step 2: Commit "feat(skill): agent skill"**

### Task 12: docs + CI

**Files:**
- Create: `README.md` (full: quickstart, integration snippet from design §3, security warning re release builds, patch/vendoring story, pinned rev policy), `AGENTS.md`, `.github/workflows/ci.yml` (fmt, clippy -D warnings, test on windows-latest + ubuntu-latest for non-gpui crates), `.github/workflows/canary.yml` (weekly build against zed main, `continue-on-error`)

- [ ] **Step 1: Write docs; Step 2: validate workflows with `gh` if available or YAML lint; Step 3: Commit "docs+ci"**

### Task 13: final review

- [ ] **Step 1: `cargo fmt --all --check`, `cargo clippy --workspace`, `cargo test --workspace` green**
- [ ] **Step 2: Re-read design doc top-to-bottom; check every §5 Phase-1+2 method, §6 CLI behavior, exit codes, discovery semantics implemented; fix gaps**
- [ ] **Step 3: Final commit**

---

## Self-review notes

- Spec coverage: §3 integration (T5/T9), §4 transport/discovery (T6), §5 phase 1 (T6/T7) + phase 2 (T10), §6 CLI (T8), §7 spikes (T3/T4) and build order respected, §8 skill (T11), §9 risks → canary CI + vendored patch + hitbox-based occlusion check, §10 licensing/layout (T1/T12). Phase 3 explicitly out of scope (design: nice-to-have).
- Types referenced across tasks: `TreeNode`/`ErrorKind` defined in T2 and reused; registry record defined in T5 and consumed by T6/T7.
- Known risk carried forward: exact gpui API details (event field names, hitbox hover semantics) are verified against `C:\dev\zed-ref` source during T3/T5/T7 rather than frozen in this plan, since the pinned-rev source is authoritative and locally available.
