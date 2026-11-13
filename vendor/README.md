# Native dependency patch

`zgui-runtime/` is copied from `zortax/zgui` revision `80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a`, with its Apache-2.0 LICENSE and NOTICE. The manifest is standalone; every sibling zgui dependency retains that revision. The root Cargo patch selects this copy for the entire dependency graph. Upstream source and tests are retained.

## Disabled activation

Upstream `Router::key` keeps the focused node after it becomes disabled. Its default activation is computed before event listeners, and `Window::carry_out_default` previously synthesized Click without rechecking current state. Real Wayland input reproduced Enter/Space activation of a disabled, already-focused control.

The only production source change checks the live node's disabled state immediately before `FrameworkDefault::Activate` executes. This also prevents activation when a key listener disables the control during that same event. It does not synthesize JS events, alter keyboard repeat policy, or replace native controls.

Regression gate: `ZGUI_PLATFORM=wayland cargo test -p native-host native_tests::native_button_actions -- --ignored --exact --nocapture`. The gate includes cached focus, same-event disabling, pointer/Enter/Space activation and native cancellation.

Remove this local patch when an upgraded pinned upstream revision provides the same execution-time validation and passes the gate. Do not silently drop it on dependency updates.
