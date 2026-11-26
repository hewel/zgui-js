# Implementation baseline

The implementation follows [the supplied specification](tsx-native-ui.md). Phases 0–7 passed; Phase 8 recorded the limited release baseline below and introduces no speculative optimization. Git was initialized during Phase 3; `0287d46` records the supplied specification. The first implementation commit covers verified Phases 0–3; each subsequent completed phase gets its own commit.

Preview commands and phase acceptance are tracked in [preview verification](preview-verification.md).

Current scope is a general-purpose UI library: native UI, resource services,
scheduling, previews, and a thin wrapper over upstream zgui layer-shell support.
Platform and window-manager support is determined by upstream zgui, not a
compositor-specific baseline here. The niri service, panel example, and
niri-dependent integration gates have been removed. Historical phase evidence
below is retained and does not describe the currently shipped API or test suite.

## Dependency policy

| Component                   | Selected identity                                                                                                                        |
| --------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| Rust                        | 1.100.0, minimal profile plus rustfmt/clippy                                                                                             |
| zgui                        | `ee29c7dc66206eb04ba7bcf514058a278c9070ba`                                                                                               |
| Fontique                    | Root Cargo patch from the same zgui revision                                                                                             |
| Native runtime / Wayland    | Same revision, with the documented [local patches](../../vendor/README.md)                                                               |
| rquickjs                    | `73fcd6062ee615aeb2a4675cd32be67a56e93973`; Phase 0 `std`, Phase 1 adds `macro` for opaque native classes                                |
| QuickJS-NG                  | Resolved submodule `6d46d07d04041b40f4f49eaa7fdebe44c314c699`                                                                            |
| Bun                         | `>= 1.4.4` — runtime, package manager, and test runner (`bunfig.toml`, `bun.lock`)                                                       |
| Solid / Universal / Signals | Exact registry versions `solid-js` 2.0.1 / `@solidjs/universal` 2.0.1 / `@solidjs/signals` 2.0.1                                         |
| CSS Modules compiler        | `lightningcss` 1.33.0 (`lightningcss-v1` native profile; supersedes StyleX per ADR-0006)                                                 |
| JSX compiler                | `@solidjs/compiler` 2.0.1 (native Oxc + WASI fallback; supersedes Babel per ADR-0007)                                                    |
| Rolldown / tsrs             | 1.3.2 / 7.1.0-beta.tsrs2                                                                                                                 |
| Oxlint / Oxfmt              | 1.94.0 (`--deny-warnings` + `typeAware` via `tsrs` + `packages/ui/oxlint/{plugin,solid}.mjs` with `eslint-plugin-solid` 0.20.1) / 0.79.0 |
| Parley                      | 0.11.1 with `complex-scripts`; required for CJK layout segmentation                                                                      |

The Fontique patch is selected deliberately for parity with the audited upstream performance implementation; it is not presented as necessary for correctness. `bun.lock` and `Cargo.lock` record the resolved graphs. Phase 0 graph inspection confirmed patched Fontique 0.11.1 and rquickjs `std`; Phase 1 deliberately added `macro`.

`tsrs` (`oxc-project/tsrs`) is pinned to `7.1.0-beta.tsrs2` in both the root and `@hewel/zgui-build` manifests. Workspace checks and one-shot application builds invoke the installed package's `bin/tsrs` synchronously through the current Bun executable with `--project … --noEmit`; development uses a cancellable asynchronous child, and `oxlint` uses `tsrs`'s `tsrs-lint` binary for type-aware rules (`options.typeAware: true`). Compiler diagnostics remain the thrown error text. The CLI is resolved from `@hewel/zgui-build`'s local dependency, never from the shell or a global `tsc`/`tsrs`. `@solidjs/compiler` + Lightning CSS + Rolldown perform runtime compilation; typechecking does not emit JavaScript or change native consumers' ES2022-only, DOM/Node-free environment.

## Commands currently implemented

```sh
bun install --frozen-lockfile
bun run typecheck
bun packages/cli/src/cli.ts smoke
bun run smoke
bun run build --app counter
bun run build --app lists
bun run build --app async
bun run build --app editing
bun run dev --app counter
bun run preview --app counter
bun run preview --app counter --watch
bun run test:build
cargo test -p js-runtime
cargo run -p zgui-host -- --smoke-module target/solid-smoke.mjs
env ZGUI_PLATFORM=wayland cargo run -p zgui-host
env ZGUI_PLATFORM=wayland cargo run -p zgui-host -- --exit-after-ms 5000
env ZGUI_PLATFORM=wayland cargo run -p zgui-host -- --bundle apps/examples/counter/dist
env ZGUI_PLATFORM=wayland cargo run -p zgui-host -- --bundle apps/examples/counter/dist --watch
env ZGUI_PLATFORM=wayland cargo run -p zgui-host -- --bundle apps/examples/lists/dist --exercise --exit-after-ms 12000
env ZGUI_PLATFORM=wayland cargo test -p zgui-host native_tests::native_button_actions -- --ignored --exact --nocapture
env ZGUI_PLATFORM=wayland cargo test -p zgui-host --bin zgui-host reload_tests::native_full_reload -- --ignored --exact --nocapture
env ZGUI_PLATFORM=wayland ZGUI_EDITING_WAYLAND_IME=1 cargo test -p zgui-host editing_tests::native_text_editing -- --ignored --exact --nocapture --test-threads=1
cargo test -p zgui-host --test native_timer
```

`--smoke-module` executes the bundled Solid fixture in the actual embedded engine without creating a window. `--exit-after-ms` requests orderly event-loop shutdown, for native integration checks. The native baseline creates a dedicated Rust-owned mount region, installs a HostBinding, creates/updates native text from JavaScript, and schedules a native one-shot timer and an owned background completion with an explicit platform wake. These are integration harness behaviors, not the public SDK.

The build uses browser/production Solid export conditions and a StyleX pass before Babel's Universal preset, followed by Rolldown. First-pass StyleX metadata is retained across the complete reachable source graph, including token/dependency modules, and finalized with the official rule processor before tree shaking can discard imports. Graph validation rejects unresolved imports, browser/server renderers, runtime injectors and untransformed compile-time calls, and verifies one production reactive Solid runtime. Schema-v2 packages include composed source maps, generated StyleX CSS after the optional base sheet, module-relative style provenance, per-artifact/canonical build digests, and build metadata with explicit module/chunk graphs. Development uses a persistent Rolldown watcher with source-keyed StyleX/Babel/Solid transform caching; Rolldown 1.3.2's public watcher still rescans the module graph. Type checking gates atomic publication. `dev` owns Cargo preparation, the builder, and a single native host; `preview --watch` owns the builder and offscreen supervisor instead. The separate smoke fixture imports upstream Solid declarations; actual native applications use `@hewel/zgui`'s ES2022-only declaration boundary without DOM or Node globals.

### Managed development verification

- `pnpm typecheck`: SDK, consumer fixtures, and all four examples passed.
- `pnpm test:build`: 37 checks passed, including external application directories,
  composed source maps, Babel transform caching, and process cancellation.
- After removing publication deduplication, `node --test tests/build/compiler.test.mjs tests/preview/watch.test.mjs` passed all 20 checks. Recovery to the exact original source now produces a fresh preview run even when the build ID is unchanged.
- `pnpm smoke` and `target/debug/zgui-host --smoke-module target/solid-smoke.mjs`
  passed in Node and embedded QuickJS respectively.
- `cargo test -p zgui-host --bin zgui-host --locked package::`: five package
  integrity/source-map checks passed.
- A real Wayland development session recovered from an initially invalid app,
  retained its package after a later type error, reloaded changed TSX and CSS
  through generations 1–3, and reaped its native child on SIGTERM.
- Application-local `pnpm dev --exit-after-ms 1000` opened the counter and exited
  cleanly with zero retained session resources.
- Managed one-shot preview passed the counter click assertion and produced two
  captures; managed `preview --watch` completed a real run and reaped its native
  supervisor on SIGINT. These checks do not imply state-preserving HMR or partial
  native module-graph scans.

## Native property and event contract

`@hewel/zgui` exposes canonical native `class`, StyleX `className`, `id`, `title`, `tabIndex`, `disabled`, `role`, `ariaLabel`, `ariaDescription`, `style`, diagnostic `data-style-src`, and `--custom` properties. Null/undefined clear values. Empty class aliases contribute no tokens; two token-bearing aliases produce `CLASS_ALIAS_CONFLICT` without replacing the previous application classes. SDK structural classes and inline invariants have separate ownership. Object styles accept the curated native property mapping with string values, finite numeric custom properties, and explicitly unitless finite numeric ordinary properties. Style objects diff per key and commit atomically only after native validation; omitted keys restore SDK defaults or remove the declaration. CSS strings replace only the application-owned map under the same validation rule. See [StyleX authoring](stylex.md) for the merge/cascade contract.

`Button` uses native control defaults, not JS keyboard synthesis. Native event props accept a function or null/undefined; a `Capture` suffix selects capture. `onFocus`/`onBlur` subscribe to bubbling `focusin`/`focusout`. Snapshots retain owned data and opaque node wrappers; cancellation methods expire when the synchronous handler returns. Pointerdown cancellation suppresses that event's focus default, not a later independent pointerup activation; cancelling pointerup suppresses its activation default. Event timestamps are monotonic milliseconds; pointer IDs are decimal strings and text-selection offsets are UTF-8 bytes.

Unexpected nested native notifications are queued without cancellation authority. Each Promise job gets a separate bounded structural turn; wake delivery requests a frame even when the last job empties the queue.

## Scheduling and service contract

Timer callbacks and rAF callbacks use native registrations and release their registration before invocation. Cancellation also removes callbacks already selected by native scheduling but not yet invoked in JS. rAF work runs before layout, including its Promise continuations, and stays pending while occluded; native timers and worker delivery continue without input.

The host permits 1,024 scheduled/ready callbacks and 64 outstanding asset requests per session. Asset reads are limited to 1 MiB of UTF-8 from manifest-declared resources. A shared two-worker executor has a bounded 64-request queue; each session has a bounded completion inbox. Only owned data, IDs, cancellation flags and a platform waker cross threads. Cancellation is definitive at delivery; duplicate/stale completions cannot settle a request twice.

Each owner checkpoint interleaves completion, scheduled-callback, deferred-notification and Promise work one entry per class. A retained cursor resumes after deadline exhaustion, so a continuously ready source cannot starve another class's cancellation. Non-Promise classes allow 32 entries each and Promise pumping allows 256 attempts, sharing a 10 ms deadline; individual JS entries retain their interrupt limit. Remaining work explicitly wakes the owner. Preflight rejects timer/frame/service creation; teardown stops delivery, cancels registrations and releases persistent roots before destroying the engine.

See [`@hewel/zgui-services`](../../packages/services/README.md) for the typed request API and cancellation contract. Console formatting is bounded and does not invoke application getters or `toString`.

## Disposal and full reload

`--watch` observes atomic package publication: the bundle path must resolve to a new immutable directory, as produced by `pnpm dev` or `pnpm build --watch`. A worker checks the canonical target every 500 ms, validates the complete package off-thread, and delivers owned data through a capacity-one inbox with an explicit owner wake. Editing files inside the same published directory is not a supported publication mechanism. Invalid publications leave the active generation untouched.

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
commit through the desktop IME. By default, the gate reads and restores the
previous clipboard contents; its helper does not retain test-log descriptors.
When reading the original clipboard is not authorized, explicitly authorize
destructive replacement and set `ZGUI_EDITING_REPLACE_CLIPBOARD=1`. That mode
neither reads nor restores the original selection and leaves test-owned contents
in place. Do not run the gate concurrently with another clipboard or keyboard-input
scenario. The real compositor/IME run is recorded in
[preview P6 verification](preview-verification.md#p6--real-waylandniri-desktop-verification).

## Pure-shell startup

Run `bun run dev --app <app> --pure-shell` (or pass `--pure-shell --bundle <dist>`
to `zgui-host`). Declare `zgui.capabilities.shellSurfaces` or
`zgui.capabilities.sessionLock` and export a synchronous `mount()` returning
a disposer. The Session starts without a primary window and can remain alive
with zero Shell surfaces.
Without `--pure-shell`, `mount(root)` remains the default.

`useMonitors()` follows event-driven output inventory; a `LayerSurface` tied
to a disconnected output is unmounted until that output returns or its owner
removes it. Multiple Shell surfaces share the same Session. A live change
to `anchor`, dimensions, layer, margins, exclusive zone, keyboard policy,
or Input region does not replace the Mount region.
Detached nodes stop contributing their rectangles to the Input region. An
exclusive zone of `-1` ignores other surfaces' reservations; `0` reserves no
space. A reload is accepted only after the candidate's requested Shell
surfaces have opened and mounted. Failed candidates close their surfaces
without replacing the working Session or its last successful build.

## Upstream layer-shell configuration

The host's generic `--layer-config PATH` wrapper selects a named output and
passes anchors, margins, keyboard policy, transparency and an explicit exclusive
zone to upstream zgui. Scaling and compositor protocol support remain upstream
responsibilities; no window-manager abstraction or compositor-specific package
is provided. The backend finishes initial output descriptions before invoking
`surfaces_available`.
The package must declare `shellSurfaces` both at startup and on reload.

Layer-parent popups use `zwlr_layer_surface_v1.get_popup`, verified by the
offline `popup_protocol` gate. A normal-window or same-surface overlay demo does
not exercise that path. The former niri panel and its acceptance gate are retired.
Any real-desktop verification must use a supported upstream backend and require
explicit permission before changing physical outputs, focus, clipboard or input.

## Observed verification

These are historical observations from the implementation phases, not a list of
current gates. In particular, the niri occlusion/layer gates and subscription
tests described below were retired with the niri service and panel example.

- `pnpm install`: passed; requested Solid/compiler pair exists in the registry.
- `pnpm typecheck`: passed for the Phase 0 smoke fixture.
- `pnpm smoke`: passed twice in Node; verifies signal/effect updates, equality, batching, effect cleanup, root cleanup, and absence of updates after disposal.
- `pnpm list --recursive --depth Infinity`: resolved package graph inspected.
- `cargo build -p zgui-host`: passed against the pinned downstream dependencies.
- `cargo test -p js-runtime`: 12 passed, including exception preservation, native callbacks, throwing jobs, interruption and actual engine memory exhaustion.
- `target/debug/zgui-host --smoke-module target/solid-smoke.mjs`: `solid-smoke:ok`.
- Direct Wayland native run: binding installed before first paint; native text updated from JS, a one-second native timer, and a two-second background completion without input.
- `target/native-baseline.png`: captured and visually inspected the real niri window showing “External wake updated JavaScript”.
- Orderly exit after 20 seconds: `session-disposed: frames=62`, `zgui-host: clean shutdown`, exit code 0. Frame count includes compositor activity and is not an idle-wake benchmark.
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

Environment: Linux x86_64 Wayland session; niri compositor reports `26.04 (v26.04-298-gad20c318-modified)`, CLI `26.04 (8ed0da4)`; Wayland client 1.26.0, xkbcommon 1.13.2, Fontconfig 2.18.3, Vulkan 1.4.363. The following measurements describe these workloads only, not general engine or renderer speed.

## Phase 8: measured optimization decision

This section preserves historical measurements, including the removed niri panel
workload. They do not imply that the panel or its service is still available.

No optimization was selected. These small workloads do not establish a sustained
bridge, reconciliation or job-pump bottleneck. Native startup paint is a visible
cost, but the measurements do not isolate its causes or justify replacing the
existing execution/ownership model.

### Configuration and procedure

- AMD Ryzen 9 7950X, 16 cores / 32 threads; Linux `7.3.3-1-cachyos`.
- The pinned dependencies above, `cargo build --release -p zgui-host`, and
  freshly built counter, lists and panel bundles. Release build and all three
  bundle builds passed.
- Real shared niri desktop, three outputs at scale 1.75; panel on DP-1. The
  desktop was not isolated from compositor or user activity.
- Three sequential runs per workload, ten seconds each, with native
  `ZGUI_LATENCY` instrumentation enabled. No concurrent benchmark processes.
- Counter and panel: sample process CPU/RSS for five seconds starting three
  seconds after the host first-frame log. Lists: sample for 4.5 seconds starting
  0.5 seconds after that log; `--exercise` performs four one-second-spaced native
  button activations, exercising list reorder, branch toggling and Index updates.
- CPU is the delta of `/proc/PID/stat` user plus system ticks; `CLK_TCK=100`.
  RSS is `/proc/PID/status` `VmRSS`, not JavaScript heap usage.
- Every measured run exited with code zero, logged clean shutdown and retired
  the engine with every reported resource count zero. Prior window focus was
  restored after the series.

The remaining counter and lists workload setup and native trace capture use:

```sh
cargo build --release -p zgui-host
pnpm build --app counter
pnpm build --app lists
env ZGUI_PLATFORM=wayland ZGUI_LATENCY=target/counter-run.jsonl \
  target/release/zgui-host --bundle apps/examples/counter/dist --exit-after-ms 10000
env ZGUI_PLATFORM=wayland ZGUI_LATENCY=target/lists-run.jsonl \
  target/release/zgui-host --bundle apps/examples/lists/dist --exercise --exit-after-ms 10000
```

Use a fresh trace path per run: the native sink appends. The recorded bundle IDs
were counter `sha256:0955f7ffdaad90434bd6751ae5b3e465a28f97800dd516edbe4e6a2f64499597`,
lists `sha256:c29c10172f52be964c159c898ed1a93852974d567c1af335a7521aa5ad2626c6`,
and panel `sha256:1378e7fcc9bec1eb4628ba33e51c7c03dc53126a995c08dad61e5bc32e62a089`.

### Observed samples

Each cell lists all three samples in run order; these are not confidence intervals.
Startup is process launch to receipt of the host's first-frame log, **not**
compositor presentation latency.

| Workload            | Startup log (ms)         | End RSS (MiB)         | Sampled process CPU (ms) | Whole-run native frames |
| ------------------- | ------------------------ | --------------------- | ------------------------ | ----------------------- |
| Counter             | 53.565 / 53.703 / 58.993 | 76.23 / 75.93 / 76.64 | <10 / <10 / <10          | 3 / 3 / 210             |
| Lists, four updates | 57.178 / 55.299 / 54.206 | 76.25 / 76.43 / 76.44 | <10 / 10 / 10            | 7 / 7 / 7               |
| Live niri panel     | 62.896 / 63.612 / 61.881 | 82.83 / 82.58 / 82.51 | <10 / <10 / <10          | 2 / 2 / 2               |

Zero observed CPU ticks are reported as below the 10 ms sampling resolution, not
zero work. Counter/panel RSS did not change within their sample windows. Lists
RSS increased by 72 / 80 / 80 KiB across its four-update windows.

Native stage maxima across each workload's three runs, including startup:

| Workload | Frame (ms) | Restyle (ms) | Layout (ms) | Paint/draw (ms) |
| -------- | ---------- | ------------ | ----------- | --------------- |
| Counter  | 6.248      | 0.406        | 0.571       | 5.060           |
| Lists    | 7.338      | 0.469        | 0.782       | 6.069           |
| Panel    | 17.209     | 1.012        | 3.580       | 12.194          |

Intervals use `f.begin`→`f.end`, `f.restyle`→`f.restyled`,
`f.layout`→`d.postlayout` and `f.paint`→`f.painted`. They include instrumentation
overhead; paint/draw is not a pure GPU execution measurement.

Counter's third run contained 207 `Skipped(Undamaged)` frames within the first
two trace seconds. The trace also records initial scale/configure transitions;
it does not establish the cause of those extra frames. They are retained in the
results, not discarded. No native frame/redraw-request marks appeared in the
approximate steady counter/panel sampling windows. Each lists window contained
four frame starts and eight redraw-request marks.

Raw local traces are `target/phase8-counter-{2,3,4}.jsonl`,
`target/phase8-lists-{1,2,3}.jsonl` and `target/phase8-panel-{1,2,3}.jsonl`.
An earlier counter pilot (`phase8-counter-1.jsonl`, before the explicit bundle
rebuild) took 287.377 ms to its first-frame log and is not included in the series.
The series is warm-machine evidence, not a controlled cold-start benchmark.

### Attribution limits

The native trace starts after Wayland application construction. The startup log
includes setup and evaluation but does not separate bundle IO, JS evaluation or
mount CPU time. This baseline does not separately measure JS callbacks/job pumps,
host mutation counts/time, JS heap size, outstanding wake requests or OS wake
frequency. Native redraw marks are not OS wakeups, and RSS is not JS heap.
The enabled trace buffer also contributes memory and timing overhead.

These limits prevent claims that every measurement requested in specification
§13.4 is already available, or that any proposed optimization would improve it.
No before/after speedup is claimed because no performance code was changed.

## Acceptance tracking

This checklist records original phase completion. Phase 7's niri service and
panel, and the niri-specific Phase 4 desktop gate, are no longer shipped.

- [x] Phase 0: downstream window, script exception, real Solid execution, binding, wake and shutdown.
- [x] Phase 1: opaque handles, topology, turn-scoped invalidation and failed-construction reclamation.
- [x] Phase 2: native SDK declarations, TSX packages, reconciliation and examples.
- [x] Phase 3: properties, native cancellation, handler replacement and Button activation.
- [x] Phase 4: bounded jobs, timer/rAF lifetimes, asynchronous requests and idle/occluded delivery.
- [x] Phase 5: disposal, preflight, reload generations and fallback.
- [x] Phase 6: controlled editing, clipboard, selection, Chinese IME and accessibility.
- [x] Phase 7: layer-shell, monitor/scale/focus policy and typed desktop services.
- [x] Phase 8: record release measurements and limits; select no unsupported optimization.
