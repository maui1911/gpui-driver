# Screenshot Fallback + Vendor Tooling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the vendored gpui_windows patch sustainable as the permanent plan of record (patch file + update script), add a no-patch PrintWindow screenshot fallback with an explicit reliability indicator, and reframe the docs accordingly.

**Architecture:** Three independent improvements. (1) The vendored change is captured as a `.patch` file plus a PowerShell script that regenerates `vendor/gpui_windows` from upstream at any rev and re-applies the patch, making rev bumps mechanical. (2) `handle_screenshot` gains a fallback: when `render_to_image` is unsupported (stock gpui_windows, no `[patch]` line), capture via Win32 `PrintWindow(PW_RENDERFULLCONTENT)` using the HWND obtained through gpui's public `HasWindowHandle` impl; the RPC result gains a `method` field (`"renderer"` | `"printwindow"`) so agents know capture reliability. (3) README/AGENTS/SKILL docs present vendoring as the supported route and upstreaming as opportunistic.

**Tech Stack:** Rust (windows 0.61 crate, raw-window-handle 0.6), PowerShell for tooling.

**Verified premise:** `gpui::Window` implements `raw_window_handle::HasWindowHandle` publicly (zed-ref `crates/gpui/src/window.rs:5986`, delegates to `PlatformWindow: HasWindowHandle`, platform.rs:620) — the HWND is reachable on stock gpui without any patch.

---

### Task 1: Protocol — `method` field on ScreenshotResult

**Files:**
- Modify: `crates/gpui-driver-protocol/src/lib.rs`

- [ ] Add `pub method: String` to `ScreenshotResult` with `#[serde(default = "default_screenshot_method")]` where the default fn returns `"renderer"` (back-compat: older servers omit it).
- [ ] Update the existing serialization test; add a deserialization test asserting the default kicks in when the field is absent.
- [ ] `cargo test -p gpui-driver-protocol` → green. Commit.

### Task 2: PrintWindow capture module

**Files:**
- Create: `crates/gpui-driver/src/printwindow.rs` (cfg(windows))
- Modify: `crates/gpui-driver/Cargo.toml`, `crates/gpui-driver/src/lib.rs`

- [ ] Add cfg(windows) deps: `windows = { version = "0.61", features = ["Win32_Foundation", "Win32_Graphics_Gdi", "Win32_UI_WindowsAndMessaging"] }`; add `raw-window-handle = "0.6"` as a regular dep.
- [ ] `pub(crate) fn capture_hwnd(hwnd: isize) -> anyhow::Result<image::RgbaImage>`:
  GetClientRect → CreateCompatibleDC(None) → top-down 32bpp BITMAPINFO → CreateDIBSection → SelectObject → `PrintWindow(hwnd, hdc, PRINT_WINDOW_FLAGS(PW_CLIENTONLY.0 | PW_RENDERFULLCONTENT))` → copy DIB bits → BGRA→RGBA with alpha forced to 255 → RgbaImage. Clean up GDI objects on all paths.
- [ ] `cargo check -p gpui-driver` (full workspace build is Windows-only anyway). Commit.

### Task 3: Handler fallback wiring

**Files:**
- Modify: `crates/gpui-driver/src/handlers.rs`

- [ ] `capture_image` returns `(image::RgbaImage, &'static str)` (image + method). Order: unless env `GPUI_DRIVER_FORCE_PRINTWINDOW=1`, try `render_to_image` first; on "not implemented" (or when forced) fall back: `HasWindowHandle::window_handle(window)` → `RawWindowHandle::Win32` → `capture_hwnd`. Non-Windows keeps the current Unsupported error.
- [ ] PrintWindow runs on the GPUI main thread (the window's own thread) — required, since it synchronously sends WM_PRINTCLIENT.
- [ ] `handle_screenshot` passes `method` through into `ScreenshotResult`.
- [ ] `cargo check -p gpui-driver`, existing tests green. Commit.

### Task 4: CLI surfaces the method

**Files:**
- Modify: `crates/gpui-driver-cli/src/main.rs`

- [ ] Human mode: after writing the PNG, if `method == "printwindow"` print a stderr warning: capture used the PrintWindow fallback and may be stale or black while occluded/minimized/locked; apply the vendored patch for reliable capture (see README). JSON mode needs no change (field flows through).
- [ ] `cargo test -p gpui-driver-cli` → green. Commit.

### Task 5: E2E verification on the demo app

- [ ] Build & run demo-app; `gpui-driver screenshot -o tmp-shots/renderer.png --json` → `"method":"renderer"`, image looks right.
- [ ] Restart demo-app with `GPUI_DRIVER_FORCE_PRINTWINDOW=1`; screenshot → `"method":"printwindow"`, stderr warning shown in human mode, image looks right while focused.
- [ ] Commit any fixes.

### Task 6: Vendor patch file + update script

**Files:**
- Create: `vendor/patches/gpui_windows-driver.patch`, `vendor/patches/upstream-Cargo.toml`, `tools/update-vendor.ps1`

- [ ] Generate the patch: temp sparse clone of zed at the pinned rev → init a throwaway git repo containing upstream `crates/gpui_windows` contents → overlay our `vendor/gpui_windows` source files (NOT Cargo.toml) → `git diff` → save as the patch (paths like `src/window.rs`, applied later with `git apply --directory=vendor/gpui_windows`).
- [ ] Save upstream's `crates/gpui_windows/Cargo.toml` at the pinned rev as `vendor/patches/upstream-Cargo.toml` (baseline for manifest-drift detection on bumps).
- [ ] `tools/update-vendor.ps1 -Rev <sha>`: sparse-clone zed at `<sha>` → replace `vendor/gpui_windows` contents except `Cargo.toml` → `git apply --directory=vendor/gpui_windows vendor/patches/gpui_windows-driver.patch` → substitute the old rev for `<sha>` in `vendor/gpui_windows/Cargo.toml` and the workspace `Cargo.toml` → diff upstream manifest vs `vendor/patches/upstream-Cargo.toml` and warn + update baseline if changed → remind to run `cargo build -p demo-app --features driver`.
- [ ] Idempotence test: run the script with the currently pinned rev → `git status` clean (modulo line endings). Commit.

### Task 7: Documentation reframe

**Files:**
- Modify: `README.md`, `AGENTS.md`, `skill/SKILL.md`, `docs/spike-a.md`

- [ ] README: vendored patch = the supported, permanent route (small additive diff, mechanical rebase via `tools/update-vendor.ps1`, weekly canary CI); upstreaming = opportunistic, not load-bearing. Document the PrintWindow fallback + `method` field with a reliability table (renderer: works occluded/minimized/locked; printwindow: best effort, visible-window only).
- [ ] AGENTS.md: rev-bump procedure now goes through the script; keep the BOTH-manifests invariant note.
- [ ] SKILL.md: tell agents to check `method` in JSON output; with `printwindow`, treat occluded/locked screenshots as unreliable evidence.
- [ ] docs/spike-a.md: upstreaming note reworded to opportunistic.
- [ ] Commit.

### Finish

- [ ] `cargo fmt --all`, `cargo clippy` clean, full test suite green.
- [ ] Merge `dev` → `main` (--no-ff), push, update memory.
