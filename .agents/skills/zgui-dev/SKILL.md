---
name: zgui-dev
description: Develop native UI apps and Wayland desktop shells with @hewel/zgui, @hewel/zgui-services, and @hewel/zgui-build. Use when building TSX screens, StyleX styles, shell surfaces (LayerSurface, PopupSurface, SessionLockSurface, WindowSurface), host services (Unix sockets, commands, control messages, image handles), vector/spring animations, or configuring zgui.capabilities manifests.
---

Guide for authoring native QuickJS-NG + Solid 2.0.1 applications and Wayland desktop shells targeting `zgui-js`.

## Architecture & runtime model

- **Host, not browser**: bundles run in QuickJS-NG (ES2022) driving Rust native nodes via `@hewel/zgui` (no DOM or browser globals).
- **Owner-scoped native lifetimes**: construct conditional native nodes inside the Solid owner that controls their visibility (`<Show>`, `<For>`, `<Switch>`). Removing a node invalidates its handle at the end of the turn; re-showing creates a fresh node, so keep stateful signals/memos above the branch or memoize independent subtrees separately.
- **Ref-bound leases**: wire node-bound leases (`observeGeometry`, `trapFocus`, `lockScroll`, `registerDismissLayer`) to a signal accessor (`const [el, setEl] = createSignal<NativeNode | null>(null); observeGeometry(el, ...); <scroll ref={setEl} />`) in the component body instead of inside JSX `ref` callbacks.
- **Mechanical guardrails live in Oxlint**: downstream apps should extend `@hewel/zgui/oxlint/recommended` (`packages/ui/oxlint/recommended.json`) so bridge encapsulation, Solid/ref-scope hygiene, top-level service calls, raw timers/rAF, and themed component imports are enforced deterministically at lint time.

## Development workflow

1. **Declare the application package** (`package.json`):
   - Set `"zgui": { "id": "<app-id>", "window": { ... }, "capabilities": { ... }, "fonts": [...] }`.
   - Declare only the capabilities the app genuinely uses; undeclared host operations fail closed with `E_CAPABILITY`.
   - For capability schemas, path templates, host services, and programmatic builds from external repos, read [`references/services-and-capabilities.md`](references/services-and-capabilities.md).
2. **Choose the surface entry point**:
   - Standard single-window app: `renderApp(() => <App />)` from `@hewel/zgui`.
   - Pure Wayland desktop shell (no default `main` window): `renderShell(() => <Shell />)` requiring `capabilities.shellSurfaces: true` (or `sessionLock: true`) and `--pure-shell`.
   - For multi-output bars (`LayerSurface`), anchored popovers (`PopupSurface`), secondary windows (`WindowSurface`), input regions, and lockscreens (`SessionLockSurface`), read [`references/shell-surfaces.md`](references/shell-surfaces.md).
3. **Author UI, styles, and motion**:
   - Use semantic native tags (`<box>`, `<row>`, `<column>`, `<scroll>`, `<text>`, `<vector>`), windowed `<VirtualList>`/`<VirtualGrid>` for large collections, and themed components from `@hewel/zgui/components`.
   - Prefer StyleX (`@stylexjs/stylex`) or typed `NativeStyle` objects. Whenever emitting a dynamic `style` string, include all inline declarations and `--*` custom properties together in that string.
   - For StyleX rules, `createSpring`, `<vector>` (`svg` / `createCanvasScene`), and wheel/scroll handling, read [`references/styling-and-motion.md`](references/styling-and-motion.md).
4. **Build, lint, and typecheck**:
   - Target `@hewel/zgui/tsconfig.native.json` (or `"jsxImportSource": "@hewel/zgui"`).
   - Build via `@hewel/zgui-build` (`buildApplication`, `watchApplication`, `devApplication`, `previewApplication`) or the workspace CLI (`bun run build --app <name>`).
5. **Verify visually in headless preview**:
   - Hand off to the `zgui-preview` skill to verify rendered layout, alpha transparency, multi-surface lifecycle, and clean resource teardown (`teardown.clean === true`).

## Completion criteria

- `oxlint` (with `@hewel/zgui/oxlint/recommended`), typecheck, and bundle validation pass cleanly.
- Every resource opened inside a Solid scope is released on owner cleanup.
- Headless preview (`zgui-preview`) passes assertions, pixel checks, and reports zero leaked teardown resources.
