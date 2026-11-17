# Deepen Core Runtime, Preview, and UI Seams

**Status:** ready-for-agent

## Problem Statement

Maintainers and agents face architectural friction across four hot seams:

1. **Bridge turn execution, re-entrant notifications, and rejection draining leak across runtime, bridge, and Session modules**: draining promise rejections runs inside an active **Bridge turn** during checkpoints (granting error getters **Mount region** mutation authority), failed **Bridge turn** exits leak installed stylesheets and host dispatchers, same-batch caught rejections emit uncoalesced pairs, and re-entrant notification plumbing forces `DeferredEvent` and two-phase dispatcher setup across the bridge seam (pushing tests to recompile internal event modules privately).
2. **Preview frame observation and Session retirement split ownership and diverge**: frame observation exposes 13 mutable fields and 3 fault slots across driver, host, action, and timeline modules while writing checkpoints directly to disk; meanwhile, surface-loss retirement and end-of-run retirement duplicate **Session** checks with three divergences—causing leaked **Session** resources at run end to pass with exit code `0`, inverting contract-vs-cleanup precedence on surface loss, and dropping source-mapped stacks during surface recreation while leaving the preview driver with zero in-process Rust tests.
3. **The in-memory JavaScript host test adapter is incomplete**: it implements only 17 of 23 host operations and omits bubbling focus/input dispatch, lease tracking, turn-end handle invalidation, and virtual time, forcing 15 build test suites to duplicate ~750 lines of host patches and fake clocks or invoke node callbacks past module interfaces.
4. **Overlay exit deferral, portal mounting, and hover coordination fragment across sibling UI modules**: presence exposes a 4-part kit requiring 6-step caller wiring, modal content duplicates ~80% of anchored surface wiring, hover modules duplicate trigger/Escape handling (emitting redundant `[false, true]` `onOpenChange` churn when refocusing an already-open controlled hover surface), and preview presence verification relies on an internal-package-import lint override.

## Solution

Deepen these four seams into high-leverage modules tested exclusively through their public interfaces:

1. Internalize **Bridge turn** fault disposal, compile-time turn borrowing, single-phase host port wiring, 256-entry re-entrant queueing, and rejection quiescence in the Rust JavaScript runtime and bridge.
2. Collapse preview frame observation, headless pipeline bootstrap, and **Session** retirement behind one idempotent **Session** retirement report, a shared RAII headless runner, and a witness-checked frame observer backed by a pure state machine and an internal persistence seam.
3. Complete the 23-operation in-memory host test adapter with active lease inspection, bubbling event/focus dispatch, strict **Bridge turn** guards, and deterministic virtual time, replacing all duplicated suite stubs and callback pokes.
4. Consolidate overlay exit deferral, same-turn QuickJS-stack-safe portal mounting, theme scoping, modal/anchored leases, and hover-intent trigger coordination inside one internal overlay-surface module while keeping every public UI export and module-map contract unchanged.

## User Stories

1. As a maintainer, I want `JsRuntime` to prevent calling promise-rejection draining inside an active **Bridge turn** at compile time, so that error-formatting getters can never mutate a **Mount region**.
2. As a maintainer, I want `JsRuntime` to cancel same-batch rejected-then-caught promises before formatting or capacity eviction and bound getter quiescence to one deadline, so that synchronously handled rejections consume zero tracking slots across both strict and logging modes.
3. As a maintainer, I want live **Session** logging to still receive cross-batch `handled: true` transitions from cached diagnostics when an earlier unhandled promise is later caught, so that stderr tracking remains accurate without re-running getters.
4. As a maintainer, I want `UiBridge` to bind its host collaboration port at construction via cyclic weak initialization, so that missing-dispatcher runtime errors become unrepresentable.
5. As a maintainer, I want `UiBridge` to route idle native input events and idle intra-frame geometry observations synchronously while queueing re-entrant or engine-busy notifications inside an internal 256-entry FIFO, so that synchronous `EventCx` cancellation and first-paint Popper relayout succeed while `DeferredEvent` disappears from the public interface.
6. As a maintainer, I want any failed **Bridge turn** or 256-entry queue overflow in `UiBridge` to invoke full disposal immediately, so that stylesheets, listeners, dismiss registrations, focus traps, scroll locks, and geometry observers never leak after a fault.
7. As a `ui-bridge` test author, I want standalone bridge tests to choose a pull-only or synchronous host port and cross the public `UiBridge` interface directly, so that tests never recompile internal source modules privately.
8. As a CI gate consumer, I want any created **Session** that retains timers, frame callbacks, host requests, or bridge resources after retirement to fail with `E_TEARDOWN` (exit code `9`) at both surface loss and run finish, so that resource leaks never pass silently.
9. As a preview author, I want **Session** retirement to prioritize unreported **Fixture runtime** contract violations over cleanup faults while preserving source-mapped cleanup diagnostics in both surface loss and run finish, so that root-cause attribution in the **Run ledger** stays accurate.
10. As a maintainer, I want `Session` retirement to snapshot pre-disposal counts, track whether a contract failure was already surfaced, and memoize one JSON-free report across repeated shutdown calls, so that **Session** teardown logic lives in one place.
11. As a preview maintainer, I want headless runtime setup, font wiring, `hold_clock(true)`, and ordered `zgui::reactive` shutdown shared by one RAII pipeline shared with calibration, so that early errors or test panics never poison the thread executor.
12. As a preview maintainer, I want frame observation hidden behind an immutable coherent-frame witness backed by a pure state machine and an internal checkpoint persistence seam, so that action/capture code cannot read stale frames and `Driver` tests run in memory in milliseconds.
13. As a UI module author, I want the in-memory host test adapter to implement all 23 `__zgui` operations, track active/disposed leases, bubble focus and input events with closure-captured cancellation, and enforce strict **Bridge turn** detached-node guards, so that build tests match production host invariants.
14. As a UI test maintainer, I want to delete the ~750 lines of duplicated host patches, fake clocks, and direct node callback invocations across build test suites, so that tests exercise only public UI module interfaces.
15. As a UI module author, I want one internal overlay-surface module to own exit deferral, same-turn QuickJS-stack-safe portal mounting, theme scoping, and modal/anchored lease lifetimes, so that modal and anchored overlays never drift apart.
16. As a UI module author, I want hover trigger, Escape dismissal, and intent handoff unified in the hover module, so that refocusing an already-open controlled hover overlay emits zero redundant `onOpenChange` transitions.
17. As a maintainer, I want build and GPU preview exit-deferral tests to verify public overlay modules directly and delete the internal-package-import lint exception, so that architectural gate rules have zero special-case exemptions.

## Implementation Decisions

### 1. Bridge Turn Borrowing, Rejection Quiescence, and `UiBridge` Re-Entrancy

- **Compile-time turn borrowing & rejection quiescence (from Design It Twice synthesis)**:
  `JsRuntime` introduces a zero-overhead exclusive turn borrow `JsTurn<'_>` and policy enum `RejectionPolicy::{Strict, Log}`:

  ```rust
  pub enum RejectionPolicy { Strict, Log }
  pub struct JsTurn<'a>(&'a mut JsRuntime); // exposes context/module/job/eval; omits drain_rejections

  impl JsRuntime {
      pub fn turn(&mut self) -> JsTurn<'_>;
      pub fn drain_rejections(&mut self, policy: RejectionPolicy) -> Result<RejectionDrain, Diagnostic>;
  }
  ```

  Same-batch `(false -> true)` cancellation resolves inside `RejectionTracker` before formatting or eviction, getter quiescence runs under one clamped `diagnostic_timeout` bounded by `max_tracked_rejections + 1` passes, and cached diagnostics serve cross-batch `handled: true` logs under `RejectionPolicy::Log` without re-entering JS getters. `Session` never calls `drain_rejections` inside `UiBridge::run_turn`.
- **Single-phase concrete `HostPort` & internal 256-entry queue (from Design It Twice synthesis)**:
  `UiBridge` replaces `DeferredEvent`, `EventDispatcher`, two-phase `set_*_dispatcher` methods, and `invoke_*` methods with a concrete `HostPort` constructed before preflight:

  ```rust
  pub enum ExternalOutcome { Completed, Faulted, Busy, Retired }

  impl HostPort {
      pub fn queued() -> Self;
      pub fn live<T: 'static>(surface: SurfaceId, waker: Arc<dyn Waker>, owner: Weak<T>, enter: fn(&T, ExternalStep<'_>) -> ExternalOutcome) -> Self;
      pub fn synchronous<T: 'static>(owner: Weak<T>, enter: fn(&T, ExternalStep<'_>) -> ExternalOutcome) -> Self;
  }

  impl UiBridge {
      pub fn new(registry: NodeRegistry, host: HostHandle, port: HostPort) -> Rc<Self>;
      pub fn run_turn<J, T, E>(&self, js: J, action: impl for<'turn> FnOnce(&BridgeTurn<'turn>, &mut J) -> Result<T, E>) -> Result<T, E>;
      pub fn flush_notification<E>(&self, enter: impl FnOnce(BridgeCallback<'_>) -> Result<(), E>) -> Result<bool, E>;
      pub fn has_queued_notifications(&self) -> Result<bool, QueueOverflow>;
  }
  ```

  - **Critical intra-frame ordering invariant**: native `Window` calls `HostBinding::checkpoint` *before* layout and `deliver_observations` *after* layout (running synchronous relayout between up to two passes before painting without calling `checkpoint` again). Therefore, when `!active` and the engine is free, **both** external input events and geometry observations enter synchronously via `HostPort` so `EventCx` cancellation and first-paint `Popper` placement settle in the same frame; only when `active` is true or `Session` cannot borrow the engine (`Busy`) does `UiBridge` enqueue into its internal 256-entry FIFO and wake the host.
  - **Single-phase initialization & full fault disposal**: `Session` binds `HostPort::live` via `Rc::new_cyclic` at construction. Any `Err` from `run_turn`, `ExternalOutcome::Faulted`, or queue overflow (>256 queued items) invokes full `UiBridge::dispose()`, retiring stylesheets, dismiss registrations, listeners, geometry observers, scroll locks, focus traps, queued items, and **Mount region** nodes together.

### 2. Preview Frame Observation, Headless Bootstrap, and Session Retirement

- **Zero-argument idempotent `Session::retire` & unified preview evaluator (from Design It Twice synthesis)**:
  `Session` replaces `shutdown`, `take_cleanup_failure`, and `runtime_telemetry` with one memoized retirement report, tracking internally whether a **Fixture runtime** contract failure was already reported before retirement so an unreported contract violation beats a cleanup diagnostic without double-logging:

  ```rust
  pub(crate) enum RetirementFailure { Contract(ContractFailure), Cleanup(js_runtime::Diagnostic) }

  pub(crate) struct SessionRetirement {
      pub generation: u64,
      pub before: ResourceCounts,
      pub counts: ResourceCounts,
      pub failure: Option<RetirementFailure>,
      pub telemetry: Option<RuntimeTelemetry>,
  }

  impl Session {
      pub(crate) fn retire(&self) -> SessionRetirement;
  }
  ```

  One preview evaluator classifies `Option<&SessionRetirement>` identically for both surface loss and run completion: `Contract` maps to its typed fault, `Cleanup` maps to `E_TEARDOWN` (exit code `9`) preserving `fault.diagnostic` mapped stacks, and any created **Session** with `counts != ResourceCounts::default()` always yields `E_TEARDOWN` (exit code `9`).
- **RAII headless pipeline & witness-checked `FrameObserver` (from Design It Twice synthesis)**:
  - A shared headless pipeline module owns `App::with_probe` / `App::with_renderer` construction ordering, profile fonts, `hold_clock(true)`, and unwind-safe teardown (`shut_down -> drop(Harness) -> zgui::reactive::shutdown()`), alongside a shared static render entry point used by calibration.
  - `FrameObserver` replaces all 13 mutable `Probe` slots, `CaptureHandle`, `SemanticModel`, and `OwnedFrame` visibility. Callers query `settled(window, session) -> Option<SettledFrame<'_>>` to obtain a coherence-certified witness before reading **Capture evidence**, trees, or GPU pixels.
  - Internally, `FrameObserver` delegates the 9-way coherence rules (generation, DOM/metadata/resource revisions, configuration ID, presentation ID, `presented_draw`, undamaged-skip reuse, frame budget) to a pure value-oriented `FrameLedger` state machine, and delegates checkpoint, capture, and **Historical evidence** persistence to a 3-method internal `CheckpointSink` port (`OutputSink` in production; fault-injectable `MemorySink` in tests), without speculative multi-surface maps.

### 3. Complete 23-Operation In-Memory `NativeHost` Test Adapter

- Extend the in-memory JavaScript host test adapter to implement all 23 `__zgui` methods (`createPortal`, `registerDismissLayer`, `trapFocus`, `lockScroll`, `isAnimating`, `observeGeometry`, and the existing 17), retaining lazy global host resolution so preflight import safety checks continue to fail on eager access.
- Model active/historical lease inspection, `FocusIn`/`FocusOut` bubbling on `focus()`, input/key bubbling with closure-captured cancellation (`preventDefault`/`stopPropagation`/`stopImmediatePropagation`) that works cleanly with shallow event cloning, strict **Bridge turn** guards (invalidating detached nodes at turn end and blocking allocations during teardown), and a single deterministic virtual clock (`advance` + `frame`).
- Replace the ~750 lines of duplicated host patches and 8 `fakeClock` copies across the 15 build test suites, and convert tests that invoke `node.callbacks` directly into user-level bubbling interactions on target/focused nodes.

### 4. Internal Overlay Surface and Hover Consolidation

- Preserve 100% of public exports in `@hewel/zgui/components` and parity in the module map.
- Consolidate exit animation/transition deferral (`data-state`, `isAnimating`, double-tick settling, 1000ms fallback), same-turn QuickJS-stack-safe portal mounting, theme re-scoping, focus-trap/dismiss/scroll-lock lease lifetimes, and modal scrim vs `Popper` anchored placement behind one internal overlay-surface module used by dialog, popover, tooltip, hover-card, and menu surfaces.
- Consolidate hover trigger, pointer/focus intent timers, handoff, and Escape dismissal inside the hover module so `HoverCard` and `Tooltip` share one state machine, calling `openNow()` on trigger focus instead of `closeNow()` followed by `setOpen(true)` (eliminating redundant `[false, true]` controlled updates).
- Migrate build and preview presence verification to exercise public overlay modules directly and remove the internal-package-import lint override from the lint configuration.

## Testing Decisions

- **Principle ("the interface is the test surface" — replace old internal tests rather than stacking new ones)**: delete tests that poke private fields, recompile private Rust source files (`#[path = "../src/events.rs"]`), or depend on internal `createPresence` imports once interface-level tests cover the same invariants.
- **Rust runtime & bridge tests**: test `JsRuntime::drain_rejections` (`Strict` and `Log`, same-batch cancellation, bounded getter quiescence, cached cross-batch recovery) directly; test `UiBridge` through `HostPort::{queued, synchronous}` over `DocumentDom` and `StubHost` (covering listener dispatch, geometry observation, 256-cap queueing, and full stylesheet/lease disposal on turn `Err`); test `Session::retire` precedence and `FrameLedger` coherence transitions directly in memory; and add in-process `Driver` tests with `MemorySink` fault injection verifying checkpoint-failure **Historical evidence** preservation and `E_TEARDOWN` on residual resources at run end.
- **TypeScript & preview tests**: run all 15 build test suites directly against the deepened 23-op host test adapter (adding assertions for strict **Bridge turn** detached-handle rejection and automatic tab `FocusIn` activation), verify zero `[false, true]` `onOpenChange` churn on controlled hover refocus, and run GPU preview presence/overlay/menu/dialog/popover suites against the consolidated overlay modules.
- **Prior art**: follow the two-adapter port pattern already established by **Session host** (`RuntimeSurface` vs stub surface) and **Run ledger** (`Driver` vs scripted driver), the pure invariant test style used for `Popper`, and the fast race-test style used by `dev`.

## Out of Scope

- Menu hierarchy state and `menubar` consolidation (Candidate 5).
- Preview output controller sealing (`doctor`/`run`/`compare`) and `O_NOFOLLOW` file-reader unification (Candidate 6).
- Bundle compiler and atomic **Generation** status publication deepening (Candidate 7).
- Changes inside vendored upstream crates or multi-window preview scenario support.

## Further Notes

Work executes cleanly across two independent tracks with four vertical tickets (`01 -> 02` for Rust runtime and preview; `03 -> 04` for TypeScript host adapter and overlays). Tickets `01` and `03` have no blockers and can start immediately in parallel.
