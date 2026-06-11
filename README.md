# gpui-driver

Agent-driven runtime automation for [GPUI](https://www.gpui.rs/) applications.

A small in-process server crate plus a CLI that lets a coding agent (e.g. Claude Code)
inspect a running GPUI app, click through it, and visually verify the result via
screenshots — **without requiring window focus, and even when the desktop session is
locked**. Verified empirically: screenshots stay fresh while the window is fully
occluded, minimized, or the Windows session is locked (see [docs/spike-a.md](docs/spike-a.md)).

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

## Why in-process?

OS-level input injection (`SendInput`, UIA, enigo) needs focus and an unlocked session.
gpui-driver instead dispatches synthetic input through GPUI's own public event path
(`Window::dispatch_event`) — the same hit-testing and handlers a real user exercises —
and reads screenshots back from the renderer, not from the screen. Your machine stays
usable while an agent tests your app, and CI-style runs survive lock screens.

## Integrating your app

```toml
# Cargo.toml
[dependencies]
gpui-driver = { git = "https://github.com/maui1911/gpui-driver", optional = true }

[features]
driver = ["dep:gpui-driver"]
```

```rust
fn main() {
    gpui_platform::application().run(|cx| {
        #[cfg(feature = "driver")]
        gpui_driver::init(cx); // starts server, writes discovery file

        // ... normal app setup
    });
}
```

Annotate the elements an agent should be able to target (and only those):

```rust
use gpui_driver::DriverExt;

div()
    .id("save")
    .on_click(...)
    .driver_id("save_button")          // stable, addressable id
    .driver_text("Save")               // optional text reported in `tree`
```

For **reliable** screenshots on Windows (fresh frames while occluded, minimized, or
session-locked), add the vendored `gpui_windows` patch to your workspace root:

```toml
[patch."https://github.com/zed-industries/zed"]
gpui_windows = { git = "https://github.com/maui1911/gpui-driver" }
```

This is the supported, permanent setup — not a stopgap. The patch is two additive
methods (`render_to_image`, `get_title`), it lives as a standalone diff in
[`vendor/patches/`](vendor/patches/), and rev bumps are mechanical via
[`tools/update-vendor.sh`](tools/update-vendor.sh). Upstreaming to zed would make it
unnecessary, but nothing here depends on that happening.

Without the patch the driver still works: `screenshot` falls back to Win32
`PrintWindow` and reports which path produced the image (see
[Screenshot methods](#screenshot-methods)).

> **⚠️ Never enable the `driver` feature in release builds.** The server accepts
> JSON-RPC on localhost, authenticated only by a token in the user's temp directory.
> It is a debugging/testing tool. Note that the `gpui_windows` patch also enables
> gpui's `test-support` feature for the whole build graph.

## Driving an app

```console
$ gpui-driver list
demo-app   pid 6828   port 63553   started 2026-06-10T22:05:54Z   alive

$ gpui-driver tree --interactive-only
(window) [0,0 640x420]
  counter_label <Div> "Count: 0" [24,24 592x32]
  increment_button <Stateful<Div>> "Increment" [24,72 104x42]
  open_dialog_button <Stateful<Div>> "Open dialog" [140,72 120x42]
  name_input <Stateful<Div>> "" [24,130 320x44]

$ gpui-driver click increment_button
clicked increment_button at center of [24,72 104x42]

$ gpui-driver wait-idle && gpui-driver screenshot -o shot.png
idle after 197 ms
wrote shot.png (640x420, scale 1)
```

Everything addressable has an id; clicks resolve to the element's current center
inside the lib. **Agents never deal in raw coordinates.** Clicking an element covered
by a modal fails with `element_not_visible` (exit code 2) through GPUI's real hit-test
— deliberately, because that's a real UI fact.

Commands: `list · info · windows · tree · query · click · focus · type · key · scroll ·
wait-idle · screenshot`. All accept `--app <name>` / `--pid <pid>`; `--json` gives raw
RPC output (automatic when piped). Exit codes: `0` ok · `2` element/window not found ·
`3` timeout · `4` no app found · `5` protocol/auth error.

An agent skill teaching the workflow ships in [`skill/SKILL.md`](skill/SKILL.md).

## Workspace layout

| Crate | Purpose |
|---|---|
| `crates/gpui-driver-protocol` | Shared JSON-RPC types (no gpui dependency) |
| `crates/gpui-driver` | In-app server, element registry, input, screenshots |
| `crates/gpui-driver-cli` | The `gpui-driver` binary (no gpui dependency, fast build) |
| `examples/demo-app` | Instrumented example app / e2e target |
| `examples/spike-a` | Occluded/locked screenshot verification harness |
| `vendor/gpui_windows` | Pinned vendored copy with the screenshot + `get_title` patch |

gpui is a git dependency pinned to zed rev `20a3f7705f18a9913571d4fcdee687b76abdb213`.
The host app and gpui-driver must use the *same* gpui build; a weekly CI canary builds
against zed `main` to surface breakage early. To bump the pinned rev, run
`tools/update-vendor.sh <new-rev>` — it refetches upstream `gpui_windows`, re-applies
the driver patch, and updates both manifests.

## Screenshot methods

The `screenshot` result carries a `method` field telling you how the image was
captured — agents should treat the two very differently:

| `method` | requires | occluded | minimized | session locked |
|---|---|---|---|---|
| `renderer` | the `[patch]` line above | ✅ fresh | ✅ fresh | ✅ fresh (verified) |
| `printwindow` | nothing (stock gpui) | ⚠️ best effort | ⚠️ may be stale/black | ⚠️ may be stale/black |

`renderer` reads pixels straight back from the DirectX renderer without presenting, so
the screen state is irrelevant. `printwindow` asks the window to paint itself into a
memory DC (`PrintWindow` + `PW_RENDERFULLCONTENT`); it is a fine fallback while the
window is visible, but its output is not trustworthy evidence once the window isn't.
The CLI prints a warning on stderr whenever the fallback was used.

## Protocol

Newline-delimited JSON-RPC 2.0 over localhost TCP, one object per line. Every request
carries the `token` from the discovery file. Errors use structured
`error.data.kind`: `element_not_found | element_not_visible | window_not_found |
timeout | unsupported | auth_failed | internal`. See
[`crates/gpui-driver-protocol`](crates/gpui-driver-protocol/src/lib.rs) for the exact
shapes, and [gpui-driver-DESIGN.md](gpui-driver-DESIGN.md) for the design rationale.

## Platform support

| Platform | tree/click/input | screenshot |
|---|---|---|
| Windows | ✅ | ✅ patched: `renderer` (occluded/minimized/locked) · unpatched: `printwindow` fallback |
| macOS | ✅ (untested) | possible upstream (`test-support` Metal path), untested |
| Linux | ✅ (untested) | ❌ `unsupported` (v0) |

## License

Apache-2.0 (matching the vendored `gpui_windows` code from zed, which is also
Apache-2.0).
