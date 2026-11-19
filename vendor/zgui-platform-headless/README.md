# zgui-platform-headless

Headless zgui platform with a test clock and scripted input.

This crate is part of [zgui](https://github.com/zortax/zgui), a native Rust UI framework.
Applications use it together with [`zgui`](https://crates.io/crates/zgui).

## Session-lock fixtures

Opt in with `PlatformCapabilities::session_lock` and supply named monitors. The
offscreen compositor immediately accepts a request unless the next decision was
set to `SessionLockDecision::Refuse`. It shows opaque black on every lock output
before application content is shown and again whenever that output's claim is
released. Unplugging an output removes its role; only explicit unlock removes
all roles and increments the unlock count.

`Headless::session_lock_snapshot` reports the state and sorted covered and
fallback output names. `OffscreenSurface::lock_fallback_frame` exposes the
fallback pixels without an application renderer or GPU readback.
