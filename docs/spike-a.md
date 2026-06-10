# Spike A — screenshots of occluded / minimized / locked GPUI windows on Windows

**Question (design §7.1):** can we read back the rendered frame of a real GPUI window
while it is (a) fully occluded, (b) minimized, (c) the Windows session is locked —
without depending on DWM/desktop composition?

**Answer: yes — with a one-method patch to `gpui_windows`, verified empirically on all
three scenarios on 2026-06-10 (Windows 11 Pro 26200, zed rev `20a3f77`).**

## Approach that works

GPUI already has the right seam: `Window::render_to_image()` (gpui `window.rs:2237`,
behind the `test-support` feature) forwards to `PlatformWindow::render_to_image(&Scene)`.
Only the test platform and macOS Metal implement it; `gpui_windows` does not.

The patch (vendored in `vendor/gpui_windows`, applied via `[patch."https://github.com/zed-industries/zed"]`):

1. `DirectXRenderer::render_scene_to_image(&mut self, scene, background_appearance)`
   (`directx_renderer.rs`): runs the **exact normal draw path** (`pre_draw` →
   `upload_scene_buffers` → `draw_batches`) into the existing swapchain backbuffer, but
   **never calls `Present`**. Then `CopyResource` backbuffer → `D3D11_USAGE_STAGING`
   texture → `Map(D3D11_MAP_READ)` → BGRA→RGBA → `image::RgbaImage`. The staging-readback
   pattern mirrors what `direct_write.rs` already does for glyph rasterization.
2. `impl PlatformWindow for WindowsWindow`: `render_to_image(&self, scene)` delegates to
   the renderer with the window's current background appearance (`window.rs`).
3. The vendored crate hard-enables `gpui/test-support` so the trait method exists in
   every build using the patch.

Because presentation/DWM is never involved — it's plain D3D11 GPU work plus a CPU
readback — occlusion and session lock do not affect it.

## Evidence

`examples/spike-a` renders a 10 Hz ticking counter and captures once per second while
walking a scenario timeline. The tick value + elapsed time are baked into each frame, so
staleness is visible by eye.

| Scenario | Result |
|---|---|
| (pre) focused | fresh captures, e.g. tick 75 @ 8.3s |
| fully occluded by another window | fresh captures, ticks advance |
| minimized (`window.minimize_window()`) | **fresh captures** — tick 103 @ 11.4s → 131 @ 14.4s |
| session locked (`LockWorkStation`, 25 s) | **fresh captures throughout** — tick 150 @ 16.3s → 264 @ 28.7s |

All 35+18 captures across both runs returned 500×300 images with zero errors. Notably,
even minimized windows kept producing fresh frames at this rev (entity notify → redraw
still runs), so the "stale scene while minimized" degradation we anticipated did not
materialize. If it ever does, `Window::draw(cx)` is public and lets the driver force a
fresh frame before capturing.

## Decisions for gpui-driver

- `screenshot` uses `window.render_to_image()`; the lib enables `gpui/test-support` on
  its own gpui dependency.
- Windows hosts need the `[patch]` section pointing at `vendor/gpui_windows` (README
  documents this). No fork of gpui itself is needed.
- macOS already implements the headless Metal path upstream; Linux (blade) is untested
  and out of v0 scope → `unsupported` error.

## Upstreaming

The patch is small, additive, and mirrors the macOS test-support capability — a good
candidate PR to zed (`gpui_windows`: implement `PlatformWindow::render_to_image`). Until
merged, the vendored copy is pinned to the same rev as the gpui git dependency.
