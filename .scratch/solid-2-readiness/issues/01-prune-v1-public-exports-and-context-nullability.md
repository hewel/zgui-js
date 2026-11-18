# 01: Prune 1.x-only public exports and make optional contexts explicitly nullable

**What to build:** Remove `batch`, `Index`, and `splitProps` from `@hewel/zgui`'s public root exports while still on `solid-js@1.9.19`, migrate the single app usage in `apps/examples/lists/src/main.tsx`, and convert every optional `createContext<T>()` in `packages/ui/src/components/` to `createContext<T | null>(null)` so Solid 2.0's `ContextNotFoundError` rule cannot break unprovided reads later.

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

## Scope & Checklist

- [ ] Remove `batch`, `Index`, and `splitProps` from `packages/ui/src/index.ts` and `packages/ui/src/solid-native.d.ts` (component internals continue using `#solid` during `1.9.19`).
- [ ] Update `apps/examples/lists/src/main.tsx` to replace `<Index>` and `batch(...)` with `<For>` and direct signal writes compatible with the public `@hewel/zgui` API.
- [ ] Update `tests/types/native-positive.tsx` and `tests/types/native-negative.tsx` (asserting `batch`, `Index`, and `splitProps` are rejected as public `@hewel/zgui` imports alongside `createDeferred`/`createRoot`), and update `docs/development/component-map.md` if any tracked items are affected (`tests/build/component-map.test.mjs`).
- [ ] Audit every `createContext` definition across `packages/ui/src/components/` (`theme.ts`, `card.ts`, `item.ts`, `groups.ts`, `tabs.ts`, `collapsible.ts`, `accordion.ts`, `overlay.ts`, `hover.ts`, `tooltip.ts`, `menubar.ts`, `menu-core.ts`) and classify each as:
  - **Optional outside provider** (e.g. `Internal` theme, `HeaderContext`, `ItemContext`, `ElevationContext`, `OverlayContext`, `Group`, `BarContext`, `MenuHierarchyContext`, `MenuSurfaceContext`, `MenuRadioContext`, `ContextMenuContext`, etc.) -> declare explicitly as `createContext<T | null>(null)` and narrow `?? undefined` / `null` checks at `useContext` callers.
  - **Mandatory inside provider** -> preserve strict typing and add/verify unit tests for missing-provider behavior where appropriate.
- [ ] Run `pnpm check`, `pnpm typecheck:smoke`, and `pnpm test:preview` and confirm all suites pass without regressions.
