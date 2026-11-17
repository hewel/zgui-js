# 03: Complete the in-memory NativeHost test adapter and migrate build suites

**What to build:** Complete the in-memory JavaScript `NativeHost` test adapter to satisfy all 23 `__zgui` operations, lease inspection, `FocusIn`/`FocusOut` and input event bubbling, strict **Bridge turn** stale-node guards, and deterministic virtual time, and replace the duplicated host patches, fake clocks, and direct callback pokes across the 15 build test suites.

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [ ] The in-memory host test adapter implements all 23 `__zgui` methods (adding `createPortal`, `registerDismissLayer`, `trapFocus`, `lockScroll`, `isAnimating`, and `observeGeometry`) with inspectable active/historical lease records, while preserving lazy global host resolution during module import (preflight checks remain red if modules touch host globals eagerly).
- [ ] `focus()` models `FocusIn`/`FocusOut` bubbling across ancestors, and bubbling input/key dispatch supports closure-captured cancellation (`preventDefault`, `stopPropagation`, `stopImmediatePropagation`) that works cleanly alongside shallow event-object spreading.
- [ ] Strict **Bridge turn** lifecycle semantics invalidate detached node handles at turn end (rejecting stale mutations after a turn settles) and reject new allocations once teardown begins, matching the Rust bridge contract.
- [ ] One shared deterministic virtual clock (`advance` + `frame`) replaces all 8 duplicated `fakeClock` copies across build test suites, and all ~750 lines of per-file `__zgui` monkey-patches are deleted.
- [ ] Build test suites that previously called `node.callbacks` directly on container nodes (including tabs and disclosure suites) drive target/focused nodes through bubbling dispatch instead, verifying automatic tab activation triggers `FocusIn` as well as `focused.current`.
- [ ] `pnpm lint`, `pnpm typecheck`, and `pnpm test:build` pass completely after the final edit.
