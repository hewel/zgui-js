---
name: zgui-dev
description: Develop native UI apps and Wayland desktop shells with @hewel/zgui, @hewel/zgui-services, and @hewel/zgui-build. Use when building TSX screens, StyleX styles, shell surfaces (LayerSurface, PopupSurface, SessionLockSurface), host services (Unix sockets, commands, control messages, image handles), vector/spring animations, or configuring zgui.capabilities manifests.
---

Guide for authoring native QuickJS-NG + Solid 2.0.1 applications and Wayland desktop shells targeting `zgui-js`.

## Runtime model

- **Host, not browser**: bundles run in QuickJS-NG (ES2022) driving Rust native nodes via `@hewel/zgui`. Browser globals (`window`, `document`, `HTMLElement`), DOM APIs, raw `setTimeout`/`setInterval`, and `requestAnimationFrame` are absent or rejected at build validation.
- **Solid 2.0.1 universal renderer**: import reactive primitives (`createSignal`, `createMemo`, `createEffect`, `onSettled`, `onCleanup`, `Show`, `For`, `Switch`, `Match`, `createStore`, `createProjection`) from `@hewel/zgui`, not `solid-js/web`.
- **Owner-scoped native lifetimes**: construct conditional native nodes inside the Solid owner that controls their visibility (`<Show>`, `<For>`, `<Switch>`). Removing a node invalidates its handle after the bridge turn completes; re-showing creates a fresh node, so keep stateful signals/memos above the branch or memoize independent subtrees separately.
- **Zero raw host access**: import public primitives from `@hewel/zgui`, `@hewel/zgui/components`, `@hewel/zgui/icons`, and `@hewel/zgui-services`. Leave `__zgui`, `__zguiShell`, and `__zguiServices` encapsulated inside the SDK.

## Development workflow

1. **Declare the application package** (`package.json`):
   - Set `"zgui": { "id": "<app-id>", "entry": "src/main.tsx", "capabilities": { ... } }` (plus optional `"styles"`, `"assets"`, `"fonts"`).
   - Declare only the capabilities the app genuinely uses; undeclared host operations fail closed with `E_CAPABILITY`.
   - For capability schemas, path templates, host services, and programmatic builds from external repos, read [`references/services-and-capabilities.md`](references/services-and-capabilities.md).
2. **Choose the entry point**:
   - Standard single-window app: `renderApp(() => <App />)` from `@hewel/zgui`.
   - Pure Wayland desktop shell (no default `main` window): `renderShell(() => <Shell />)` from `@hewel/zgui`, requiring `capabilities.shellSurfaces: true` (or `sessionLock: true`) and launching with `--pure-shell`.
   - For multi-output bars, popups, input regions, and lockscreens, read [`references/shell-surfaces.md`](references/shell-surfaces.md).
3. **Author UI, styles, and motion**:
   - Use semantic native tags (`<box>`, `<text>`, `<vector>`) and themed primitives from `@hewel/zgui/components` (`Button`, `TextField`, `TextArea`, `Image`, dialogs, menus, popovers, tabs, etc.).
   - Prefer StyleX (`@stylexjs/stylex`) or typed `NativeStyle` objects. Whenever emitting a dynamic `style` string, emit all inline declarations and `--*` custom properties together in the same string because a native `style` string replaces every inline declaration on that node.
   - For StyleX rules, `createSpring`, `<vector>` (`svg` / `createCanvasScene`), and wheel/scroll handling, read [`references/styling-and-motion.md`](references/styling-and-motion.md).
4. **Build and typecheck**:
   - Use `@hewel/zgui-build` (`buildApplication`, `watchApplication`, `devApplication`, `previewApplication`) or the workspace CLI (`bun run build --app <name>`).
   - Ensure TypeScript targets the native JSX runtime (`@hewel/zgui/tsconfig.native.json` or `"jsxImportSource": "@hewel/zgui"`).
5. **Verify visually in headless preview**:
   - Hand off to the `zgui-preview` skill to verify rendered layout, alpha transparency, multi-surface lifecycle, and clean resource teardown (`teardown.clean === true`).

## Completion criteria

Work on a `zgui-js` UI or shell feature is done when:
- Typecheck and bundle validation pass with zero lint suppressions and zero undeclared capability errors.
- Every resource opened inside a Solid scope (`connectUnixSocket`, `spawnCommand`, `onControlMessage`, `loadImageAsset`, `loadImageBytes`, `requestSessionLock`, `observeGeometry`) is released on owner cleanup.
- Headless preview (`zgui-preview`) passes assertions, pixel checks, and reports zero leaked teardown resources.
