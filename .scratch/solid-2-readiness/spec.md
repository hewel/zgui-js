# Solid 2.0 Readiness & Staged Migration Specification

**Status:** proposed

## Problem Statement

An audit of `solid-js@2.0.0`, `@solidjs/universal@2.0.0`, `@solidjs/signals@2.0.0`, and `@solidjs/babel-plugin@2.0.0` against the current `zgui-js` (`solid-js@1.9.19`) codebase identified four structural friction layers that make an immediate `2.0.0` cutover high-cost and high-risk:

1. **Scheduling & Bridge turn inversion**
   - Solid 2.0 removes `batch` (currently 35 occurrences across 22 files, including `packages/ui/src/render-app.ts` and `packages/ui/src/renderer/index.ts`) and stages signal writes until the microtask queue drains or `flush()` runs (`setX(1); x()` continues to return the prior value).
   - `zgui-js` assumes synchronous reactive commits inside a single **Bridge turn**:
     - Native input dispatch (`crates/native-host/src/session.rs`, exercised across multi-key controlled editing in `crates/native-host/src/editing_tests.rs`).
     - Intra-frame `deliver_observations()` (`vendor/zgui-runtime/src/window/frame.rs`), which runs *after* `binding.checkpoint()` in the same frame and expects `observeGeometry` callbacks (`packages/ui/src/components/popper.ts`) to commit layout properties before paint so first-paint Popper geometry lands without a one-frame lag.
     - Synchronous verification in `tests/fixtures/solid-smoke.ts` and direct `dispatch()` assertions in `tests/build/native-fixture.mjs`.

2. **Component & owner-lifetime contract shifts**
   - **`mergeProps` → `merge` (133 occurrences / 42 files)**: Solid 2.0 `merge` treats explicit `undefined` properties as overriding values rather than preserving earlier defaults, threatening components like `packages/ui/src/components/button.ts`, `packages/ui/src/components/overlay.ts`, and `packages/ui/src/text-control.ts`. In addition, `packages/ui/src/renderer/index.ts` wraps `renderer.mergeProps` to concatenate `__zguiClass` and merge `__zguiStyle` / `__zguiInvariantStyle` channels, which must adapt to `@solidjs/universal` `merge` (`viewOf` / `OmitView` internals).
   - **`createContext` nullability & Provider removal (42 definitions / 18 files, 62 reads / 20 files)**: In Solid 2.0, `createContext<T>()` without a default value throws `ContextNotFoundError` when `useContext` runs outside a provider. All component contexts in `packages/ui/src/components/` currently omit default values even when callers perform optional lookups (`ThemeProvider`/`Internal`, `HeaderContext`, `ItemContext`, `ElevationContext`, `OverlayContext`, `Group`, `BarContext`, `MenuHierarchyContext`, `MenuSurfaceContext`, `MenuRadioContext`, `ContextMenuContext`). `Context.Provider` is also removed in favor of `<Ctx value={...}>` / `Ctx({ value, ... })`, impacting `provide()` in `packages/ui/src/components/shared.ts` and `no-raw-insert-or-provider` in `tools/oxlint-zgui-plugin.mjs`.
   - **`createEffect(compute, apply)` owner & equality changes (35 occurrences / 15 files)**:
     - `compute` return values are not deduplicated by equality; unchanged primitive outputs still re-run `apply` and cleanups unless guarded by `createMemo`.
     - `@solidjs/signals` `runEffect` executes `apply` with `getOwner() === null`: leases (`observeGeometry`, `trapFocus`, `lockScroll`, `registerDismissLayer` in `packages/ui/src/nodes.ts`) must be cleaned up by returning their disposer from `apply` (`return observeGeometry(...)`), and `overlayPortal` in `packages/ui/src/components/overlay.ts` must capture its parent owner via `getOwner()` and construct its portal subtree inside `runWithOwner(parentOwner, () => createRoot(dispose => ...))`, returning `dispose` from `apply` so closed overlays never leak native portal nodes.
   - **API removals & curated SDK types**: `splitProps` → `omit` (107 / 37 files), `onMount` → `onSettled` (14 / 8 files), `Index` → `<For keyed={false}>` (8 / 4 files), plus `packages/ui/src/index.ts` and `packages/ui/src/solid-native.d.ts` (preserving `NativeNode` / `NativeChild` brand enforcement verified by `tests/types/native-negative.tsx`).

3. **Compiler, bundler, and QuickJS global/graph validation gates**
   - Cutover replaces `babel-preset-solid` with `@solidjs/babel-plugin@2.0.0` and `solid-js/universal` with `@solidjs/universal@2.0.0` (`packages/build/src/compiler.mjs`, `packages/build/src/build.mjs`).
   - Scanning published `2.0.0` bundles against `packages/build/src/validate.mjs`'s QuickJS global allowlist surfaces `Node` in `@solidjs/signals/dist/prod/store/types.js` (pulled via `$PROXY` in `@solidjs/universal`, guarded with `typeof Node === "undefined"`) and `fetch`, `window`, `document` inside `solid-js/dist/solid.js` hydration helpers.
   - `validateGraph` (`packages/build/src/validate.mjs`) and `packages/build/src/smoke.mjs` require `solid-js` to resolve as a single `/solid-js/dist/solid.js` module (and `inputs.length === 2` for smoke), whereas 2.0 splits reactivity across `solid-js` and `@solidjs/signals/dist/prod/*`.

4. **Async cancellation, Diagnostics posture, and QuickJS / preview budgets**
   - `HostRequest.cancel()` in `packages/services/src/index.ts` and `crates/native-host/src/work.rs` rejects pending Promises with `Error` (`code: "Cancelled"`), which trips `RejectionPolicy::Strict` in preview **Session** runs (`crates/native-host/src/session.rs`) if unhandled when an owner disposes mid-flight under `<Loading>`.
   - `crates/js-runtime/src/lib.rs` enforces strict QuickJS-NG budgets (`stack_limit_bytes: 256 KiB`, `entry_timeout: 50 ms`, `job_timeout: 10 ms`, `max_jobs_per_pump: 256`); `crates/native-host/src/preview/inventory.rs` pins `solidAuditedSourceRevision`; `preview/baselines/*` seals output artifacts and **Artifact inventory** digests.

## Solution

Adopt a two-phase posture recorded in [ADR-0001](../../docs/adr/0001-solid-2-migration-posture.md):

1. **Phase 1 — 1.9.19 Pre-Convergence (Immediate, non-breaking):**
   - Prune `batch`, `Index`, and `splitProps` from `@hewel/zgui` public root exports (`packages/ui/src/index.ts`, updating `apps/examples/lists/src/main.tsx` and `tests/types/`).
   - Convert all optional `createContext<T>()` definitions in `packages/ui/src/components/` to explicit `createContext<T | null>(null)` while leaving mandatory provider contexts strict.
   - Refactor `mergeProps(defaults, props)` usage across `packages/ui/src/` so defaults are applied via `??` at read sites rather than relying on `undefined`-skipping object merges.
   - Add `tools/oxlint-zgui-plugin.mjs` + `.oxlintrc.json` guardrails against deprecated 1.x APIs and intra-turn write-then-read signal access; relax `packages/build/src/validate.mjs` so safe `typeof UnknownGlobal` checks are permitted while value references remain blocked.
   - Introduce an explicit settlement seam in `tests/build/native-fixture.mjs` and eliminate intra-turn write-then-read assumptions in component handlers (`createControllable`, etc.).

2. **Phase 2 — Solid 2.0.x Cutover (Gated on Upgrade Trigger):**
   - Execute only once the Upgrade Trigger Gate is met.
   - Switch dependencies and compiler pipeline to `solid-js@2.x`, `@solidjs/universal`, and `@solidjs/babel-plugin`.
   - Adopt Solid 2.0's native microtask-staged commit model (draining **Mount region** updates in `Session::checkpoint`), keeping synchronous `flush()` strictly to:
     1. Initial `renderApp` mount (`packages/ui/src/render-app.ts`),
     2. `observeGeometry` callbacks (`packages/ui/src/nodes.ts`), preserving single-frame Popper layout and virtual-list arrivals inside `vendor/zgui-runtime/src/window/frame.rs`,
     3. Explicit fixture settlement in `tests/build/native-fixture.mjs`.
   - Clean-cut `@hewel/zgui` exports to Solid 2.0 primitives (`createEffect(compute, apply)`, `onSettled`, `omit`, `merge`, `flush`, `Loading`, `Errored`, `Reveal`, `Repeat`, `createStore`, `createProjection`, `isPending`, `latest`, `refresh`, `action`) with zero 1.x compatibility aliases, keeping `NativeNode` / `NativeChild` branding in `packages/ui/src/solid-native.d.ts`.
   - Split build postures: `zgui-js dev` (and explicit diagnostics mode) enables `development` condition + `@solidjs/babel-plugin` `sourceNames` emitted to host `console`; `build` and sealed `preview` lock `production` condition.
   - Keep `readTextAsset(): HostRequest<T>` in `@hewel/zgui-services` and add an owner-scoped Promise helper that calls `cancel()` on `onCleanup` and absorbs `"Cancelled"` rejections cleanly under `RejectionPolicy::Strict`.
   - Re-audit QuickJS stack/entry budgets, update `solidAuditedSourceRevision`, and reseal preview baselines.

## Upgrade Trigger Gate

Do **not** begin Issue 04 (Solid 2.0.x cutover) until **at least one** of the following holds:

1. Upstream Solid 2.0 has stabilized on a `2.0.x` patch series without critical universal-renderer/scheduler regressions, **or**
2. A concrete product requirement in `zgui-js` needs Solid 2.0's async reactive graph (`<Loading>` / `<Errored>`) or built-in draft stores (`createStore` / `createProjection`).

## Implementation Decisions

| Area | Phase 1 (1.9.19) | Phase 2 (2.0.x Cutover) |
| --- | --- | --- |
| Public SDK surface | Keep `@hewel/zgui` sole entry; remove `batch`, `Index`, `splitProps` | Clean-cut 2.0 primitives + `NativeNode` types; no 1.x aliases |
| Scheduling & `flush()` | No intra-turn write-then-read in handlers; fixture settle seam | Microtask commits in `Session::checkpoint`; `flush()` only in `renderApp` mount, `observeGeometry`, fixture settle |
| Context & `provide()` | Optional contexts become `createContext<T \| null>(null)` | `provide()` calls `context({ value, children })`; `oxlint` updated for `<Ctx value={...}>` |
| Effects & Portals | Document owner contract; no raw `createEffect` in apps | Convert to `createEffect(compute, apply)` with returned lease cleanup; `overlayPortal` uses captured parent owner + `createRoot` returning `dispose` |
| Props & Defaults | Resolve defaults via `??` at read sites instead of `mergeProps` fallback | Replace `mergeProps`/`splitProps` with `merge`/`omit`; preserve `__zguiClass`/`__zguiStyle` multi-channel Proxy |
| `@hewel/zgui-services` | Keep `readTextAsset(): HostRequest<T>` unchanged | Keep `HostRequest<T>` + add cleanup-cancelling Promise helper swallowing `"Cancelled"` |
| Build & Diagnostics | Allow `typeof UnknownGlobal` in `validate.mjs` | `dev` enables `development` + `sourceNames`; `build`/`preview` lock `production`; update `validateGraph` & `smoke.mjs` |

## Verification Matrix

- **Phase 1 (Issues 01–03):**
  - `pnpm check` (`pnpm lint && pnpm typecheck && pnpm test:build && pnpm test:rust`)
  - `pnpm typecheck:smoke && pnpm smoke`
  - `pnpm test:preview` (confirming zero behavioral drift on `1.9.19`)
- **Phase 2 (Issue 04, on cutover):**
  - All Phase 1 gates plus `pnpm test:vendor`
  - QuickJS-NG stack/entry/job telemetry checks (`--max-js-stack-bytes`, `metrics.json`) across `apps/examples/gallery`, `apps/examples/async`, and `crates/native-host` editing/reload integration tests (`editing_tests.rs`, `reload_tests.rs`)
  - `crates/native-host` `inventory.rs` lockfile/revision audit and resealed `preview/baselines/*` **Artifact inventory** verification
