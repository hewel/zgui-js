# zgui-runtime

The zgui frame pipeline: style, layout, paint and draw in a window.

This crate is part of [zgui](https://github.com/zortax/zgui), a native Rust UI framework.
It is an internal part of the framework. Applications depend on
[`zgui`](https://crates.io/crates/zgui).

## Stylesheet revision evidence

`Window::style_revision()` counts applied application/window and named SDK stylesheet operations.
Named replacements retain their cascade slot; byte-identical replacements and absent removals
do not advance it. Install/remove commands apply in queue order, not hash-map order.
`CompletedFrame::style_revision` identifies the last restyle's consumed revision, which can trail
the live revision after late callbacks. It does not assert that pixels were presented.
Freshness also requires `!Window::has_pending_stylesheets()` so queued-only theme changes cannot
reuse evidence from the previous sheet set.
