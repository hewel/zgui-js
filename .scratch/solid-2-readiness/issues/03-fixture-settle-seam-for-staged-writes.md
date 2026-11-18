# 03: Introduce fixture reactive settlement seam and remove intra-turn write-then-read coupling

**What to build:** Decouple component logic and `tests/build/` component suites from Solid 1.9's synchronous post-write read behavior by removing intra-turn write-then-read chains in component state helpers and routing test assertions through an explicit settlement helper in `tests/build/native-fixture.mjs`.

**Blocked by:** 01

**Status:** ready-for-agent

## Scope & Checklist

- [ ] Audit `packages/ui/src/components/` (starting with `state.ts`, `overlay.ts`, `menu-core.ts`, `menu-sub.ts`, `menubar.ts`, `hover.ts`, `tabs.ts`, `accordion.ts`, `collapsible.ts`, `groups.ts`, `slider.ts`, `roving.ts`) for any handler or reaction that writes a signal and reads it (or a derived memo) again in the same synchronous frame; compute and pass the `next` value explicitly instead.
- [ ] Add an explicit settlement API (e.g., `settle()` / `flushReactive()` or integrated `dispatchAndSettle()` alongside `createClock.settle()`) in `tests/build/native-fixture.mjs` that acts as a no-op / microtask drain under `1.9.19` today and will delegate to Solid 2.0 `flush()` on cutover.
- [ ] Update `tests/build/*.test.mjs` suites that dispatch synthetic native events (`dispatch(...)`, `focus(...)`, dismiss triggers, geometry callbacks) and immediately inspect native tree/lease state so they go through the fixture settlement seam rather than assuming zero-latency synchronous effect execution.
- [ ] Confirm `pnpm check` and `pnpm test:preview` pass cleanly on `1.9.19`.
