# 04: Cut over compiler, universal renderer, UI components, services, and preview baselines to Solid 2.x

**What to build:** Execute the full migration from `solid-js@1.9.19` to Solid `2.x` (`solid-js`, `@solidjs/universal`, `@solidjs/babel-plugin`), adopting microtask-staged **Mount region** commits with narrow `flush()` boundaries, two-phase `createEffect(compute, apply)` owner discipline, diagnostics/production build split, `@hewel/zgui-services` async cancellation helpers, and resealed preview **Artifact inventory** baselines.

**Blocked by:** 01, 02, 03 (and gated on `.scratch/solid-2-readiness/spec.md` Upgrade Trigger Gate)

**Status:** needs-triage

## Scope & Checklist

### Dependencies, Compiler, & Build Validation
- [ ] Upgrade `solid-js` (`^2.0.x`), add `@solidjs/universal` and `@solidjs/babel-plugin`, and remove `babel-preset-solid` across `package.json`, `packages/ui/package.json`, and `packages/build/package.json` (`pnpm-lock.yaml`).
- [ ] Update `packages/build/src/compiler.mjs` and `packages/build/src/build.mjs`:
  - Switch JSX lowering preset/plugin from `babel-preset-solid` to `@solidjs/babel-plugin` (`generate: "universal"`, `moduleName: "@hewel/zgui/renderer"`).
  - Lock `build` and `preview` to `conditionNames: ["browser", "production", "import", "default"]` and `NODE_ENV: "production"`.
  - Add a `development` posture for `zgui-js dev` (and optional diagnostics mode) resolving `development` conditions and enabling `@solidjs/babel-plugin` `sourceNames` so reactive diagnostics log owner/location stacks to host `console`.
- [ ] Update `packages/build/src/validate.mjs` and `packages/build/src/smoke.mjs`:
  - Update `validateGraph` to accept Solid 2.0's split `@solidjs/signals/dist/prod/*` + `solid-js/dist/solid.js` (and `dev` variants when in `dev` mode) while still blocking `@solidjs/web` / `solid-js/web` / server runtimes.
  - Audit tree-shaking / gating of browser hydration globals (`window`, `document`, `fetch` in `solid-js/dist/solid.js`) in production bundles; ensure no hydration code executes or fails AST global checks.
  - Rewrite `tests/fixtures/solid-smoke.ts` and `packages/build/src/smoke.mjs` for 2.0 staged writes + `flush()`, updating the module-graph assertion in `smoke.mjs`.

### Renderer, Bridge Scheduling, & UI Primitives
- [ ] Rewrite `packages/ui/src/solid-native.d.ts` and `packages/ui/src/index.ts` as a clean-cut Solid 2.0 public surface (`createSignal`, `createMemo`, two-arg `createEffect`, `onSettled`, `onCleanup`, `untrack`, `flush`, `For`, `Show`, `createContext`, `useContext`, `omit`, `merge`, `Loading`, `Errored`, `Reveal`, `Repeat`, `createStore`, `createProjection`, `isPending`, `latest`, `refresh`, `action`) while keeping `NativeNode` / `NativeChild` branding intact.
- [ ] Migrate `packages/ui/src/renderer/index.ts` to `createRenderer` from `@solidjs/universal`:
  - Re-implement the `__zguiClass` / `__zguiStyle` / `__zguiInvariantStyle` multi-channel wrapper over Solid 2.0 `merge` (preserving compatibility with `viewOf` / `OmitView`).
  - Wrap `observeGeometry` callbacks in `packages/ui/src/nodes.ts` with `flush(() => callback(value))` so `deliver_observations()` in `vendor/zgui-runtime/src/window/frame.rs` commits relayout properties before paint.
  - Update `packages/ui/src/render-app.ts` to use `flush()` on mount/disposal and `runWithOwner(null, ...)` if detached roots are needed; remove `batch`.
- [ ] Migrate `packages/ui/src/components/shared.ts` `provide()` to call `context({ value, get children() { ... } })` directly (no `.Provider`) and update `no-raw-insert-or-provider` in `tools/oxlint-zgui-plugin.mjs` for `<Ctx value={...}>` syntax.
- [ ] Convert all `createEffect` call sites (35 sites across 15 files, including `overlay.ts`, `popper.ts`, `hover.ts`, `menu-core.ts`, `slider.ts`, etc.) to `createEffect(compute, apply)`:
  - Guard unchanged outputs with `createMemo` where re-running `apply` would churn native leases or animations.
  - Return cleanup functions from `apply` (`return observeGeometry(...)`, `return () => ...`) since `getOwner()` is `null` inside `apply`.
  - Fix `overlayPortal` (`packages/ui/src/components/overlay.ts`) by capturing `const owner = getOwner()` outside the effect and executing `provide(ElevationContext, ...)` inside `runWithOwner(owner, () => createRoot(dispose => ...))`, returning `dispose` from `apply`.
- [ ] Convert all remaining `splitProps` → `omit` (107 sites), `mergeProps` → `merge` (133 sites), `onMount` → `onSettled` (14 sites), `batch` removals (35 sites), and `<Index>` → `<For keyed={false}>` across `packages/ui/`, `apps/`, and `tests/`.

### Services, Native Host Budgets, & Verification
- [ ] Keep `readTextAsset(): HostRequest<T>` in `@hewel/zgui-services` (`packages/services/src/index.ts`) and add an owner-scoped Promise helper that invokes `request.cancel()` on `onCleanup` and catches/suppresses `"Cancelled"` rejections so cancelled reads do not fault strict preview **Session** runs under `RejectionPolicy::Strict`.
- [ ] Wire `tests/build/native-fixture.mjs` settlement helper to Solid 2.0 `flush()` (and verify `Session::checkpoint` drains microtasks cleanly in `crates/native-host/src/session.rs` and `editing_tests.rs` / `reload_tests.rs`).
- [ ] Re-verify QuickJS-NG stack (`256 KiB`), `entry_timeout` (`50 ms`), and `job_timeout` (`10 ms`) budgets (`crates/js-runtime/src/lib.rs`) on `apps/examples/gallery` and `apps/examples/async`.
- [ ] Update `solidAuditedSourceRevision` in `crates/native-host/src/preview/inventory.rs` and reseal all `preview/baselines/*` **Artifact inventory** snapshots.
- [ ] Run full verification: `pnpm check && pnpm typecheck:smoke && pnpm smoke && pnpm test:vendor && pnpm test:preview`.
