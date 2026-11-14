# Implementation baseline

The implementation follows [the supplied specification](tsx-native-ui.md). Phases 0–7 passed; Phase 8 records measurements before considering optimizations. Git was initialized during Phase 3; `0287d46` records the supplied specification. The first implementation commit covers verified Phases 0–3; each subsequent completed phase gets its own commit.

## Dependency policy

| Component | Selected identity |
| --- | --- |
| Rust | 1.98.0, minimal profile plus rustfmt/clippy |
| zgui | `80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a` |
| Fontique | Root Cargo patch from the same zgui revision |
| Native runtime / Wayland | Same revision, with the documented [local patches](../../vendor/README.md) |
| rquickjs | `73fcd6062ee615aeb2a4675cd32be67a56e93973`; Phase 0 `std`, Phase 1 adds `macro` for opaque native classes |
| QuickJS-NG | Resolved submodule `6d46d07d04041b40f4f49eaa7fdebe44c314c699` |
| Node | Tested build tooling on 26.13.0 |
| pnpm | 12.3.4 |
| Solid / babel-preset-solid | Exact registry versions 1.9.19 / 1.9.19 |
| JSX compiler | babel-plugin-jsx-dom-expressions 0.40.14 |
| Babel core / TypeScript preset | 7.29.13 / 7.29.7 |
| esbuild / TypeScript | 0.28.2 / 5.9.3 |
| Parley | 0.11.1 with `complex-scripts`; required for CJK layout segmentation |

The Fontique patch is selected deliberately for parity with the audited upstream performance implementation; it is not presented as necessary for correctness. `pnpm-lock.yaml` and `Cargo.lock` record the resolved graphs. Phase 0 graph inspection confirmed patched Fontique 0.11.1 and rquickjs `std`; Phase 1 deliberately added `macro`.

## Commands currently implemented

```sh
pnpm install --frozen-lockfile
pnpm typecheck
pnpm ui:build --smoke
pnpm smoke
pnpm ui:build --app counter
pnpm ui:build --app lists
pnpm ui:build --app async
pnpm ui:build --app editing
pnpm ui:build --app panel
pnpm ui:dev --app counter
pnpm test:build
cargo test -p js-runtime
cargo run -p native-host -- --smoke-module target/solid-smoke.mjs
env ZGUI_PLATFORM=wayland cargo run -p native-host
env ZGUI_PLATFORM=wayland cargo run -p native-host -- --exit-after-ms 5000
env ZGUI_PLATFORM=wayland cargo run -p native-host -- --bundle apps/examples/counter/dist
env ZGUI_PLATFORM=wayland cargo run -p native-host -- --bundle apps/examples/counter/dist --watch
env ZGUI_PLATFORM=wayland cargo run -p native-host -- --bundle apps/examples/lists/dist --exercise --exit-after-ms 12000
env ZGUI_PLATFORM=wayland cargo test -p native-host native_tests::native_button_actions -- --ignored --exact --nocapture
env ZGUI_PLATFORM=wayland cargo test -p native-host --bin native-host async_tests::native_async_delivery -- --ignored --exact --nocapture
env ZGUI_PLATFORM=wayland cargo test -p native-host --bin native-host reload_tests::native_full_reload -- --ignored --exact --nocapture
env ZGUI_PLATFORM=wayland ZGUI_EDITING_WAYLAND_IME=1 cargo test -p native-host editing_tests::native_text_editing -- --ignored --exact --nocapture --test-threads=1
env ZGUI_PLATFORM=wayland cargo run -p native-host -- --bundle apps/examples/panel/dist --layer-config apps/examples/panel/layer.json
env ZGUI_PLATFORM=wayland cargo test -p native-host shell::proof::real_panel_layer -- --ignored --exact --nocapture --test-threads=1
cargo test -p native-host --test native_timer
```

`--smoke-module` executes the bundled Solid fixture in the actual embedded engine without creating a window. `--exit-after-ms` requests orderly event-loop shutdown, for native integration checks. The native baseline creates a dedicated Rust-owned mount region, installs a HostBinding, creates/updates native text from JavaScript, and schedules a native one-shot timer and an owned background completion with an explicit platform wake. These are integration harness behaviors, not the public SDK.

The build uses browser/production Solid export conditions and Babel's Universal preset before esbuild. Graph validation rejects unresolved imports and browser/server renderers and verifies one production reactive Solid runtime. Packages include composed source maps and build metadata. The separate smoke fixture imports upstream Solid declarations; actual native applications use `@zgui/ui`'s ES2022-only declaration boundary without DOM or Node globals.

## Native property and event contract

`@zgui/ui` exposes canonical native `class`, `id`, `title`, `tabIndex`, `disabled`, `role`, `ariaLabel`, `ariaDescription`, `style`, and `--custom` properties. Null/undefined clear values. Object styles accept the curated kebab-case keys declared in `NativeStyleName`, with string values and explicit units; CSS strings use native parser recovery. HTML aliases and listener-option tuples are rejected.

`Button` uses native control defaults, not JS keyboard synthesis. Native event props accept a function or null/undefined; a `Capture` suffix selects capture. `onFocus`/`onBlur` subscribe to bubbling `focusin`/`focusout`. Snapshots retain owned data and opaque node wrappers; cancellation methods expire when the synchronous handler returns. Pointerdown cancellation suppresses that event's focus default, not a later independent pointerup activation; cancelling pointerup suppresses its activation default. Event timestamps are monotonic milliseconds; pointer IDs are decimal strings and text-selection offsets are UTF-8 bytes.

Unexpected nested native notifications are queued without cancellation authority. Each Promise job gets a separate bounded structural turn; wake delivery requests a frame even when the last job empties the queue.

## Scheduling and service contract

Timer callbacks and rAF callbacks use native registrations and release their registration before invocation. Cancellation also removes callbacks already selected by native scheduling but not yet invoked in JS. rAF work runs before layout, including its Promise continuations, and stays pending while occluded; native timers and worker delivery continue without input.

The host permits 1,024 scheduled/ready callbacks and 64 outstanding asset requests per session. Asset reads are limited to 1 MiB of UTF-8 from manifest-declared resources. A shared two-worker executor has a bounded 64-request queue; each session has a bounded completion inbox. Only owned data, IDs, cancellation flags and a platform waker cross threads. Cancellation is definitive at delivery; duplicate/stale completions cannot settle a request twice.

Each owner checkpoint interleaves completion, subscription, scheduled-callback, deferred-notification and Promise work one entry per class. A retained cursor resumes after deadline exhaustion, so a continuously ready source cannot starve another class's cancellation. Non-Promise classes allow 32 entries each and Promise pumping allows 256 attempts, sharing a 10 ms deadline; individual JS entries retain their interrupt limit. Remaining work explicitly wakes the owner. Preflight rejects timer/frame/service creation; teardown stops delivery, cancels registrations and releases persistent roots before destroying the engine.

See [`@zgui/services`](../../packages/services/README.md) for the typed request API and cancellation contract. Console formatting is bounded and does not invoke application getters or `toString`.

## Disposal and full reload

`--watch` observes atomic package publication: the bundle path must resolve to a new immutable directory, as produced by `ui:dev`. A worker checks the canonical target every 500 ms, validates the complete package off-thread, and delivers owned data through a capacity-one inbox with an explicit owner wake. Editing files inside the same published directory is not a supported publication mechanism. Invalid publications leave the active generation untouched.

Candidate preflight evaluates the module, validates exports/version and drains bounded initialization jobs while UI mutation, scheduling and services remain unavailable. Evaluation failure preserves the active app. Successful preflight explicitly retires the old session, replaces its weak HostBinding, and mounts a fresh generation. Mount or runtime failure cleans partial state and displays a Rust-owned diagnostic; native Retry mounts the last successful package in another fresh generation. Late worker, harness and Retry deliveries are generation-checked.

Shutdown is idempotent. Stopping first rejects new nodes, insertion, listeners and async work, but permits cleanup. The JS disposer runs under a deadline even after bridge failure; a thrown cleanup error cannot skip native cancellation, root release or engine destruction. A retired/preflight-only session cannot remove its successor's stylesheet. Native handler registrations are revoked even when another owner retained the callback.

## Native editing

`TextField`/`TextArea` wrap native `field`/`editor` nodes. The bridge receives its
`HostHandle` from `BuildCx` and uses `set_value`, not generic properties or
reconciled text children. Editor-owned descendants remain opaque; equal-value
controlled echoes preserve the native caret and programmatic resets emit no
input event. See the [SDK contract](../../packages/ui/README.md) for initial-only
defaults, read-only state, snapshots and child rejection.

Rust-root-owned whitespace defaults preserve single-line spaces and multiline
layout across JS generations. The host explicitly enables Parley's
`complex-scripts` feature because the pinned zgui dependency does not forward it;
without it, Parley's CJK layout segmentation reports missing models. Editor
Ctrl-arrow movement follows zgui-edit's separate character-class policy.

The ignored editing gate waits at `native-editing-input-ready` for a real
compositor key (for example, `wtype -k F24` sent to its verified owned window).
This supplies Wayland's required input serial for clipboard ownership; injected
runtime events alone cannot authorize it. With `ZGUI_EDITING_WAYLAND_IME=1`, it
then waits at `native-wayland-ime-ready` for actual Chinese composition and
commit through the desktop IME. Clipboard contents are restored after the gate;
its helper does not retain test-log descriptors. Do not run it concurrently with
another clipboard or keyboard-input scenario.

## Native desktop shell

The [panel example](../../apps/examples/panel/README.md) uses an explicit
layer-shell profile and the Rust `watchNiri` service. The service consumes direct
Unix-socket IPC, not the installed niri CLI event parser, which rejects this
compositor's additional `TouchpadToggled` event. Unrelated event types are ignored;
malformed known events and disconnects terminate the subscription without retries.

Snapshots coalesce behind a native wake and contain lossless decimal-string IDs
and explicit nulls. Four subscriptions are allowed per session. Payloads define
own data properties rather than invoking inherited setters; delivery rechecks
cancellation around reentrant engine error construction. Cancellation and terminal
errors release callbacks and the transport before session destruction.

The host selects a named output, applies anchors/margins/keyboard policy and an
explicit exclusive zone, and leaves scaling to the native backend. The backend
finishes initial output descriptions before invoking `surfaces_available`.
Layer-parent out-of-surface popups are explicitly unsupported; a normal-window
demo is not treated as proof of that capability.

### Native shell acceptance

`ZGUI_SHELL_LIFECYCLE=1` extends the ignored layer gate to wait up to 40 seconds
for an externally driven output removal/return. It requires a fresh live JS
generation and zero resources from both generations. Changing physical outputs
requires explicit permission; restore their scale/enablement, prior focus and
any clipboard changed by screenshot capture afterward.

## Observed verification

- `pnpm install`: passed; requested Solid/compiler pair exists in the registry.
- `pnpm typecheck`: passed for the Phase 0 smoke fixture.
- `pnpm smoke`: passed twice in Node; verifies signal/effect updates, equality, batching, effect cleanup, root cleanup, and absence of updates after disposal.
- `pnpm list --recursive --depth Infinity`: resolved package graph inspected.
- `cargo build -p native-host`: passed against the pinned downstream dependencies.
- `cargo test -p js-runtime`: 12 passed, including exception preservation, native callbacks, throwing jobs, interruption and actual engine memory exhaustion.
- `target/debug/native-host --smoke-module target/solid-smoke.mjs`: `solid-smoke:ok`.
- Direct Wayland native run: binding installed before first paint; native text updated from JS, a one-second native timer, and a two-second background completion without input.
- `target/native-baseline.png`: captured and visually inspected the real niri window showing “External wake updated JavaScript”.
- Orderly exit after 20 seconds: `session-disposed: frames=62`, `native-host: clean shutdown`, exit code 0. Frame count includes compositor activity and is not an idle-wake benchmark.
- Phase 1: 12 real-DocumentDom topology tests and 2 real-rquickjs wrapper tests passed. Tests cross native frame reclamation, verify identity-preserving moves, fabricated/stale/foreign-handle rejection, root protection and cleanup of never-linked nodes.
- Phase 1 native integration: timer and external wake updated class-backed opaque nodes; five native frames and clean exit, with no raw native IDs exposed to JavaScript.
- Phase 2: native declaration typechecks, both package builds and seven compiler tests passed (AST validation, composed maps, atomic publication, watcher recovery and symlink rejection).
- Phase 2 Rust checks: 28 tests passed across runtime, bridge and package path/integrity validation.
- Actual counter package on Wayland: Count 0→4 and memo 0→8; screenshot `target/counter.png`; clean shutdown with zero nodes/wrappers.
- Actual lists package on Wayland: A/B/C→C/A/B, lazy branch hidden across native frames then recreated, Index updated, list restored; screenshot `target/lists.png`; clean shutdown with zero nodes/wrappers.
- Phase 3: 11 property tests, 5 event-registry/token tests, and 4 package/source-map tests passed. Native SDK typechecks and counter build passed.
- Runtime ownership/rejection/deadline checks: 22 tests passed. Independent review found a diagnostic accessor could extend a job/cleanup deadline; the deadline clamp and regression passed, and read-only re-review reported no remaining findings.
- Phase 3 real Wayland input gate: passed over 27 settled frames at scale 1.75. Pointer/Enter/Space each activate once; repeats do not; native focus and activation cancellation, capture/bubble stopping, handler replacement/removal, and 48 expired cancellation attempts passed. Cached-disabled focus and disabling during the same keydown cannot activate after the documented native patch. Shutdown retained zero nodes/wrappers.
- `target/phase3-counter.png`: visually inspected the real counter with styled native Button; interactive binary started and exited with code 0.
- Phase 4: 10 worker-executor tests and 8 real-engine request/callback lifecycle tests passed. Native SDK typechecks, async package build and 9 compiler tests passed.
- Self-replenishing microtask regression passed: the pump yields after eight attempts; a subsequent owner entry cancels the chain, and the final pending job drains.
- `target/async.png`: visually inspected real native text after timer, frame, Promise layout and Rust asset completion; orderly exit code 0.
- Independent async ownership review found cancelled native timers retained closures until their original deadlines. Eager in-place reclamation and its focused vendored-module gate passed all 9 timer tests; read-only re-review reported no further findings.
- Phase 4 real niri occlusion gate passed after all native patches: timer and Rust task completed while occluded; rAF/Promise remained pending; the first visible fired frame contained the Promise row at 56 device px (32 CSS px × 1.75); all resource counts reached zero.
- The gate verifies its own PID/app-ID window, moves it by stable IPC workspace ID without following focus, waits for settled real occlusion, and restores the original workspace. Startup does not require focus or an initial visibility notification. Bounded bootstrap frame probes let the backend detect missed compositor callbacks before JS starts; no synthetic occlusion events are used. Earlier startup assumptions, transient visibility and output-relative CLI workspace indexes were rejected during gate development.
- Phase 5 focused checks passed: 29 host tests across package validation, callback/request ownership, watcher publication, Rust fallback and session lifecycle, plus 3 real-engine bridge-node tests. Coverage includes 32 teardown cycles, preflight microtasks, throwing/faulted cleanup, retained old sessions and worker completions crossing generations. Independent read-only lifecycle review reported no actionable findings.
- Phase 5 real Wayland reload gate passed: evaluation failure preserved the active app; 16 replacements stabilized fresh resource counts and retired every generation to zero; native frame reclamation removed old layout rows; mount failure displayed the Rust fallback; native pointer activation of Retry recovered; stale Update/Retry messages had no effect.
- `target/reload-fallback.png`: captured and visually inspected the real Rust diagnostic and native Retry button.
- Production `--watch` smoke passed without input: an invalid atomic publication preserved the counter, the next valid publication mounted the async package, and both retired generations reached zero resources on clean shutdown. `target/watch-reload.png` visually confirmed timer, frame, Promise layout and Rust task completion after the watched reload.
- Phase 6: 36 bridge tests, native SDK typechecks and the reactive editor-child rejection compiler regression passed. On the unlocked desktop, the native editing gate passed controlled-value/caret behavior, clipboard copy/cut/paste, undo, native CJK grapheme movement and vertical multiline navigation, default/read-only/disabled behavior, focus and stable accessibility identities. Real Rime preedit and Chinese commit reached the controlled field. Both generations retired to zero resources; clipboard and prior focus/IME activation were restored. `target/native-editing-verified.png` was captured and visually inspected, confirming separate lines and rendered Chinese glyphs.
- Phase 7 preparation: 21 backend tests plus the focused rejected-update publication regression passed; SDK/panel typechecks and the panel bundle build passed. Final owner-thread subscription regressions passed all 8 tests; session lifecycle/fairness checks passed all 7. Earlier unchanged work-lifecycle checks also passed.
- Independent subscription review identified prototype-setter reentry, unfair checkpoint ordering and deferred-queue borrow lifetime issues. Own-property conversion, cancellation rechecks, round-robin scheduling and explicit borrow release were verified by focused regressions; final read-only re-review reported no remaining findings in those fixes.
- Real niri layer smoke passed without input: compositor IPC reported namespace `dev.zgui.js.panel`, output `DP-1`, top layer and keyboard mode `None`. Wayland requested size `0 × 360` and exclusive zone `360`; the compositor configured `2142 × 360` CSS pixels and fractional scale `210/120 = 1.75`. Real workspace/window data reached native TSX rows while idle. Shutdown reclaimed every resource, including the subscription, to zero. This run did not capture a screenshot.
- Phase 7 full output-lifecycle gate passed after the desktop was unlocked and output changes were explicitly authorized. Three existing tiled windows each lost exactly 360 CSS pixels of height while the layer reserved its zone. DP-1 scale changed 1.75→2→1.75, with native redraws. Turning DP-1 off released generation 1 completely; turning it on recreated the layer and mounted generation 2 with a fresh live niri subscription. Both generations reached zero resources. All output geometries, prior focus and clipboard were restored. `target/native-panel-recreated.png` was visually inspected after recreation.

Environment: Linux x86_64 Wayland session; niri compositor reports `26.04 (v26.04-298-gad20c318-modified)`, CLI `26.04 (8ed0da4)`; Wayland client 1.26.0, xkbcommon 1.13.2, Fontconfig 2.18.3, Vulkan 1.4.363. No performance claims are made from this baseline.

## Acceptance tracking

- [x] Phase 0: downstream window, script exception, real Solid execution, binding, wake and shutdown.
- [x] Phase 1: opaque handles, topology, turn-scoped invalidation and failed-construction reclamation.
- [x] Phase 2: native SDK declarations, TSX packages, reconciliation and examples.
- [x] Phase 3: properties, native cancellation, handler replacement and Button activation.
- [x] Phase 4: bounded jobs, timer/rAF lifetimes, asynchronous requests and idle/occluded delivery.
- [x] Phase 5: disposal, preflight, reload generations and fallback.
- [x] Phase 6: controlled editing, clipboard, selection, Chinese IME and accessibility.
- [x] Phase 7: layer-shell, monitor/scale/focus policy and typed desktop services.
- [ ] Phase 8: only optimizations justified by measured bottlenecks.
