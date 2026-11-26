# Coding Standards

Judgement-call engineering standards for code review and authoring. Mechanical rules—formatting (`oxfmt`), lint (`oxlint` + `@hewel/zgui/oxlint`), strict `exactOptionalPropertyTypes` typechecking, Lightning CSS grammar (`packages/build/src/css.ts`), numeric child coercion (`tests/build/renderer-children.test.mjs`), `styles.d.ts`↔`properties.rs` parity, `component-map.md` / `llms.txt` parity, `sync-ui-resources.ts` regeneration parity, `tools/test-vendor.mjs` registration parity, zero-stylesheet primitives, and Clippy—are enforced by `bun run check` and omitted here.

## Solid 2.0 & Native Component Lifetimes

- **Owner-scoped native nodes**: Keep conditional native nodes inside the Solid owner controlling their presence. A removed node becomes invalid once its bridge turn completes; re-showing creates a fresh node, and memoizing independent content separately preserves state.
- **Solid 2.0 effect batching, dev-mode scopes & callback signatures**: Reading a signal synchronously right after `setSignal(v)` inside the same `createEffect` returns the previous value—derive follow-up work from the local value or a reactive effect instead. Wrap intentional non-tracking reads in component setup, `onSettled`, or the second callback of `createEffect(compute, effect)` in `untrack(...)`, and never invoke parent signal setters synchronously inside `children` render callbacks or `createMemo`. Declare interface callbacks as function properties (`readonly fn: (...) => ...`) rather than method syntax so callers satisfy `typescript(unbound-method)`.
- **Same-turn overlay construction**: Build nested overlay content inside the same-turn `createEffect` in `packages/ui/src/components/overlay.ts` so eager Portal/provider construction stays within QuickJS stack budgets on initially open dialogs. Preserve initial-open and pre-flush owner-cancellation coverage when modifying overlays.
- **Batched root lifecycle**: Batch native root mounting and disposal so tree-order reactions settle once and teardown never re-registers events on retiring sessions.
- **Fix SDK boundary violations at the architecture layer**: When `bun run lint` or `packages/build/src/validate.ts` flags a boundary violation, solve it in the SDK/component seam rather than weakening rules or adding overrides.

## Non-DOM Native Host, Overlays & Budgets

- **Bounded Rust array reads**: Read inbound native array lengths in Rust with a single bounded `u32` snapshot before allocation, without consulting mutable JavaScript globals.
- **Atomic inline styles**: A native `style` string replaces all inline declarations including `--*` custom properties. Emit dynamic styles and `--*` variables together or pass typed `NativeStyle` objects.
- **Host-owned dev overlays & lock-surface isolation**: Inserting a host-owned overlay (such as `PickerOverlay` or `DevBanner`) increments `window.dom().revision()`; ensure same-frame hover→click hit-testing tolerates the overlay revision bump, never evaluate app JS getters, redact secure inputs, and hard-refuse dev overlays/inspection on `SessionLockSurface`.
- **QuickJS bytecode compilation contract**: `zgui-host compile-bytecode` must keep `strip_debug: false` so sidecar `.mjs.map` stack remapping remains accurate, and pass a contained `--source-dir` so Rolldown-split static sibling chunks resolve deterministically.
- **Runtime budgets**: Keep interactive trees and event handlers within QuickJS stack and entry-duration budgets (`--max-js-stack-bytes`, `metrics.json`).

## Scoped Styles & Upstream Provenance

- **Unified Lightning CSS Modules + `--zui-*` + `[data-*]`**: Author application styles in global `src/app.css` and locally scoped `*.module.css` files (`lightningcss-v1`), composing static classes with `cx(...)` onto `class` and reserving inline `style={{ ... }}` for continuous dynamic values.
- **Stylo hot-path attribute discipline**: On styled `@hewel/zgui/components` rows (menus, lists), rely on native CSS pseudo-classes (`:focus`, `:hover`) rather than toggling extra `data-*` attributes on every pointer/key move; reserve `data-highlighted` hooks for headless `@hewel/zgui/primitives`.
- **Attribute-scoped part styles**: Scope component part styles to their own context-derived attributes whenever ancestor selectors could cross nested component roots, while preserving layout-wrapper composition. Put all upstream CSS adaptations (including `TextareaStyle`, `TabsStyle`, `AccordionStyle`, `CollapsibleStyle`) in `packages/build/src/sync-ui-resources.ts` so regeneration and `#49` static `components.css` hoisting emit identical rules.
- **Generated stylesheet refresh**: Refresh `packages/ui/src/components/generated/` from `crates/ui-bridge/examples/ui_resources.rs` through `packages/build/src/sync-ui-resources.ts`, preserving upstream license headers and provenance.

## Vendored Crates & Cross-Crate Invariants

- **Document every vendor patch**: Record non-trivial upstream fixes in `vendor/README.md` with the root cause and regression gate (`tests/build/vendor-suites.test.mjs` checks that cited test files are registered in `tools/test-vendor.mjs`).
- **Atlas tile residency**: Any new derived `AtlasKey` namespace for shared images in `vendor/zgui-paint` must register residency in `ContentCache::image_tile_resident` and clean up in `remove_shared_tile` and `clear`, so preview asset readiness does not time out (`E_ASSET_TIMEOUT`).
- **Reuse shared runtime test harnesses**: Prefer extending `vendor/zgui-runtime/tests/support/mod.rs` (such as `support::capture` and `support::FrameRecorder`) over duplicating `Renderer`/`Platform` trait stubs in new integration tests.

## Verification Depth

- Verify component and rendering changes at the real boundary:
  - JS/renderer behavior via `tests/build/native-fixture.mjs` (detached-node invalidation, leases, `FocusIn`/`FocusOut` bubbling, closure-captured cancellation).
  - Visual, geometry, and multi-surface behavior via `tests/preview/harness.mjs` (remounts, Portal paint/hit escape, first-paint Popper geometry after entrance animations, styled-frame `overlay.ts` exit deferral, parked-pointer menu autofocus, settled theme/transition captures, and pixel/glyph rasterization).
- Prefer asserting the painted/observable seam users actually see (layout bounds, display-list quads, PNG pixels, `scrollMetrics`) over internal planning state or zero-assertion captures alone.
