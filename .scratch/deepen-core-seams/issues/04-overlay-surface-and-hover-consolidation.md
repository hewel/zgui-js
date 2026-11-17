# 04: Consolidate internal overlay surface, exit deferral, and hover coordination

**What to build:** Consolidate exit animation deferral, same-turn QuickJS-stack-safe portal mounting, theme scoping, modal/anchored lease lifetimes, and hover-intent trigger coordination behind a single internal overlay-surface implementation and unified hover module—keeping all public UI exports unchanged, fixing controlled hover refocus `onOpenChange` churn, and removing the internal-package-import lint exception.

**Blocked by:** 03 (Complete the in-memory NativeHost test adapter and migrate build suites)

**Status:** ready-for-agent

- [ ] One internal overlay-surface module absorbs exit deferral (`data-state`, `isAnimating`, double-tick frame settle, 1000ms fallback), same-turn `createEffect` portal construction (preserving QuickJS stack budgets on initially open dialogs and pre-flush owner cancellation), theme re-scoping, and modal scrim vs `Popper` anchored lease coordination (`trapFocus`, `lockScroll`, `registerDismissLayer` with `leaving`).
- [ ] All public exports of `@hewel/zgui/components` and exported module-map / `testId` parity remain 100% intact.
- [ ] Shared overlay subparts (triggers, close controls, titles, descriptions) are unified rather than imported sideways between modal and popover modules or copy-pasted.
- [ ] Hover trigger, Escape dismissal, and intent handoff live inside the hover module so `Tooltip` and `HoverCard` share one state machine, and refocusing an already-open controlled hover surface calls `openNow()` without emitting a spurious `[false, true]` `onOpenChange` sequence.
- [ ] Build and GPU preview exit-deferral verification tests exercise public overlay modules (covering animated exit, zero-duration, no-motion, endless 1000ms timeout, and owner cancellation) instead of importing internal modules directly, and the internal-package-import override in the lint configuration is deleted.
- [ ] Any repo agent rule referencing retired internal overlay filenames is updated to reflect the consolidated internal module, and `pnpm check` plus preview overlay/presence suites pass after the final edit.
