# Native component development

## 1. Owner-Scoped Native Node Lifetimes

- Keep conditional native nodes inside the Solid owner that controls their presence. A removed node becomes invalid once its bridge turn completes; re-showing requires a fresh node, and memoizing independent content separately preserves state.
- Build nested overlay content inside the same-turn `createEffect` in `packages/ui/src/components/overlay.ts` so eager Portal/provider construction never overflows QuickJS stacks on initially open dialogs; retain initial-open and pre-flush owner-cancellation coverage.
- Batch native root mounting and disposal so tree-order reactions settle once and teardown does not re-register events on retiring sessions. Coerce numeric array children to strings in `packages/ui/src/renderer/index.ts`.

## 2. Enforced SDK Boundaries

- Architectural gates (`__zgui` encapsulation, `append`/`slot`/`provide` instead of raw `insert` or `<Context.Provider>`, themed `@hewel/zgui/components` primitives, no raw timers/`requestAnimationFrame`, and concise comments) are enforced by `.oxlintrc.json` + `tools/oxlint-zgui-plugin.mjs` (`bun run lint`) and bundle constraints in `packages/build/src/validate.ts`. Fix violations at the SDK/architecture layer rather than suppressing checks.

## 3. Non-DOM Native Host & Budgets

- Read inbound native array lengths in Rust with one bounded `u32` snapshot before allocation, without consulting mutable JavaScript globals.
- A native `style` string replaces all inline declarations including custom properties; emit dynamic styles and `--*` variables together or use typed style objects (`NativeStyle`).
- Keep interactive preview trees and handlers within QuickJS stack and entry-duration budgets (`--max-js-stack-bytes`, `metrics.json`).

## 4. Scoped Styles & Upstream Provenance

- Scope part styles to their own context-derived attributes when ancestor selectors can cross nested component roots while preserving layout-wrapper composition.
- Refresh `packages/ui/src/components/generated/` by capturing `crates/ui-bridge/examples/ui_resources.rs` once and passing that snapshot to `packages/build/src/sync-ui-resources.ts`, preserving upstream license and provenance.

## 5. Dual Verification Tiers

- `tests/build/native-fixture.mjs` implements all 23 `__zgui` ops with Bridge-turn detached-node invalidation, lease inspection, `FocusIn`/`FocusOut` bubbling, and closure-captured event cancellation; verify remounts, Portal paint/hit escape, first-paint Popper anchor geometry (checking settled surface/arrow gaps after entrance animations), styled-frame `overlay.ts` exit deferral, parked-pointer menu autofocus, settled theme/transition captures, and glyph rasterization (`tests/preview/vector-text.test.mjs`) through `tests/preview/harness.mjs`.
- Run vendored Rust checks with `cargo test -j 4 --manifest-path vendor/<crate>/Cargo.toml ... --offline` (`bun run test:vendor`); these crates are excluded from the root workspace.

## 6. Component Map & Vocabulary

- Update [`docs/development/component-map.md`](docs/development/component-map.md) whenever adding or renaming exports in `@hewel/zgui/components`, interactive `testId`s in `apps/examples/gallery`, or wrapped `generated/*Style.ts` sheets (`tests/build/component-map.test.mjs` enforces parity).

## Agent skills

### Issue tracker

Issues, specs, and wayfinding maps are tracked as GitHub Issues in `hewel/zgui-js` on GitHub Project #1 (`zgui-js`). See `docs/agents/issue-tracker.md`.

### Triage labels

Uses the default five-role vocabulary (`needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`). See `docs/agents/triage-labels.md`.

### Domain docs

Single-context layout: root `GLOSSARY.md` and `docs/adr/`. Read before exploring, designing, or naming domain concepts. See `docs/agents/domain.md`.
