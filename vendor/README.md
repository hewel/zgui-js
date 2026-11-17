# Native dependency patch

`zgui-runtime/` is copied from `zortax/zgui` revision `80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a`, with its Apache-2.0 LICENSE and NOTICE. The manifest is standalone; every sibling zgui dependency retains that revision. The root Cargo patch selects this copy for the entire dependency graph. Upstream source and tests are retained.

## Disabled activation

Upstream `Router::key` keeps the focused node after it becomes disabled. Its default activation is computed before event listeners, and `Window::carry_out_default` previously synthesized Click without rechecking current state. Real Wayland input reproduced Enter/Space activation of a disabled, already-focused control.

The activation patch checks the live node's disabled state immediately before `FrameworkDefault::Activate` executes. This also prevents activation when a key listener disables the control during that same event. It does not synthesize JS events, alter keyboard repeat policy, or replace native controls.

Regression gate: `ZGUI_PLATFORM=wayland cargo test -p native-host native_tests::native_button_actions -- --ignored --exact --nocapture`. The gate includes cached focus, same-event disabling, pointer/Enter/Space activation and native cancellation.

## Occluded frame callbacks

Upstream `Window::run_frame_callbacks` drained callbacks even in a frame run for hidden-window housekeeping, despite the public `ViewHost` contract promising retention until visibility resumes. A controlled occlusion gate on the real Wayland runtime reproduced rAF and its Promise layout work running while `Window::is_occluded()` remained true.

The frame-callback patch returns before draining when the window is occluded. Native timers, owner checkpoints and task completions continue; unocclusion releases the preserved callbacks before layout.

Historical verification used the now-retired `async_tests::native_async_delivery`
gate to move only its PID/app-ID-verified window to an inactive niri workspace,
wait for native occlusion, then restore it. Timers and Rust completion finished
while rAF remained pending; the first visible frame included the Promise layout
change. That compositor-specific gate is no longer shipped. The patch remains a
generic upstream runtime correctness fix, not a niri support requirement.

## Cancelled timer retention

Upstream cancellation kept heap entries and closures until their original deadlines. Repeatedly creating and cancelling long timers therefore bypassed the host's live-callback limit; `peek_for` only filtered tombstones without reclaiming them.

Cancellation now removes the entry in place with `BinaryHeap::retain`, immediately releasing its closure. Obsolete tombstones are removed entirely. Cancellation is O(n) in currently scheduled native timers, rather than retaining unbounded historical cancellations; a more complex indexed queue is not justified without measurement. Already-selected native callback batches retain their original semantics; the host's ready queue separately enforces JS cancellation before invocation.

Regression checks in `timer::tests` cover 4,096 long-deadline cancellations without polling, immediate captured-resource release, repeated cancellation, and cancellation of a rearmed repeat.

## Repeated IME preedit

Upstream `Editor::preedit` returns a splice even when the provisional text has
not changed. The runtime treated every splice as a value update, so repeated
preedit notifications (including cursor-only updates from an IME such as fcitx5)
could repeatedly dispatch `input` and feed the same value back through a controlled
binding.

The runtime compares incoming preedit bytes with the active composition without
allocating a text copy. Identical text updates only the selection; it does not
rewrite document text or dispatch another `input`. Changed provisional text,
cancellation that removes provisional text, and final commits still announce
their values. This is not a commit-only input policy.

Regression gate:
`cargo test --manifest-path vendor/zgui-runtime/Cargo.toml --test controlled --test value --test editing`.
The headless native-window tests cover repeated and cursor-only preedit, UTF-8
and multiline text, controlled echoes, identical-text commit, undo, cancellation,
and resumed typing. They do not automate a live fcitx5 session.

## IME positioning across caret blink

`Window::caret_rect` previously read the painted highlight rectangles. While the
caret was blinked off, it returned no rectangle (or a selection band), so IME
positioning fell back to the field box instead of the insertion point. Alternating
blink phases therefore sent different cursor rectangles to Wayland even without
typing, allowing candidate-window repositioning and preedit replies to feed back.

The caret plan now retains logical insertion geometry independently of its painted
marks. IME positioning uses only that geometry, transformed with the current line
fragment; hidden phases still paint no caret. Actual cursor movement continues to
update the candidate position.

Regression gate:
`cargo test --manifest-path vendor/zgui-runtime/Cargo.toml --test controlled --test caret --test editing --test secure`.
The native headless regression advances four blink phases during composition,
with and without a selected preedit range, and checks stable surface IME geometry,
continued visual blinking, and subsequent cursor movement. Live fcitx5 popup
rendering remains a manual verification boundary.

## Primary layer configuration

`Runtime::primary_surface_attributes_mut` exposes the retained primary surface
specification only while that surface is absent. The native host uses it before
opening a layer and after output loss, without replacing the standard graphics,
text, or document factories. It refuses mutation while open or permanently closed.

`zgui-platform-wayland/` is also copied from the same pinned revision, with its
LICENSE and NOTICE and standalone manifest. Four focused protocol changes apply:

- The layer namespace comes from `SurfaceAttributes.application_id`, retaining
  `zgui` only when no application ID was supplied. This permits compositor rules
  to identify a native layer surface.
- Explicit zero width/height remains zero in `set_size`, as required to stretch
  between opposite anchors. Positive dimensions retain upstream rounding.
- One initial roundtrip completes bound output descriptions before
  `surfaces_available`. Without it, a connected named monitor was incorrectly
  reported as unavailable because its name had not reached the client yet.
- Output removal excludes the departing proxy when rebuilding monitor and scale
  inventory. SCTK invokes `output_destroyed` before removing that proxy from its
  own cache; reading the cache unchanged republished a disconnected monitor and
  prevented the host from suspending/recreating its layer.

The host validates stretch anchors and an explicitly named available monitor.
It always supplies a nonnegative exclusive zone: the pinned backend's `None`
emits protocol `-1`, which ignores other exclusive zones rather than reserving
the layer's extent. Out-of-surface layer-parent popups remain unsupported by
the pinned upstream backend; same-surface overlays do not establish that support.

Historical verification used the former panel example and the now-retired
`shell::proof::real_panel_layer` gate. It opened a real layer, observed niri
service delivery without input, rejected keyboard focus and checked resource
cleanup. It reproduced the initial-output race before the roundtrip fix; an
explicitly authorized output off/on run reproduced the stale output-cache bug
before its fix. The panel, niri service and compositor-specific gate are no
longer shipped. These generic layer-shell patches remain upstream wrappers:
platform and window-manager capabilities are determined by zgui, with no
compositor-specific package or abstraction in this library.

## Package image authority and capture readiness

Package images retain verified immutable bytes rather than reopening ambient
filesystem paths. `ImagePolicy::package_assets` preserves ordinary asynchronous
native publication; `package_only` additionally stages complete worker batches
in launch order for deterministic preview checkpoints. Neither path fabricates
decoded pixels or substitutes a separate image renderer.

`Window::replace_image_policy` is used after previous-owner shutdown and before
new mount. It retires the old completion domain, pending source writes, bindings,
intrinsics, shared image tiles and replay leases. Attribute-hook queue identity
and monotonic request/atlas identities survive, so same-path reloads cannot reuse
old package bytes or accept delayed old-generation completions.

Resource snapshots distinguish decode/attachment from actual atlas allocation,
successful texture upload and current residency. Native placed geometry, viewport,
ancestor clips and current opacity determine which images are required. Offscreen
images do not block capture, but become upload requirements when visible. Image
retirement runs after DOM reclamation and before completed-frame observation,
including paint-skipped frames; removed nodes cannot remain required resources.

Regression gates in the standalone runtime:

```sh
cargo test --manifest-path vendor/zgui-runtime/Cargo.toml --test image_policy --test images_end_to_end
cargo test --manifest-path vendor/zgui-runtime/Cargo.toml --lib delayed_
cargo test --manifest-path vendor/zgui-runtime/Cargo.toml --lib live_package_images_
```

These cover real atlas refusal, viewport/clip/opacity transitions, controlled
delayed decode publication, replacement/clear retirement and asynchronous
same-path package replacement. The host's real GPU and Wayland pixel evidence is
recorded in [preview verification](../docs/development/preview-verification.md#p3--shared-time-fixture-providers-and-native-resources).

## Geometry observation deliveries

`zgui-view-dom/` is copied from the same pinned revision `80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a`, with its Apache-2.0 LICENSE and NOTICE and standalone manifest; the same patch pattern selects it for the whole graph. Its delivery patch applies:

- Observation callbacks are invoked outside the registry's `RefCell` borrow, so a callback may register, cancel itself or cancel a sibling.
- Each registration receives its own fresh post-layout sample, including a first zero measurement and subscribers added inside another delivery; a prepared callback whose registration was cancelled is skipped.
- Duplicate values are suppressed per watched quantity through a cached last sample, so equal geometry is not replayed.
- A device-pixel-ratio change invalidates pending samples, so unchanged device geometry is redelivered under the new scale.
- The two-pass delivery budget and next-frame settlement bound are unchanged.

Regression gates:

```sh
cargo test --manifest-path vendor/zgui-view-dom/Cargo.toml --test observations
cargo test --manifest-path vendor/zgui-runtime/Cargo.toml --test geometry_observation --test observe_owed --test observed_place --test scale
```

Remove each local patch only when an upgraded pinned upstream revision implements its contract and passes the corresponding gate. Do not silently drop patches on dependency updates.

## Text replay content identity

`zgui-paint/` is copied from the same pinned revision `80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a`, with its Apache-2.0 LICENSE and NOTICE and standalone manifest; the same patch pattern selects it for the whole graph. Its replay signature patch applies:

- A `Line`/`TextRun` fragment's painted content signature carries the shaped paragraph key, because layout reclaims and reuses paragraph slots across the prepaint layout passes a single frame can run. Without it, a recycled slot replays the previous frame's glyph pixels for different shaped content.

Regression gates:

```sh
cargo test --manifest-path vendor/zgui-runtime/Cargo.toml --test geometry_observation --test damage --test shared_text
cargo test --manifest-path vendor/zgui-paint/Cargo.toml --lib walk::replay
ZGUI_KEEP_PREVIEW_OUTPUT=1 node --test tests/preview/geometry.test.mjs
```

Remove each local patch only when an upgraded pinned upstream revision implements its contract and passes the corresponding gate. Do not silently drop patches on dependency updates.

## Inherited colour on raw text items

`zgui-layout/` is copied from revision `80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a` with its LICENSE and NOTICE. Standalone manifests and the root patch retain the pinned dependency graph.

Raw text items in Flex and Grid own text-change marks while inheriting their parent's appearance. Fragment damage now includes the style generator's repaint/reshape marks for these items. Colour transitions repaint their glyphs without remounting or forcing full-surface damage.

Regression gates:

    cargo test -p zgui-runtime --test damage raw_
    ZGUI_REPAINT_WALK=off cargo test -p zgui-runtime --test damage raw_
    ZGUI_REPAINT_WALK=shadow cargo test -p zgui-runtime --test damage raw_
    ZGUI_KEEP_PREVIEW_OUTPUT=1 node --test tests/preview/disclosure.test.mjs

The recording renderer checks every incremental frame; the last frame alone can contain no glyphs when its damage is empty.

## Cross-parent node moves

`zgui-dom/` is copied from revision `80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a` with its LICENSE and NOTICE and a standalone manifest.

Cross-parent insertion marks the old parent for child-list rebuilding, layout, repaint, and accessibility updates. This retires the moved subtree's old layout and paint records even when its new parent is detached. Same-parent reorders retain their existing path.

Regression gates:

    cargo test -p zgui-runtime --test damage raw_portal_
    ZGUI_REPAINT_WALK=off cargo test -p zgui-runtime --test damage raw_portal_
    ZGUI_REPAINT_WALK=shadow cargo test -p zgui-runtime --test damage raw_portal_
    ZGUI_KEEP_PREVIEW_OUTPUT=1 node --test tests/preview/portal.test.mjs

## Focus restoration after native defaults

Automatic focus-trap restore commands retain their source scope and recheck focus when they
are carried out. A trap removed synchronously from a capture listener — before the pointer
framework default has applied — is restored against the focus that remains after the default:
a press that landed on a control outside the surface keeps the focus it was given, and
`preventDefault` on that press keeps the restore.

## Animation counts on generated content

`vendor/zgui-style` is pinned upstream zgui rev `80baa3c` with one change:
`Animations::running_on` sums the running animation/transition count across the
element's own set and its native `::before`/`::after` sets. Each motion counts
individually, finished motions kept for a lifecycle edge are not running, and
descendants never contribute. The lookup reads three fixed rows — no table
scan.

Regression gates:

    cargo test -p zgui-runtime --test anim_lifecycle
    cargo test -p zgui-style --lib driver::animations
    ZGUI_KEEP_PREVIEW_OUTPUT=1 node --test tests/preview/animation.test.mjs

## Frozen elastic scrolling

`zgui-scroll/` is copied from revision `80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a` with its LICENSE and NOTICE. Its standalone manifest and the root patch retain the pinned graph.

The runtime pauses elastic advancement on a frozen window root and excludes that paused edge from animation deadlines. It preserves displacement and resumes released edges on thaw. Other containers keep advancing. Existing root input filtering and motion cancellation remain in place.

Regression gates:

    cargo test --manifest-path vendor/zgui-scroll/Cargo.toml --lib
    cargo test --manifest-path vendor/zgui-runtime/Cargo.toml --test scroll_root --test scroll_touchpad --test scroll_owed
    ZGUI_KEEP_PREVIEW_OUTPUT=1 node --test tests/preview/scroll-lock.test.mjs
