# Native dependency patch

`zgui-runtime/` is copied from `zortax/zgui` revision `80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a`, with its Apache-2.0 LICENSE and NOTICE. The manifest is standalone; every sibling zgui dependency retains that revision. The root Cargo patch selects this copy for the entire dependency graph. Upstream source and tests are retained.

## Disabled activation

Upstream `Router::key` keeps the focused node after it becomes disabled. Its default activation is computed before event listeners, and `Window::carry_out_default` previously synthesized Click without rechecking current state. Real Wayland input reproduced Enter/Space activation of a disabled, already-focused control.

The activation patch checks the live node's disabled state immediately before `FrameworkDefault::Activate` executes. This also prevents activation when a key listener disables the control during that same event. It does not synthesize JS events, alter keyboard repeat policy, or replace native controls.

Regression gate: `ZGUI_PLATFORM=wayland cargo test -p native-host native_tests::native_button_actions -- --ignored --exact --nocapture`. The gate includes cached focus, same-event disabling, pointer/Enter/Space activation and native cancellation.

## Occluded frame callbacks

Upstream `Window::run_frame_callbacks` drained callbacks even in a frame run for hidden-window housekeeping, despite the public `ViewHost` contract promising retention until visibility resumes. A controlled occlusion gate on the real Wayland runtime reproduced rAF and its Promise layout work running while `Window::is_occluded()` remained true.

The frame-callback patch returns before draining when the window is occluded. Native timers, owner checkpoints and task completions continue; unocclusion releases the preserved callbacks before layout.

Regression gate: `ZGUI_PLATFORM=wayland cargo test -p native-host async_tests::native_async_delivery -- --ignored --exact --nocapture`. Startup compositor visibility is consumed before controlled occlusion, so launch events cannot invalidate the scenario.

## Cancelled timer retention

Upstream cancellation kept heap entries and closures until their original deadlines. Repeatedly creating and cancelling long timers therefore bypassed the host's live-callback limit; `peek_for` only filtered tombstones without reclaiming them.

Cancellation now removes the entry in place with `BinaryHeap::retain`, immediately releasing its closure. Obsolete tombstones are removed entirely. Cancellation is O(n) in currently scheduled native timers, rather than retaining unbounded historical cancellations; a more complex indexed queue is not justified without measurement. Already-selected native callback batches retain their original semantics; the host's ready queue separately enforces JS cancellation before invocation.

Regression checks in `timer::tests` cover 4,096 long-deadline cancellations without polling, immediate captured-resource release, repeated cancellation, and cancellation of a rearmed repeat.

Remove each local patch only when an upgraded pinned upstream revision implements its contract and passes the corresponding gate. Do not silently drop patches on dependency updates.
