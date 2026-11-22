---
name: zgui-dev
description: Develop native UI apps and Wayland desktop shells with @hewel/zgui, @hewel/zgui/primitives, @hewel/zgui/router, @hewel/zgui-services, @hewel/zgui-cli, and @hewel/zgui-build. Use when building TSX screens, CSS Modules (*.module.css) styles, shell surfaces (LayerSurface, PopupSurface, SessionLockSurface, WindowSurface), host services (HTTP/SSE/WS, SQLite, local files, clipboard, Unix sockets, commands, control messages, workers, image handles), vector/spring animations, or configuring zgui.capabilities manifests.
---

Guide for authoring native QuickJS-NG + Solid 2.0.1 applications and Wayland desktop shells targeting `zgui-js`. For the full machine-checked export list, read [`llms.txt`](../../../llms.txt).

## Architecture & runtime model

- **Host, not browser**: bundles run in QuickJS-NG (ES2022) driving Rust native nodes via `@hewel/zgui` (no DOM or browser globals).
- **Mandatory module entrypoint**: `src/main.tsx` must export `export const hostApiVersion = 1;` and `export function mount(root: NativeNode): () => void { return renderApp(() => <App />, root); }` (or `renderShell(() => <Shell />, root)` for `--pure-shell`).
- **Owner-scoped native lifetimes**: construct conditional native nodes inside the Solid owner that controls their visibility (`<Show>`, `<For>`, `<Switch>`). Removing a node invalidates its handle at the end of the turn; re-showing creates a fresh node, so keep stateful signals/memos above the branch or memoize independent subtrees separately.
- **Ref-bound leases**: wire node-bound leases (`observeGeometry`, `trapFocus`, `lockScroll`, `registerDismissLayer`) to a signal accessor (`const [el, setEl] = createSignal<NativeNode | null>(null); observeGeometry(el, ...); <scroll ref={setEl} />`) in the component body instead of inside JSX `ref` callbacks.
- **Solid 2.0 state tiers & batching**: keep Solid 2.0 as the sole client state model (`createSignal` for local UI state → `createContext` for shared subtree state → `createStore` / `createProjection` / `reconcile` for structured or keyed collections, with own-slot `Draft<T>` and void `reconcile`). Reading a signal immediately after `setSignal(v)` inside the same `createEffect` returns the previous value; derive follow-up work from `v` or use functional updaters. Use `createKvState` for single-key versioned KV persistence (note: async `KvState.flush()` is a disk durability barrier, distinct from `@hewel/zgui`'s synchronous Solid render `flush()`) and `createHostQuery` for replaceable cancellable `HostRequest<T>` reads from `@hewel/zgui-services`.
- **Mechanical guardrails live in Oxlint**: downstream apps should extend `@hewel/zgui/oxlint/recommended` (`packages/ui/oxlint/recommended.json`) so bridge encapsulation, Solid/ref-scope hygiene, top-level service calls, raw timers/rAF, and themed component imports are enforced deterministically at lint time.

## Development workflow

1. **Scaffold & declare the application package** (`package.json`):
   - Scaffold new apps with `zgui-js init [dir] --template desktop-app|wayland-shell` (or `bun create zgui-js`).
   - Set `"zgui": { "id": "<app-id>", "window": { ... }, "capabilities": { ... }, "fonts": [...] }`.
   - Declare only the capabilities the app genuinely uses; undeclared host operations fail closed with `E_CAPABILITY`.
   - For capability schemas, path templates, host services (HTTP/SSE/WS, SQLite/KV, local files/bookmarks, clipboard, Unix sockets, commands, control messages, compute workers), and programmatic builds, read [`references/services-and-capabilities.md`](references/services-and-capabilities.md).
2. **Choose the surface entry point**:
   - Standard window app: `renderApp(() => <App />, root)` from `@hewel/zgui` (preserve dev state across reloads via `createDevState` and `@hewel/zgui/router`).
   - Pure Wayland desktop shell (no default `main` window): `renderShell(() => <Shell />, root)` requiring `capabilities.shellSurfaces: true` (or `sessionLock: true`) and `--pure-shell`.
   - For multi-output bars (`LayerSurface`), anchored popovers (`PopupSurface`, including `geometryInsets` outer shadow bleed via `xdg_surface.set_window_geometry`), secondary windows (`WindowSurface`), input regions, declarative compositor background blur (`blurRegion` via Wayland `ext_background_effect_v1`, read-only reactive `getBackgroundBlurSupported()`, excluded from `SessionLockSurface`), and lockscreens (`SessionLockSurface`), read [`references/shell-surfaces.md`](references/shell-surfaces.md).
3. **Author UI, styles, and motion**:
   - Use semantic native tags (`<box>`, `<row>`, `<column>`, `<scroll>`, `<text>`, `<vector>`), windowed `<VirtualList>`/`<VirtualGrid>` for large collections, themed components from `@hewel/zgui/components`, and unstyled behavior primitives from `@hewel/zgui/primitives` (`createPresence`, `createRetainedPresence` for CSS + spring-`settled` exit retention, `createVirtualFocus` for data-driven 1D/2D virtualized lists and grids with `ariaActiveDescendant` + IME/caret-safe key routing, `RadioGroupPrimitive`/`RadioItemPrimitive` for unstyled roving-tabindex radio groups with arbitrary children, Overlay/Roving/Menu — ideal for custom `<PopupSurface>` shell menus and headless motion without `Portal`/`Popper`/`MenuStyle` coupling). Use scalar `ariaActiveDescendant?: NativeNode | null | undefined` for virtual focus on comboboxes/listboxes (sibling options supported; rejects arrays and raw text nodes atomically) and guard custom keyboard handlers with `if (event.isComposing) return;` using `NativeKeyEvent.isComposing` (capture-phase Escape dismiss also protects active IME preedit automatically).
   - Author application styles in `src/app.css` and locally scoped `*.module.css` (compiled by Lightning CSS with `--zui-*` tokens and `[data-*]` state selectors), compose classes with `cx(...)` on `class`, and reserve typed `NativeStyle` objects for continuous dynamic values. Whenever emitting a dynamic `style` string, include all inline declarations and `--*` custom properties together in that string.
   - For CSS Modules rules, `createSpring` (`initial`, `settled`, `jump`, `impulse`, `configure`, shared per-bridge rAF ticker), `createRetainedPresence`, `<vector>` (`svg` / `createCanvasScene`), and wheel/scroll handling, read [`references/styling-and-motion.md`](references/styling-and-motion.md).
4. **Build, inspect, package, lint, and typecheck**:
   - Target `@hewel/zgui/tsconfig.native.json` (or `"jsxImportSource": "@hewel/zgui"`).
   - CLI workflow (`@hewel/zgui-cli` / `zgui-js`): `zgui-js doctor`, `zgui-js dev [--dev-inspect SOCK]`, `zgui-js build [--bytecode]`, `zgui-js inspect [--ui] --socket SOCK [--op tree|profile | pick]`, `zgui-js package --out DIR [--bytecode]`.
   - Programmatic API (`@hewel/zgui-build`): `buildApplication`, `watchApplication`, `devApplication`, `previewApplication`, `packageApplication`.
5. **Verify visually in headless preview**:
   - Hand off to the `zgui-preview` skill to verify rendered layout, alpha transparency, multi-surface lifecycle, and clean resource teardown (`teardown.clean === true`).

## Completion criteria

- `oxlint` (with `@hewel/zgui/oxlint/recommended`), `tsrs` typecheck, and bundle validation pass cleanly.
- Every resource opened inside a Solid scope is released on owner cleanup.
- Headless preview (`zgui-preview`) passes assertions, pixel checks, and reports zero leaked teardown resources.
