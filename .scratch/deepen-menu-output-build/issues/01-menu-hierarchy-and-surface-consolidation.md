# 01: Consolidate menu hierarchy state, surface orchestration, and item builders

**What to build:** Consolidate root/submenu hierarchy state, `ContextMenu` anchor state, surface keyboard/focus/typeahead orchestration, and parameterized item builders into a new internal `packages/ui/src/components/menu-core.ts` module—retaining all 34 public `@hewel/zgui/components` menu exports and `docs/development/component-map.md` parity while deleting `WeakMap` globals, retiring `menu.ts`'s 10 internal exports, cascading leaf dismissal across nested `MenuSub` branches, and returning focus to `MenuSubTrigger` on submenu `Escape`.

**Blocked by:** none

**Status:** resolved

- [x] Internal `packages/ui/src/components/menu-core.ts` absorbs chained root/branch hierarchy context, `ContextMenu` anchor coordinate context, surface focus/typeahead/arrows orchestration (`trap: true` vs `trap: false`), and parameterized `menu` / `menubar` row, checkbox, radio, label, group, and separator builders.
- [x] All 10 internal-only exports from `packages/ui/src/components/menu.ts` (`MenuContext`, `MenuSurfaceContext`, `MenuRadioContext`, `menuRoot`, `createTypeaheadHandler`, `menuSurface`, `menuRow`, `menuIndicator`, `createMenuCheckedState`, `createMenuRadioValue`) are retired, and both module-global `WeakMap` side channels (`subClosings` in `menu-sub.ts` and `anchorState` in `context-menu.ts`) are deleted.
- [x] All 34 public menu/menubar/dropdown/context-menu/submenu exports in `packages/ui/src/components/index.ts` and `docs/development/component-map.md` parity remain intact, preserving `__zguiClass` StyleX forwarding and `MenuCheckboxItem` (`closeOnSelect = true`) vs `MenubarCheckboxItem` (`closeOnSelect = false`) defaults.
- [x] Pressing `Escape` inside an open `MenuSubContent` closes only the innermost open `MenuSub` and returns focus to its `MenuSubTrigger` (matching `ArrowLeft`), while `outside-press` dismisses without forcing focus onto `MenuSubTrigger` and root `MenubarContent` continues focusing `MenubarTrigger` only on `escape` or item dismissal.
- [x] Selecting a leaf item with `closeOnSelect` inside nested `MenuSub` branches cascades `closing.cancel()` and `state.setOpen(false)` from the innermost branch outward (firing controlled `MenuSub` `onOpenChange(false)` and starting exit transitions synchronously) before invoking root menu dismissal.
- [x] Build suites (`tests/build/menu.test.mjs`, `tests/build/menubar.test.mjs`, `tests/build/context-menu.test.mjs`, `tests/build/stylex-props.test.mjs`, `tests/build/component-map.test.mjs`), `pnpm lint`, `pnpm typecheck`, and GPU preview menu suites (`menu`, `menubar`, `context-menu`) pass after the final edit.

## Verification

- Optimized `mergeProps` in `packages/ui/src/renderer/index.ts` and removed redundant `mergeProps(props, { get children() ... })` root wrappers in `menuRoot` (`menu-core.ts`) and `ContextMenu` (`context-menu.ts`, merging `PlaceContext` + `AnchorContext` into one internal context) so `gallery.test.mjs` (`< 25 ms`) and `gallery-fuzz.test.mjs` (`< 30 ms`) stay comfortably within QuickJS entry-duration budgets (`AGENTS.md` §3).
