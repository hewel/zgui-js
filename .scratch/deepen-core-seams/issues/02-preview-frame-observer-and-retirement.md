# 02: Deepen preview frame observation, headless bootstrap, and Session retirement

**What to build:** Consolidate **Session** retirement reporting, headless native bootstrap (shared with calibration), and preview frame observation behind one idempotent `Session::retire` contract, one RAII headless pipeline, and one witness-checked `FrameObserver` backed by a pure coherence state machine and an internal persistence seam—fixing silent exit-`0` resource leaks at run end and enabling fast in-process driver tests.

**Blocked by:** 01 (Internalize Bridge turn disposal, HostPort queueing, and rejection quiescence)

**Status:** ready-for-agent

- [ ] `Session::retire(&self) -> SessionRetirement` is zero-argument, idempotent, and memoized, capturing pre-shutdown `ResourceCounts`, tracking `contract_reported` internally so unreported contract violations beat cleanup diagnostics without duplicate reporting, remapping cleanup `Diagnostic` stacks before dropping the engine, and merging final runtime telemetry.
- [ ] Preview surface loss (`surfaces_lost`) and run completion (`Driver::finish`) share one `SessionRetirement` evaluator: whenever a **Session** was created and `counts != ResourceCounts::default()`, both paths always produce an `E_TEARDOWN` fault (exit code `9`), and cleanup exceptions retain source-mapped diagnostic stacks on both paths.
- [ ] One shared RAII headless pipeline module owns `App::with_probe` / `App::with_renderer` construction ordering, profile fonts, `hold_clock(true)`, static calibration rendering, and strict `shut_down -> drop(Harness) -> zgui::reactive::shutdown()` cleanup across normal completion, early errors, and test panics.
- [ ] `FrameObserver` hides all 13 mutable `Probe` fields, `CaptureHandle`, `SemanticModel`, and `OwnedFrame`, exposing a coherence-checked `SettledFrame<'_>` witness for live action/capture consumers and separate **Historical evidence** publication on fault.
- [ ] Internal pure `FrameLedger` transitions (presented frames, undamaged skips, resize configuration lag, frame-budget enforcement) and a 3-operation internal `CheckpointSink` (`OutputSink` in production; fault-injectable `MemorySink` in tests) enable fast in-process Rust tests for frame coherence, checkpoint write faults, **Historical evidence** fallback, and leaked-resource `E_TEARDOWN` verification.
- [ ] Workspace Rust tests (`pnpm test:rust`), build/gate checks (`pnpm check`), and preview suites pass after the final edit.
