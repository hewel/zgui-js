# 01: Internalize Bridge turn disposal, HostPort queueing, and rejection quiescence

**What to build:** Internalize **Bridge turn** enforcement, re-entrant notification queueing, fault disposal, and promise-rejection quiescence across the JavaScript runtime, native bridge, and **Session** modules so error getters never gain **Mount region** mutation authority, failed turns drop stylesheets and leases completely, and external callers/tests interact through one narrower seam without `DeferredEvent` or two-phase dispatcher setup.

**Blocked by:** None (can start immediately)

**Status:** resolved

- [x] `JsRuntime` exposes a turn-scoped exclusive borrow handle (`JsTurn<'_>`) that omits `drain_rejections`, making calling `drain_rejections` inside a **Bridge turn** a compile-time error while preserving normal `with_context`, `with_module`, `with_cleanup_context`, `execute_job`, and `evaluate_script` entries.
- [x] `JsRuntime::drain_rejections(RejectionPolicy)` internally cancels same-batch `(false -> true)` rejected-then-caught promises before formatting or slot eviction, runs getter quiescence under one clamped `diagnostic_timeout` within `max_tracked_rejections + 1` passes, and reuses cached diagnostics for cross-batch `handled: true` updates under `RejectionPolicy::Log`.
- [x] `UiBridge` deletes `DeferredEvent`, `EventDispatcher`, `set_dispatcher`, `set_geometry_dispatcher`, `invoke_event`, `invoke_deferred`, and `invoke_geometry` from its public interface and accepts a concrete `HostPort` (`queued`, `live`, `synchronous`) at construction; `Session` wires `HostPort::live` in one phase via `Rc::new_cyclic`.
- [x] Idle native input events and idle intra-frame geometry observations enter synchronously through `HostPort` (preserving synchronous `EventCx` cancellation and first-paint `Popper` relayout before paint), while re-entrant or engine-busy notifications buffer into `UiBridge`'s internal 256-entry FIFO and wake the host for checkpoint drainage.
- [x] Any `Err` returned by `UiBridge::run_turn`, external synchronous dispatch, or >256 queued notifications invokes full `UiBridge::dispose()`, clearing stylesheets, listeners, dismiss registrations, focus traps, scroll locks, geometry leases, and **Mount region** nodes identically to normal disposal.
- [x] All standalone bridge tests cross `UiBridge`'s public interface using `HostPort::{queued, synchronous}` (removing private `events` source recompilation), and `Session` checkpoint/shutdown tests pass without reaching into private bridge/rejection helpers.
- [x] Workspace Rust checks (`pnpm test:rust`) and preview geometry/menu/runtime integration suites pass after the final edit.
