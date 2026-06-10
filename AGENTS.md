# AGENTS.md

Guidance for coding agents working **on** this repository. (For driving GPUI apps
**with** this tool, read `skill/SKILL.md` instead.)

## What this is

Three-crate workspace: `gpui-driver-protocol` (serde types, no gpui), `gpui-driver`
(in-app JSON-RPC server, gpui git dep), `gpui-driver-cli` (the binary, no gpui).
`gpui-driver-DESIGN.md` is the design source of truth; `docs/spike-a.md` and
`docs/spike-b.md` record why the architecture is what it is.

## Build & test

```console
cargo test -p gpui-driver-protocol -p gpui-driver-cli   # fast, no gpui
cargo test -p gpui-driver                               # compiles gpui (slow first time)
cargo build -p demo-app && ./target/debug/demo-app.exe  # e2e target
./target/debug/gpui-driver.exe list                     # then drive it
```

`default-members` excludes gpui-dependent crates so a bare `cargo check` stays fast.

## Invariants to preserve

- **gpui rev is pinned** in the workspace `Cargo.toml` AND in
  `vendor/gpui_windows/Cargo.toml` (three dep entries there). Bump them together, and
  re-diff the vendored crate against `crates/gpui_windows` at the new rev — it is a
  copy with two additive changes: `render_to_image` (renderer readback, see
  `directx_renderer.rs` + `window.rs`) and `get_title`.
- **Everything UI-touching runs on the GPUI main thread.** Server threads only parse
  and forward; handlers run inside `cx.spawn`/`update_window`. Never call gpui from
  the TCP threads.
- **The agent never needs raw coordinates.** Don't add coordinate-based RPCs.
- **Exit codes are a stable contract** (0/2/3/4/5, see `crates/gpui-driver-cli/src/main.rs`).
- **Synthetic input must go through `Window::dispatch_event` / `dispatch_keystroke`**
  (the real hit-test path), not direct handler invocation.
- Registry collection only happens during handler-forced draws (`begin_collect` →
  `refresh()` + `draw(cx).clear()` → `end_collect`); normal frames must stay cheap.

## Gotchas

- `Window::render_to_image` only exists when gpui's `test-support` feature is on; the
  vendored `gpui_windows` enables it unconditionally for that reason.
- GPUI caches clean views' paint (`reuse_paint`); `window.refresh()` before a forced
  draw is what guarantees every `DriverNode` re-prepaints.
- `dispatch_keystroke` simulates IME (`with_simulated_ime`), which is what makes
  `type_text` reach `key_char`-reading handlers and input handlers.
- Discovery files in `%TEMP%/gpui-driver/<pid>.json`; the CLI deletes entries whose
  port no longer accepts connections.
