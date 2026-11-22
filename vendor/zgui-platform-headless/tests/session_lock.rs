//! Session locks at the platform and offscreen presentation seams.

use std::sync::Arc;

use zgui_geom::{CssPx, DevicePx, Point, Size};
use zgui_platform::{
    MonitorInfo, PlatformCapabilities, PlatformCx, PlatformError, SessionLockState, Surface,
    SurfaceAttributes, SurfaceRole, WakeReason,
};
use zgui_platform_headless::{Headless, OffscreenSurface, SessionLockDecision};

fn monitor(name: &str, width: f32, scale: f64) -> MonitorInfo {
    MonitorInfo::new(
        Point::new(DevicePx(0.0), DevicePx(0.0)),
        Size::new(DevicePx(width), DevicePx(6.0)),
        scale,
    )
    .with_name(name)
}

fn lock_platform() -> Headless {
    let mut capabilities = PlatformCapabilities::none();
    capabilities.session_lock = true;
    Headless::new().with_capabilities(capabilities)
}

fn lock_surface(output: &str) -> SurfaceAttributes {
    let mut attributes =
        SurfaceAttributes::new("lock UI").with_size(Size::new(CssPx(1.0), CssPx(1.0)));
    attributes.role = SurfaceRole::Lock(output.into());
    attributes
}

fn fallback_for(platform: &Headless, output: &str) -> Arc<OffscreenSurface> {
    let target = zgui_platform::MonitorTarget::from(output);
    platform
        .offscreens()
        .into_iter()
        .find(|surface| matches!(surface.requested_attributes().role, SurfaceRole::Lock(ref t) if t == &target))
        .expect("the output has a platform-owned lock role")
}

#[test]
fn headless_lock_is_opt_in_and_consumes_one_refusal() {
    let unsupported = Headless::new().with_monitors(vec![monitor("A", 8.0, 1.0)]);
    assert!(
        matches!(
            unsupported.request_session_lock(),
            Err(PlatformError::Unsupported(_))
        ),
        "the default backend cannot acquire a session lock"
    );
    assert!(
        matches!(
            unsupported.create_surface(&lock_surface("A")),
            Err(PlatformError::Unsupported(_))
        ),
        "the default backend cannot create lock content"
    );
    assert_eq!(
        unsupported.session_lock_state(),
        SessionLockState::Unlocked,
        "an unsupported request leaves the session unlocked"
    );
    assert!(unsupported.offscreens().is_empty(), "no lock role was created");

    let platform = lock_platform().with_monitors(vec![monitor("A", 8.0, 1.0)]);
    assert!(
        platform.create_surface(&lock_surface("A")).is_err(),
        "content cannot claim an output before acquisition"
    );
    platform.set_session_lock_decision(SessionLockDecision::Refuse);
    platform
        .request_session_lock()
        .expect("a refusal is a compositor decision");
    assert_eq!(
        platform.session_lock_state(),
        SessionLockState::Refused,
        "the simulated compositor refuses the next request"
    );
    assert!(
        platform.session_lock_snapshot().covered_outputs.is_empty(),
        "refusal creates no lock coverage"
    );
    assert!(platform.try_unlock_session().is_err(), "a refused lock cannot be unlocked");
    assert_eq!(
        platform.session_lock_snapshot().unlock_count,
        0,
        "a rejected unlock must not count as acknowledged"
    );
    assert!(
        matches!(platform.drain_wakes().as_slice(), [WakeReason::SessionLockFinished]),
        "refusal wakes the app to observe the decision"
    );

    platform
        .request_session_lock()
        .expect("the next decision defaults to lock");
    assert_eq!(
        platform.session_lock_state(),
        SessionLockState::Locked,
        "refusal is consumed by only one request"
    );
    assert!(
        matches!(platform.drain_wakes().as_slice(), [WakeReason::SessionLocked]),
        "acquisition wakes the app to observe the decision"
    );
    assert_eq!(
        platform.session_lock_snapshot().fallback_outputs,
        ["A"],
        "the newly acquired lock covers the monitor before a UI claims it"
    );
    let id = fallback_for(&platform, "A").id();
    platform
        .request_session_lock()
        .expect("requesting again is harmless");
    assert_eq!(fallback_for(&platform, "A").id(), id, "repeated acquisition reuses the role");
    assert!(platform.drain_wakes().is_empty(), "idempotent acquisition does not wake the app");
}

#[test]
fn headless_covers_outputs_until_explicit_unlock_even_without_a_ui() {
    let platform = lock_platform().with_monitors(vec![
        monitor("A", 8.0, 1.0),
        monitor("B", 12.0, 2.0),
    ]);
    let ordinary = platform
        .create_surface(&SurfaceAttributes::new("ordinary"))
        .expect("open a surface unrelated to the lock");
    platform.request_session_lock().expect("acquire the simulated lock");
    let a = fallback_for(&platform, "A");
    let b = fallback_for(&platform, "B");
    assert!(a.is_visible(), "fallback is visible without application content");
    let frame = a
        .lock_fallback_frame()
        .expect("published even without an app renderer");
    assert_eq!(frame.pixel(0, 0), Some([0, 0, 0, 255]), "origin is opaque black");
    assert_eq!(frame.pixel(7, 5), Some([0, 0, 0, 255]), "last pixel is opaque black");
    assert_eq!(frame.pixel(8, 5), None, "fallback uses the exact output width");
    assert_eq!(frame.pixel(7, 6), None, "fallback uses the exact output height");
    assert_eq!(
        platform.session_lock_snapshot().fallback_outputs,
        ["A", "B"],
        "each output starts covered by fallback"
    );
    assert!(
        platform.create_surface(&lock_surface("missing")).is_err(),
        "only an existing output can be claimed"
    );

    let claim = platform
        .create_surface(&lock_surface("B"))
        .expect("claim an existing role");
    assert_eq!(claim.id(), b.id(), "the claim uses B's existing lock surface");
    assert_eq!(
        claim.size(),
        Size::new(DevicePx(12.0), DevicePx(6.0)),
        "claim dimensions come from output B, not the requested size"
    );
    assert_eq!(claim.scale_factor(), 2.0, "the claim inherits B's scale");
    assert!(
        claim.request_size(Size::new(CssPx(1.0), CssPx(1.0))).is_none(),
        "application size requests cannot shrink the compositor lock"
    );
    assert!(
        platform.create_surface(&lock_surface("B")).is_err(),
        "a plain duplicate claim cannot replace live content"
    );
    assert!(
        b.lock_fallback_frame().is_some(),
        "claiming does not present app pixels"
    );
    claim.set_visible(true);
    assert!(b.lock_fallback_frame().is_none(), "visible content takes over B");
    assert_eq!(
        platform.session_lock_snapshot().covered_outputs,
        ["A", "B"],
        "app content changes the presentation, not which outputs are covered"
    );
    assert_eq!(
        platform.session_lock_snapshot().fallback_outputs,
        ["A"],
        "only the unclaimed output keeps showing fallback"
    );
    platform.destroy_surface(claim.id());
    assert_eq!(
        platform.session_lock_state(),
        SessionLockState::Locked,
        "destroying app content cannot unlock the platform"
    );
    assert_eq!(platform.session_lock_snapshot().unlock_count, 0, "teardown sends no unlock");
    assert_eq!(
        platform.session_lock_snapshot().fallback_outputs,
        ["A", "B"],
        "destroying the claim restores fallback on B"
    );
    assert!(b.is_visible(), "the underlying role stays visible after teardown");
    assert_eq!(
        b.lock_fallback_frame().and_then(|frame| frame.pixel(11, 5)),
        Some([0, 0, 0, 255]),
        "restored fallback extends to the far corner of B"
    );

    platform.set_monitors(vec![monitor("C", 10.0, 1.0), monitor("B", 16.0, 2.0)]);
    assert!(platform.offscreen(a.id()).is_none(), "unplugging A removes its role");
    assert!(!a.is_visible(), "a retained handle cannot keep a removed role visible");
    assert_eq!(fallback_for(&platform, "B").id(), b.id(), "resizing B reuses its role");
    assert_eq!(
        b.lock_fallback_frame().and_then(|frame| frame.pixel(15, 5)),
        Some([0, 0, 0, 255]),
        "opaque fallback grows to cover the resized output"
    );
    assert_eq!(
        platform.session_lock_snapshot().fallback_outputs,
        ["B", "C"],
        "the resized and newly attached outputs both have fallback"
    );
    let c = platform.create_surface(&lock_surface("C")).expect("claim the attached output");
    c.set_visible(true);

    platform.drain_wakes();
    platform
        .try_unlock_session()
        .expect("the deterministic compositor acknowledged unlock");
    let snapshot = platform.session_lock_snapshot();
    assert_eq!(snapshot.state, SessionLockState::Unlocked, "explicit unlock is acknowledged");
    assert_eq!(snapshot.unlock_count, 1, "exactly one explicit unlock was sent");
    assert!(snapshot.covered_outputs.is_empty(), "unlock retires all lock coverage");
    assert!(snapshot.fallback_outputs.is_empty(), "unlock retires all black fallbacks");
    assert!(platform.offscreen(b.id()).is_none(), "unlock removes B's fallback role");
    assert!(platform.offscreen(c.id()).is_none(), "unlock removes C's claimed role");
    assert!(platform.offscreen(ordinary.id()).is_some(), "unlock preserves unrelated surfaces");
    assert!(
        matches!(platform.drain_wakes().as_slice(), [WakeReason::SessionLockFinished]),
        "unlock wakes the application to observe acknowledgement"
    );
    assert!(platform.try_unlock_session().is_err(), "the acknowledged lock cannot be unlocked twice");
    assert_eq!(
        platform.session_lock_snapshot().unlock_count,
        1,
        "the rejected second unlock never reaches the compositor"
    );
}
