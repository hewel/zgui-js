# Native dependency patch

`zgui-runtime/` is copied from `zortax/zgui` revision `80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a`, with its Apache-2.0 LICENSE and NOTICE. The manifest is standalone; every sibling zgui dependency retains that revision. The root Cargo patch selects this copy for the entire dependency graph. Upstream source and tests are retained.

## Disabled activation

Upstream `Router::key` keeps the focused node after it becomes disabled. Its default activation is computed before event listeners, and `Window::carry_out_default` previously synthesized Click without rechecking current state. Real Wayland input reproduced Enter/Space activation of a disabled, already-focused control.

The activation patch checks the live node's disabled state immediately before `FrameworkDefault::Activate` executes. This also prevents activation when a key listener disables the control during that same event. It does not synthesize JS events, alter keyboard repeat policy, or replace native controls.

Regression gate: `ZGUI_PLATFORM=wayland cargo test -p native-host native_tests::native_button_actions -- --ignored --exact --nocapture`. The gate includes cached focus, same-event disabling, pointer/Enter/Space activation and native cancellation.

## Occluded frame callbacks

Upstream `Window::run_frame_callbacks` drained callbacks even in a frame run for hidden-window housekeeping, despite the public `ViewHost` contract promising retention until visibility resumes. A controlled occlusion gate on the real Wayland runtime reproduced rAF and its Promise layout work running while `Window::is_occluded()` remained true.

The frame-callback patch returns before draining when the window is occluded. Native timers, owner checkpoints and task completions continue; unocclusion releases the preserved callbacks before layout.

Regression gate: `ZGUI_PLATFORM=wayland cargo test -p native-host async_tests::native_async_delivery -- --ignored --exact --nocapture`. The gate moves only its PID/app-ID-verified window to an inactive niri workspace by stable IPC ID, waits for settled native occlusion, then restores it. Timers and Rust completion must finish while rAF remains pending; the first visible fired frame must include the Promise layout change.

## Cancelled timer retention

Upstream cancellation kept heap entries and closures until their original deadlines. Repeatedly creating and cancelling long timers therefore bypassed the host's live-callback limit; `peek_for` only filtered tombstones without reclaiming them.

Cancellation now removes the entry in place with `BinaryHeap::retain`, immediately releasing its closure. Obsolete tombstones are removed entirely. Cancellation is O(n) in currently scheduled native timers, rather than retaining unbounded historical cancellations; a more complex indexed queue is not justified without measurement. Already-selected native callback batches retain their original semantics; the host's ready queue separately enforces JS cancellation before invocation.

Regression checks in `timer::tests` cover 4,096 long-deadline cancellations without polling, immediate captured-resource release, repeated cancellation, and cancellation of a rearmed repeat.

## Primary layer configuration

`Runtime::primary_surface_attributes_mut` exposes the retained primary surface
specification only while that surface is absent. The native host uses it before
opening a layer and after output loss, without replacing the standard graphics,
text, or document factories. It refuses mutation while open or permanently closed.

`zgui-platform-wayland/` is also copied from the same pinned revision, with its
LICENSE and NOTICE and standalone manifest. Four focused protocol changes apply:

- The layer namespace comes from `SurfaceAttributes.application_id`, retaining
  `zgui` only when no application ID was supplied. This permits compositor rules
  to identify the native panel.
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
the layer's extent. Out-of-surface layer-parent popups remain unsupported;
see the [panel policy](../apps/examples/panel/README.md#popups-and-menus).

Regression gate: build the panel, then run
`ZGUI_PLATFORM=wayland cargo test -p native-host shell::proof::real_panel_layer -- --ignored --exact --nocapture --test-threads=1`.
It opens the configured real layer, requires real niri data to reach native TSX
rows without input, rejects keyboard focus, and checks zero resources at shutdown.
This gate reproduced the initial-output race before the roundtrip fix. With
`ZGUI_SHELL_LIFECYCLE=1`, it waits for an externally authorized output off/on
sequence and requires a fresh live generation; this also reproduced the stale
output-cache bug before its fix. Visual inspection remains a separate check.

Remove each local patch only when an upgraded pinned upstream revision implements its contract and passes the corresponding gate. Do not silently drop patches on dependency updates.
