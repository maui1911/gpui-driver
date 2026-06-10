# Spike B — can `tree` ride on GPUI's inspector data?

**Question (design §7.2):** GPUI ships an `inspector` feature that tracks
`InspectorElementId`s. Can a library crate enumerate elements + bounds per frame through
public(ish) API, so the `tree` endpoint becomes a serialization job and `driver_id`
becomes an optional alias?

**Answer: no — `tree` rides on our own `driver_id` registry.**

All findings below are against zed rev `20a3f7705f18a9913571d4fcdee687b76abdb213`
(the rev gpui-driver pins).

## Findings

### Inspector machinery tracks one element, not a tree

`crates/gpui/src/inspector.rs` defines `Inspector` as *selection state*: an
`active_element: Option<InspectedElement>` plus a picking depth. There is no per-frame
collection of all elements. `InspectorElementId`s are minted lazily during paint/prepaint
(`Window::build_inspector_element_id`, window.rs:5602) and only retained in
`next_inspector_instance_ids` (an id-allocation counter map) and `inspector_hitboxes`.

### The per-frame data we would need is `pub(crate)`

- `Frame::hitboxes` and `Frame::inspector_hitboxes` (window.rs:837-840) — `pub(crate)`.
- `Window::rendered_frame` / `next_frame` — `pub(crate)`.
- `inspector_hitboxes` is additionally only populated **while picking mode is active**
  (`Window::insert_inspector_hitbox` early-returns unless `is_inspector_picking`,
  window.rs:5645-5660), so even with access it is not a steady-state tree source.

### Element styles/bounds reach the inspector only for the active element

`Div`'s `DivInspectorState` (elements/div.rs:1570) is written through
`Window::with_inspector_state`, which short-circuits unless the queried id *is the active
inspector element* (window.rs:5581-5599). Library code cannot use this to enumerate.

### AccessKit tree: promising future, not usable today

gpui now builds an AccessKit `TreeUpdate` per frame (`window/a11y.rs`) with node ids,
bounds (`node_bounds`), focus and action listeners — semantically exactly what `tree`
wants. But the `A11y` struct is `pub(crate)`, the tree is only built when a screen
reader/assistive client activates the adapter (`active_flag`), and Zed's own UI sets no
a11y attributes yet (a11y.rs:174-184 even warns if more than the root node exists).
Upstreaming public read access to the a11y tree would benefit both gpui-driver and
accessibility tooling — tracked as a future direction, not v0.

## Decision

`tree` is built from gpui-driver's own registry:

- `DriverExt::driver_id("...")` wraps any element in a `DriverNode` (a delegating
  `Element` impl). During `prepaint` it records the element's bounds and registers a real
  GPUI `Hitbox` (`window.insert_hitbox`), keyed per window, with parent/child structure
  recovered from a prepaint-time stack (children prepaint inside their parent's
  `prepaint` call).
- The hitbox means `click` can use GPUI's genuine hit-test results for occlusion checks
  instead of geometric guessing.
- Consequence (vs. the inspector dream): only annotated elements appear in `tree`, and
  `text` is best-effort. This matches the design's stated fallback (§3, §7.2).

## What `dispatch_event` research settled along the way

- `Window::dispatch_event(PlatformInput, cx)` (window.rs:4498) and
  `Window::dispatch_keystroke` (window.rs:4456) are public — synthetic input needs **no
  patch** and goes through the real hit-test + handler path.
- `Window::draw(cx)` (window.rs:2603) is public — the driver can force a fresh frame
  before reading the registry or capturing, even when the platform frame loop is parked
  (e.g. minimized windows).
