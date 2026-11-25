# Changelog

All notable changes to `zgui-js` are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and all published
packages version in lockstep using [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0] - 2026-11-25

Initial public npm release of `zgui-js` — native Linux/Wayland desktop apps and Wayland shells
written in TypeScript, Solid 2.0, and Lightning CSS Modules, rendered natively by Rust (`zgui`,
Stylo, Taffy, Parley, `wgpu`/Vello) and QuickJS-NG with no browser DOM or WebView.

### Published Packages

All 6 public packages publish in lockstep at `0.1.0`:

- `@hewel/zgui` — compiled ES2022 `.js` + `.d.ts` zero-DOM Solid 2.0 UI SDK, renderer, themed
  components (`@hewel/zgui/components`), headless primitives (`@hewel/zgui/primitives`), typed
  memory router (`@hewel/zgui/router`), vector icons (`@hewel/zgui/icons`), `tsconfig.native.json`,
  and `oxlint` preset (`@hewel/zgui/oxlint/recommended`).
- `@hewel/zgui-services` — compiled `.js` + `.d.ts` capability-gated host services SDK.
- `@hewel/zgui-build` — programmatic compiler (`@solidjs/compiler` Oxc, Lightning CSS Modules
  `lightningcss-v1`, Rolldown), `tsrs` typechecker, host resolver, and app packager.
- `@hewel/zgui-host-linux-x64` — prebuilt Linux `x86_64` GNU `glibc >= 2.39` (`ubuntu-24.04`)
  `zgui-host` binary installed automatically via `@hewel/zgui-cli` `optionalDependencies`.
- `@hewel/zgui-cli` — `zgui-js` CLI (`init`, `dev`, `build`, `preview`, `package`, `doctor`,
  `inspect`, `smoke`) with prebuilt read-only `inspector-bundle/`.
- `create-zgui-js` — standalone project scaffolder (`bun create zgui-js`).

### Highlights

- **Zero-DOM Solid 2.0 SDK & Styling:** Compiled `@hewel/zgui` and `@hewel/zgui-services` packages
  with `#solid` zero-DOM isolation compatible with strict `tsrs` and official `tsc`
  (`skipLibCheck: false`, `exactOptionalPropertyTypes: true`), static `components.css` hoisting,
  native `--zgui-scrollbar-mode: overlay` scrollbars, native `inert` subtree isolation, scalar
  `ariaActiveDescendant`, and IME-safe `NativeKeyEvent.isComposing`.
- **Headless Primitives & Motion:** Unstyled behavioral primitives decoupled from built-in styles
  (`createPresence`, `createRetainedPresence`, `createOverlayState`, `createDismissLayer`,
  `createRovingFocus`, `createVirtualFocus`, `RadioGroupPrimitive`/`RadioItemPrimitive`,
  `SliderPrimitive`, and `Menu*Primitive` including cross-`PopupSurface` submenus and DPI-aware
  `headingToward` hover corridors), plus `createSpring` with shared per-bridge rAF ticking.
- **Wayland Shell Surfaces:** Declarative `LayerSurface`, `PopupSurface` (with `geometryInsets`
  via `xdg_surface.set_window_geometry`), `WindowSurface`, `SessionLockSurface`, reactive
  `useMonitors()` and `getBackgroundBlurSupported()`, and native frame-synchronous `blurRegion`
  resolution via `ext_background_effect_v1`.
- **Capability-Gated Host Services:** Manifest-declared HTTP/SSE/WebSocket, SQLite/KV (with
  `createKvState`), reactive `createHostQuery`, local files/bookmarks & XDG portals, clipboard,
  Unix sockets, commands, control channel, image handles, and isolated compute workers.
- **Fast Dev Loop, Packaging & Headless Preview:** Non-blocking async `tsrs`, CSS-only hot-swap,
  atomic staged reload with `createDevState`, self-hosted visual Inspector (`zgui-js inspect --ui`),
  self-contained bytecode app packaging (`zgui-js package --bytecode --desktop-entry` with
  redistributed `licenses/`), and deterministic headless Vulkan (`wgpu`/Lavapipe) offscreen preview
  verification.
- **Release Infrastructure:** Automated `ubuntu-24.04` GitHub Actions release pipeline with parallel
  Rust/vendor verification, `ldd`/glibc audit, isolated clean-runner installation verification,
  idempotent fail-closed SRI publishing, and npm Trusted Publishing (OIDC `--provenance`).

[Unreleased]: https://github.com/hewel/zgui-js/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/hewel/zgui-js/releases/tag/v0.1.0
