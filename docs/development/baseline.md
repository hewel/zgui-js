# Implementation baseline

The implementation follows [the supplied specification](tsx-native-ui.md). Phases 0–3 passed; Phase 4 is in progress. Later phase acceptance gates are not yet passed. Git was initialized during Phase 3; `0287d46` records the supplied specification. The first implementation commit covers verified Phases 0–3; each subsequent completed phase gets its own commit.

## Dependency policy

| Component | Selected identity |
| --- | --- |
| Rust | 1.98.0, minimal profile plus rustfmt/clippy |
| zgui | `80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a` |
| Fontique | Root Cargo patch from the same zgui revision |
| Native runtime | Same revision, with the documented [disabled-activation patch](../../vendor/README.md) |
| rquickjs | `73fcd6062ee615aeb2a4675cd32be67a56e93973`; Phase 0 `std`, Phase 1 adds `macro` for opaque native classes |
| QuickJS-NG | Resolved submodule `6d46d07d04041b40f4f49eaa7fdebe44c314c699` |
| Node | Tested build tooling on 26.13.0 |
| pnpm | 12.3.4 |
| Solid / babel-preset-solid | Exact registry versions 1.9.19 / 1.9.19 |
| JSX compiler | babel-plugin-jsx-dom-expressions 0.40.14 |
| Babel core / TypeScript preset | 7.29.13 / 7.29.7 |
| esbuild / TypeScript | 0.28.2 / 5.9.3 |

The Fontique patch is selected deliberately for parity with the audited upstream performance implementation; it is not presented as necessary for correctness. `pnpm-lock.yaml` and `Cargo.lock` record the resolved graphs. Phase 0 graph inspection confirmed patched Fontique 0.11.1 and rquickjs `std`; Phase 1 deliberately added `macro`.

## Commands currently implemented

```sh
pnpm install --frozen-lockfile
pnpm typecheck
pnpm ui:build --smoke
pnpm smoke
pnpm ui:build --app counter
pnpm ui:build --app lists
pnpm ui:dev --app counter
pnpm test:build
cargo test -p js-runtime
cargo run -p native-host -- --smoke-module target/solid-smoke.mjs
env ZGUI_PLATFORM=wayland cargo run -p native-host
env ZGUI_PLATFORM=wayland cargo run -p native-host -- --exit-after-ms 5000
env ZGUI_PLATFORM=wayland cargo run -p native-host -- --bundle apps/examples/counter/dist
env ZGUI_PLATFORM=wayland cargo run -p native-host -- --bundle apps/examples/lists/dist --exercise --exit-after-ms 12000
env ZGUI_PLATFORM=wayland cargo test -p native-host native_tests::native_button_actions -- --ignored --exact --nocapture
```

`--smoke-module` executes the bundled Solid fixture in the actual embedded engine without creating a window. `--exit-after-ms` requests orderly event-loop shutdown, for native integration checks. The native baseline creates a dedicated Rust-owned mount region, installs a HostBinding, creates/updates native text from JavaScript, and schedules a native one-shot timer and an owned background completion with an explicit platform wake. These are integration harness behaviors, not the public SDK.

The build uses browser/production Solid export conditions and Babel's Universal preset before esbuild. Graph validation rejects unresolved imports and browser/server renderers and verifies one production reactive Solid runtime. Packages include composed source maps and build metadata. The separate smoke fixture imports upstream Solid declarations; actual native applications use `@zgui/ui`'s ES2022-only declaration boundary without DOM or Node globals.

## Native property and event contract

`@zgui/ui` exposes canonical native `class`, `id`, `title`, `tabIndex`, `disabled`, `role`, `ariaLabel`, `ariaDescription`, `style`, and `--custom` properties. Null/undefined clear values. Object styles accept the curated kebab-case keys declared in `NativeStyleName`, with string values and explicit units; CSS strings use native parser recovery. HTML aliases and listener-option tuples are rejected.

`Button` uses native control defaults, not JS keyboard synthesis. Native event props accept a function or null/undefined; a `Capture` suffix selects capture. `onFocus`/`onBlur` subscribe to bubbling `focusin`/`focusout`. Snapshots retain owned data and opaque node wrappers; cancellation methods expire when the synchronous handler returns. Pointerdown cancellation suppresses that event's focus default, not a later independent pointerup activation; cancelling pointerup suppresses its activation default. Event timestamps are monotonic milliseconds; pointer IDs are decimal strings and text-selection offsets are UTF-8 bytes.

Unexpected nested native notifications are queued without cancellation authority. Each Promise job gets a separate bounded structural turn; wake delivery requests a frame even when the last job empties the queue. Native public timer/rAF/services remain Phase 4 work.

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

Environment: Linux x86_64 Wayland session; niri compositor reports `26.04 (v26.04-298-gad20c318-modified)`, CLI `26.04 (8ed0da4)`; Wayland client 1.26.0, xkbcommon 1.13.2, Fontconfig 2.18.3, Vulkan 1.4.363. No performance claims are made from this baseline.

## Acceptance tracking

- [x] Phase 0: downstream window, script exception, real Solid execution, binding, wake and shutdown.
- [x] Phase 1: opaque handles, topology, turn-scoped invalidation and failed-construction reclamation.
- [x] Phase 2: native SDK declarations, TSX packages, reconciliation and examples.
- [x] Phase 3: properties, native cancellation, handler replacement and Button activation.
- [ ] Phase 4: bounded jobs, timer/rAF lifetimes, asynchronous requests and idle/occluded delivery.
- [ ] Phase 5: disposal, preflight, reload generations and fallback.
- [ ] Phase 6: controlled editing, clipboard, selection, Chinese IME and accessibility.
- [ ] Phase 7: layer-shell, monitor/scale/focus policy and typed desktop services.
- [ ] Phase 8: only optimizations justified by measured bottlenecks.
