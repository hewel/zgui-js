# Coding Standards

Judgement-call engineering standards for code review and authoring. Mechanical rules (lint, typecheck, `styles.d.ts`↔`properties.rs` parity, `component-map.md` parity, Clippy) are enforced by `bun run check` and omitted here.

## Solid & Native Component Lifetimes

- **Owner-scoped native nodes**: Keep conditional native nodes inside the Solid owner controlling their presence. A removed node becomes invalid once its bridge turn completes; re-showing creates a fresh node, and memoizing independent content separately preserves state.
- **Same-turn overlay construction**: Build nested overlay content inside the same-turn `createEffect` in `packages/ui/src/components/overlay.ts` so eager Portal/provider construction stays within QuickJS stack budgets on initially open dialogs. Preserve initial-open and pre-flush owner-cancellation coverage when modifying overlays.
- **Batched root lifecycle**: Batch native root mounting and disposal so tree-order reactions settle once and teardown never re-registers events on retiring sessions. Coerce numeric array children to strings in `packages/ui/src/renderer/index.ts`.
- **Fix SDK boundary violations at the architecture layer**: When `bun run lint` or `packages/build/src/validate.ts` flags a boundary violation, solve it in the SDK/component seam rather than weakening rules or adding overrides.

## Non-DOM Native Host & Budgets

- **Bounded Rust array reads**: Read inbound native array lengths in Rust with a single bounded `u32` snapshot before allocation, without consulting mutable JavaScript globals.
- **Atomic inline styles**: A native `style` string replaces all inline declarations including `--*` custom properties. Emit dynamic styles and `--*` variables together or pass typed `NativeStyle` objects.
- **Runtime budgets**: Keep interactive trees and event handlers within QuickJS stack and entry-duration budgets (`--max-js-stack-bytes`, `metrics.json`).

## Scoped Styles & Upstream Provenance

- **Attribute-scoped part styles**: Scope component part styles to their own context-derived attributes whenever ancestor selectors could cross nested component roots, while preserving layout-wrapper composition.
- **Generated stylesheet refresh**: Refresh `packages/ui/src/components/generated/` by capturing `crates/ui-bridge/examples/ui_resources.rs` once and passing that snapshot to `packages/build/src/sync-ui-resources.ts`, preserving upstream license headers and provenance.

## Vendored Crates & Cross-Crate Invariants

- **Document every vendor patch**: Record non-trivial upstream fixes in `vendor/README.md` with the root cause and regression gate.
- **Atlas tile residency**: Any new derived `AtlasKey` namespace for shared images in `vendor/zgui-paint` must register residency in `ContentCache::image_tile_resident` and clean up in `remove_shared_tile` and `clear`, so preview asset readiness does not time out (`E_ASSET_TIMEOUT`).
- **Keep `tools/test-vendor.mjs` current**: Whenever adding a new integration test file in `vendor/*/tests/`, register it in `tools/test-vendor.mjs` so `bun run test:vendor` executes it.
- **Reuse shared runtime test harnesses**: Prefer extending `vendor/zgui-runtime/tests/support/mod.rs` (such as `support::capture` and `support::FrameRecorder`) over duplicating `Renderer`/`Platform` trait stubs in new integration tests.

## Verification Depth

- Verify component and rendering changes at the real boundary:
  - JS/renderer behavior via `tests/build/native-fixture.mjs` (detached-node invalidation, leases, `FocusIn`/`FocusOut` bubbling, closure-captured cancellation).
  - Visual, geometry, and multi-surface behavior via `tests/preview/harness.mjs` (remounts, Portal paint/hit escape, first-paint Popper geometry after entrance animations, styled-frame `overlay.ts` exit deferral, parked-pointer menu autofocus, settled theme/transition captures, and pixel/glyph rasterization).
- Prefer testing the painted/observable seam (display-list quads, PNG pixels, `scrollMetrics`) over internal planning state alone.
