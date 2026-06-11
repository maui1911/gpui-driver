# gpui-driver — Design Document

Agent-driven runtime automation for GPUI applications. A small in-process server crate plus a
CLI that lets a coding agent (e.g. Claude Code) inspect a running GPUI app, click through it,
and visually verify the result via screenshots — **without requiring window focus, and even
when the desktop session is locked**.

Status: pre-implementation design. This document is the source of truth for the initial build.

---

## 1. Goals & non-goals

### Goals
- Drive a *real, running* GPUI application (not a `TestAppContext` unit test).
- Work when the app window is unfocused, occluded, or the Windows session is locked.
  This forces the core architectural decision: **everything happens in-process**.
- Agent-friendly CLI: stable ids, JSON output, meaningful exit codes.
- Generic: a reusable community crate, not tied to one app. Conventions follow the
  `gpui-*` ecosystem (`gpui-component`, `gpui-mobile`, ...).
- Never present in release builds (feature-gated, off by default).

### Non-goals (v0.x)
- No MCP server (a CLI + an agent skill is the integration surface).
- No OS-level input injection (`SendInput`/enigo) — it requires focus and an unlocked
  session, which violates the core goal. All input is synthetic, dispatched inside GPUI.
- No record/replay, no assertion DSL, no visual diffing. The agent's eyes are the assertions.
- No support for non-GPUI windows embedded in the app.

---

## 2. Architecture overview

```
┌─────────────────────────────┐         ┌──────────────────┐
│ GPUI app (debug build)      │         │ gpui-driver CLI  │
│                             │  TCP    │ (separate proc)  │
│  gpui-driver lib            │◄───────►│                  │
│  ├─ JSON-RPC server         │ JSON-   │  tree / click /  │
│  ├─ element registry        │ RPC     │  screenshot /    │
│  ├─ synthetic input dispatch│         │  wait-idle ...   │
│  └─ backbuffer screenshot   │         └──────────────────┘
└─────────────────────────────┘
        │ writes on startup
        ▼
  %TEMP%/gpui-driver/<pid>.json   (discovery: app name, pid, port, auth token)
```

Three crates in one workspace / one GitHub repo:

| Crate                  | Type | Depends on gpui? | Purpose                                  |
|------------------------|------|------------------|------------------------------------------|
| `gpui-driver-protocol` | lib  | no               | Shared request/response types (serde)     |
| `gpui-driver`          | lib  | yes (git dep)    | In-app server, registry, input, screenshot|
| `gpui-driver-cli`      | bin  | no               | The `gpui-driver` binary the agent calls  |

Rationale for the split: the CLI must compile fast and must not drag in gpui (which is a git
dependency with frequent breaking changes). The protocol crate keeps both sides in lockstep.

---

## 3. In-app integration (what an app author does)

```toml
# Cargo.toml of the host app
[dependencies]
gpui-driver = { version = "0.1", optional = true }

[features]
driver = ["dep:gpui-driver"]
```

```rust
fn main() {
    Application::new().run(|cx| {
        #[cfg(feature = "driver")]
        gpui_driver::init(cx); // starts server, writes discovery file

        // ... normal app setup
    });
}
```

Element annotation via an extension trait (only interactive elements that the agent
should be able to target need an id):

```rust
use gpui_driver::DriverExt;

div()
    .id("save_button")
    .driver_id("save_button")   // registers bounds + metadata each frame
    .on_click(...)
```

**Spike B (see §7) may eliminate `driver_id` entirely**: GPUI ships an `inspector` feature
(`gpui_macros/inspector`) that already tracks an `InspectorElementId` — a `GlobalElementId`
qualified by the source location of element construction — plus picking/selection state.
If that data is reachable from library code, the tree endpoint becomes a serialization job
and `driver_id` becomes an optional "stable alias" on top of inspector ids.

---

## 4. Transport & discovery

- **Transport:** TCP on `127.0.0.1`, ephemeral port. Newline-delimited JSON-RPC 2.0
  (one request/response object per line). TCP instead of named pipes to stay
  cross-platform (Windows first, but nothing should preclude macOS/Linux).
- **Discovery:** on `init()`, write `%TEMP%/gpui-driver/<pid>.json`:

```json
{
  "app_name": "codescope",
  "pid": 31337,
  "port": 49213,
  "token": "<32 random bytes, hex>",
  "protocol_version": 1,
  "started_at": "2026-06-10T14:03:11Z"
}
```

  File is removed on clean shutdown; the CLI treats entries with dead pids as stale and
  deletes them. The token must be sent as `"token"` param on every request — cheap
  protection against other local processes poking the socket.
- **Threading:** the server runs on a background thread; every RPC that touches UI state is
  marshalled onto the GPUI main thread (`cx.spawn` / foreground executor) and awaited.

---

## 5. RPC protocol (v1)

All requests: `{"jsonrpc":"2.0","id":N,"method":"...","params":{"token":"...", ...}}`.
Errors use standard JSON-RPC error objects with structured `data.kind`:
`element_not_found | element_not_visible | window_not_found | timeout | unsupported`.

### Phase 1 (MVP — this is 90% of the value)

#### `info`
→ `{ "app_name", "app_version", "protocol_version", "gpui_driver_version" }`

#### `windows`
→ `{ "windows": [ { "window_id": 0, "title": "CodeScope", "bounds": {x,y,w,h}, "active": true } ] }`

#### `tree`
params: `{ "window_id": 0 }`
→ nested element tree. Node shape:

```json
{
  "id": "save_button",            // driver_id if set, else inspector-derived id
  "kind": "div",
  "text": "Save",                 // visible text content, best effort
  "bounds": { "x": 412.0, "y": 88.0, "w": 96.0, "h": 32.0 },
  "visible": true,
  "enabled": true,
  "focused": false,
  "interactive": true,            // has click/key handlers
  "children": [ ... ]
}
```

Design rule: **the agent must never need raw coordinates.** Everything addressable has an id;
clicks resolve to the center of the element's current bounds *inside* the lib.

#### `click`
params: `{ "window_id": 0, "id": "save_button", "button": "left", "modifiers": [] }`
Dispatches synthetic `MouseMove` → `MouseDown` → `MouseUp` through GPUI's event path on the
element's current center. Returns `{ "clicked": true, "resolved_bounds": {...} }`.
Errors if the element is missing, zero-sized, or occluded by another element at that point
(hit-test check — this is deliberate: it catches real UI bugs).

#### `screenshot`
params: `{ "window_id": 0 }`
→ `{ "format": "png", "data_base64": "..." , "width": 1280, "height": 800, "scale": 1.0 }`
Implementation: force a redraw, then read back the renderer's backbuffer/target texture.
Must not depend on DWM/desktop composition (that path dies when locked/occluded). See Spike A.

#### `wait_idle`
params: `{ "window_id": 0, "timeout_ms": 5000, "quiet_ms": 150 }`
Resolves when no frame has been requested for `quiet_ms` (i.e. animations/effects settled),
or errors with `timeout`. The agent calls this between every action and screenshot.

### Phase 2

- `type_text` `{ "text": "hello" }` — synthetic key/IME events to the focused element
- `key` `{ "combo": "ctrl-s" }` — parsed like GPUI keystrokes
- `scroll` `{ "id": "...", "delta_y": -120 }`
- `focus` `{ "id": "..." }`
- `query` `{ "text_contains": "Save" }` — find elements without dumping the whole tree

### Phase 3 (nice-to-have)
- `subscribe_frames` (streaming screenshots), `drag`, `hover`, multi-monitor metadata.

---

## 6. CLI design

Binary name: `gpui-driver`. All commands accept `--app <name>` or `--pid <pid>`
(if exactly one instrumented app is running, auto-select). `--json` for machine output
(default when stdout is not a TTY — agents get JSON for free).

```
gpui-driver list                          # discovery files + liveness
gpui-driver info --app codescope
gpui-driver windows --app codescope
gpui-driver tree --app codescope [--window 0] [--interactive-only]
gpui-driver click save_button --app codescope
gpui-driver screenshot --app codescope -o shot.png
gpui-driver wait-idle --app codescope [--timeout 5000]
# phase 2:
gpui-driver type "hello world" --app codescope
gpui-driver key ctrl-s --app codescope
```

Exit codes: `0` ok · `2` element/window not found · `3` timeout · `4` no app found / stale
discovery · `5` protocol/auth error. Human output is terse; `--json` mirrors the RPC response.

`tree` default output is a compact indented list (id, kind, text, bounds) rather than raw
JSON — cheaper in agent context windows. `--json` gives the full structure.

---

## 7. Build order (spikes first — kill risk early)

1. **Spike A — occluded/locked screenshot (highest risk).**
   Minimal GPUI app on Windows; attempt to read back the rendered frame from the renderer
   target after forcing a redraw, while the window is (a) fully occluded, (b) minimized,
   (c) session locked. Watch for presentation throttling — we may need to render to texture
   explicitly rather than rely on the swapchain, and possibly carry a small patch/fork of
   gpui if the renderer isn't reachable from library code. Output of the spike: a written
   note in `docs/spike-a.md` stating which approach works and what (if anything) needs
   upstreaming/vendoring.
2. **Spike B — inspector data.**
   Build gpui with `features = ["inspector"]`, read `crates/gpui/src/inspector.rs`, and
   determine: can a library crate enumerate elements + bounds per frame through public(ish)
   API? Outcome decides whether `tree` rides on inspector ids or on our own
   `driver_id` registry. Document in `docs/spike-b.md`.
3. `gpui-driver-protocol`: types + serde round-trip tests.
4. `gpui-driver` lib: server thread, discovery file, `info`/`windows`/`tree`.
5. Synthetic input: `click` (reuse the dispatch approach GPUI's own test framework uses
   for simulated input, but against the real `Window`).
6. `wait_idle` + `screenshot` (wire in Spike A's result).
7. `gpui-driver-cli`.
8. Example app in `examples/` (a few buttons, a dialog, a text field) — doubles as the
   integration test target in CI.
9. The agent skill (see §8).

## 8. Agent skill (ships in `skill/` in the repo)

SKILL.md teaching the workflow, roughly:

1. `gpui-driver list` → pick the app.
2. Always `tree --interactive-only` before acting; **never click coordinates**, only ids.
3. Action → `wait-idle` → `screenshot` → *look at the image* and judge the result.
4. If the tree doesn't match expectations, re-fetch the tree; don't click blind.
5. On `element_not_found`, suspect a renamed/refactored id — grep the app source for
   `driver_id` before retrying.

## 9. Risks & open questions

- **gpui is a moving target** (git dependency, breaking changes between releases; crates.io
  releases lag). Mitigation: pin a rev, CI job that builds against zed `main` weekly to
  surface breakage early.
- **Renderer readback may need gpui changes.** If Spike A shows the renderer isn't reachable,
  options: upstream a small `capture_frame()` hook to gpui (Zed has shown openness to
  devtools-adjacent features), or vendor a patched gpui behind a cargo `[patch]`.
- **Hit-testing fidelity:** synthetic events bypass the OS but must still go through GPUI's
  real hit-test path, otherwise we test less than a real user. Verify in step 5.
- **Multiple windows / popovers:** GPUI overlays (menus, tooltips) — make sure they appear
  in `tree` and are clickable. Likely fine since they're regular elements, but verify with
  the example app.
- **Security:** debug-only feature, localhost + token, discovery files in the user temp dir.
  README must state clearly: never enable the `driver` feature in release builds.

## 10. Licensing & repo conventions

- License: Apache-2.0 (matches the vendored zed code).
- Repo: `gpui-driver` on GitHub. Workspace layout:

```
gpui-driver/
├── Cargo.toml              # workspace
├── crates/
│   ├── gpui-driver-protocol/
│   ├── gpui-driver/
│   └── gpui-driver-cli/
├── examples/demo-app/
├── skill/SKILL.md
├── docs/spike-a.md, spike-b.md
└── AGENTS.md
```
- CI: Gitea Actions or GitHub Actions — fmt, clippy, test, plus the weekly zed-main canary
  build. Trivy/Semgrep per the usual security-scanning setup.
