# 02: Harden mergeProps default hygiene, add oxlint guardrails, and allow typeof probes

**What to build:** Eliminate implicit reliance on `mergeProps(defaults, props)` skipping explicit `undefined` values across UI components, add `oxlint` rules preventing new Solid 1.x debt and intra-turn write-then-read signal bugs, and relax `validate.mjs` so defensive `typeof UnknownGlobal` checks are allowed in QuickJS bundles while bare value references remain forbidden.

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

## Scope & Checklist

- [ ] Audit all `mergeProps` call sites across `packages/ui/src/` (133 occurrences in 42 files, including `button.ts`, `text-control.ts`, `components/button.ts`, `components/overlay.ts`, `components/popper.ts`, `components/shared.ts`, `components/slider.ts`, `components/tabs.ts`, `components/groups.ts`, etc.) so any prop default (`tabIndex`, `variant`, `size`, `placement`, `offset`, `role`, etc.) resolves via `??` at property/getter read sites rather than assuming `undefined` properties in `props` will fall back to an earlier object.
- [ ] Add build tests under `tests/build/` verifying explicit `undefined` props (e.g. `<Button variant={undefined} size={undefined} tabIndex={undefined} />`) retain expected defaults under `1.9.19` today and are protected against Solid 2.0 `merge` semantics.
- [ ] Extend `tools/oxlint-zgui-plugin.mjs` and `.oxlintrc.json`:
  - Disallow importing removed 1.x patterns (`Index`, `createResource`, `Suspense`, `ErrorBoundary`, `classList`, `on:`, `attr:`, `use:`) and disallow new `batch` usage outside `packages/ui/src/render-app.ts`, `packages/ui/src/renderer/index.ts`, and remaining internal component migrations.
  - Disallow default-less `createContext<T>()` in `packages/ui/src/components/` unless explicitly audited/whitelisted as mandatory.
  - Warn/error on obvious same-block synchronous write-then-read of the same signal (`setFoo(...); foo()`).
- [ ] Update `ReferencedIdentifier` validation in `packages/build/src/validate.mjs` so `typeof UnknownGlobal` (`UnaryExpression` with `operator === "typeof"` whose immediate `argument` is the identifier) does not trigger `Unsupported runtime global`, while bare reads (`window.fetch`, `document.getElementById`, `Node`, `fetch(...)`) still fail validation.
- [ ] Add regression tests in `tests/build/compiler.test.mjs` covering both permitted `typeof Node === "undefined"` probes and rejected bare globals (`document`, `window`, `Node`, `fetch`), then run `pnpm check`.
