#!/usr/bin/env bash
# Regenerates vendor/gpui_windows from upstream zed at a given rev and re-applies the
# driver patch (vendor/patches/gpui_windows-driver.patch). This is the supported way to
# bump the pinned gpui rev — see AGENTS.md ("Bumping the pinned gpui rev").
#
# Usage:  tools/update-vendor.sh <40-char zed commit sha>
# (On Windows, run from Git Bash — it ships with Git for Windows.)
#
# What it does:
#   1. fetches zed's crates/gpui_windows at <rev> (shallow, blobless, sparse)
#   2. replaces vendor/gpui_windows contents (except Cargo.toml, which is our own
#      rewrite of upstream's workspace-inherited manifest) with the fresh upstream copy
#   3. applies vendor/patches/gpui_windows-driver.patch
#   4. substitutes the old rev for <rev> in vendor/gpui_windows/Cargo.toml and the
#      workspace Cargo.toml
#   5. warns when upstream's own Cargo.toml changed since the last bump (then
#      vendor/gpui_windows/Cargo.toml must be reconciled by hand)
#
# If step 3 fails, upstream changed the code our patch touches: re-derive the
# two-method diff by hand against the fresh sources, regenerate the patch file
# (git diff between pristine and patched src trees), and re-run.

set -euo pipefail

rev="${1:-}"
if ! [[ "$rev" =~ ^[0-9a-f]{40}$ ]]; then
    echo "usage: tools/update-vendor.sh <40-char zed commit sha>" >&2
    exit 2
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
vendor_dir="$repo_root/vendor/gpui_windows"
patch_file="$repo_root/vendor/patches/gpui_windows-driver.patch"
baseline_manifest="$repo_root/vendor/patches/upstream-Cargo.toml"

old_rev="$(grep -oE 'rev = "[0-9a-f]{40}"' "$vendor_dir/Cargo.toml" | head -1 | grep -oE '[0-9a-f]{40}')"
[ -n "$old_rev" ] || { echo "error: no pinned rev found in vendor/gpui_windows/Cargo.toml" >&2; exit 1; }
echo "Current rev: $old_rev"
echo "Target rev:  $rev"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

# 1. Fetch exactly the target commit, trees-only, sparse to the one crate.
git -C "$tmp" init -q
git -C "$tmp" remote add origin https://github.com/zed-industries/zed
git -C "$tmp" sparse-checkout set crates/gpui_windows
git -C "$tmp" fetch -q --depth 1 --filter=blob:none origin "$rev" \
    || { echo "error: fetch of $rev failed — does the rev exist upstream?" >&2; exit 1; }
git -C "$tmp" checkout -q FETCH_HEAD
upstream="$tmp/crates/gpui_windows"

# 2. Replace vendor contents, preserving our Cargo.toml rewrite.
find "$vendor_dir" -mindepth 1 -maxdepth 1 ! -name Cargo.toml -exec rm -rf {} +
(cd "$upstream" && find . -mindepth 1 -maxdepth 1 ! -name Cargo.toml -exec cp -r {} "$vendor_dir/" \;)

# 3. Re-apply the driver patch.
if ! git -C "$repo_root" apply --directory=vendor/gpui_windows "$patch_file"; then
    echo "error: patch no longer applies — upstream changed window.rs or" >&2
    echo "directx_renderer.rs. Re-derive the patch (see header of this script)." >&2
    exit 1
fi

# 4. Point the git deps at the new rev (vendored manifest + workspace manifest).
if [ "$old_rev" != "$rev" ]; then
    for f in "$vendor_dir/Cargo.toml" "$repo_root/Cargo.toml"; do
        sed -i "s/$old_rev/$rev/g" "$f"
    done
fi

# 5. Detect upstream manifest drift (new/removed deps need a manual reconcile).
if ! cmp -s "$upstream/Cargo.toml" "$baseline_manifest"; then
    echo "WARNING: upstream gpui_windows/Cargo.toml changed since the last bump." >&2
    echo "Reconcile vendor/gpui_windows/Cargo.toml by hand (git diff" >&2
    echo "vendor/patches/upstream-Cargo.toml shows what changed), then commit." >&2
    cp "$upstream/Cargo.toml" "$baseline_manifest"
else
    echo "Upstream manifest unchanged — no manual reconcile needed."
fi

echo
echo "Done. Now verify:"
echo "  cargo build -p demo-app"
echo "  cargo test"
echo "and run the demo-app screenshot smoke test (method should be 'renderer')."
