---
name: gpui-driver
description: Drive and visually verify a running GPUI desktop app via the gpui-driver CLI — inspect the element tree, click by stable id, type, and screenshot, all without window focus and even while the session is locked. Use when asked to test, click through, verify, or debug the UI of a GPUI app instrumented with the gpui-driver crate.
---

# Driving a GPUI app with gpui-driver

You are driving a *real, running* GPUI application through an in-process automation
server. All interaction goes through stable element ids; screenshots are your eyes.

## Requirements

- The app must be built with its `driver` feature (which calls `gpui_driver::init`).
- The `gpui-driver` CLI must be on PATH (or use the path to the built binary).
- The app must be running. Window focus is NOT required; minimized/occluded/locked all work.

## The core loop

1. **Find the app:** `gpui-driver list` — pick the app (use `--app <name>` on every
   later call if more than one is running; with a single app you can omit it).
2. **See what's there:** `gpui-driver tree --interactive-only` before acting.
   Never guess ids; read them from the tree.
3. **Act by id:** `gpui-driver click <id>`. **Never click coordinates** — there is no
   coordinate API, by design. If you need something that has no id, ask for a
   `driver_id` annotation in the app source instead.
4. **Settle:** `gpui-driver wait-idle` after every action (resolves when the rendered
   output stops changing).
5. **Verify with your eyes:** `gpui-driver screenshot -o shot.png`, then *look at the
   image* and judge the result. The screenshot is the assertion. Check the reported
   capture `method`: `renderer` is always trustworthy; `printwindow` (fallback for
   apps built without the gpui_windows patch — a stderr warning appears) is only
   trustworthy while the window is visible on screen. With `printwindow`, do not
   treat screenshots taken while occluded/minimized/locked as evidence.
6. Repeat from 2 — re-fetch the tree after every state change; don't click blind.

## Commands

```
gpui-driver list                          discovered apps + liveness
gpui-driver info                          app/protocol versions
gpui-driver windows                       windows with ids, titles, bounds
gpui-driver tree [--interactive-only]     element tree (id, kind, text, bounds)
gpui-driver query --text-contains "Save"  find elements without the full tree
gpui-driver query --id-contains save
gpui-driver click <id> [--button right] [--modifier ctrl]
gpui-driver focus <id>                    focus via synthetic click
gpui-driver type "hello world"            types into the focused element
gpui-driver key ctrl-s                    GPUI keystroke syntax (enter, backspace, ...)
gpui-driver scroll <id> --delta-y -120
gpui-driver wait-idle [--timeout 5000] [--quiet 150]
gpui-driver screenshot -o shot.png
```

All commands accept `--app <name>` / `--pid <pid>` and `--json` (JSON is automatic when
stdout is piped — you get machine output for free).

## Exit codes

| code | meaning | what you should do |
|---|---|---|
| 0 | ok | continue |
| 2 | element/window not found, or element occluded | re-fetch the tree; see below |
| 3 | timeout (`wait-idle`) | UI kept animating; screenshot anyway and look |
| 4 | no instrumented app found | start the app / check the `driver` feature / disambiguate with `--app` |
| 5 | protocol or auth error | stale discovery state: restart the app; check versions with `info` |

## When things go wrong

- **`element_not_found` (exit 2):** the id may have been renamed or the element isn't
  currently rendered. Re-fetch the tree first. If it's gone from the tree but you
  expected it, grep the app source for `driver_id` to find the current ids before
  retrying.
- **Occlusion error on click (exit 2 with "not hit-testable"):** something covers the
  element (modal, menu). That is real information about the UI state — handle the
  overlay first (it's in the tree).
- **Tree doesn't match expectations:** never act on a stale mental model; re-fetch.
- **`type` has no effect:** focus the target first (`focus <id>`), then type.

## Anti-patterns

- Clicking coordinates (impossible — and don't ask for it).
- Acting without a fresh `tree` after the UI changed.
- Asserting success without looking at a screenshot.
- Busy-looping screenshots instead of `wait-idle` → screenshot once.
