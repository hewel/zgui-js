# Changelog

All notable changes to `zgui-js` are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and all published
packages version in lockstep using [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.4] - 2026-11-28

### Added

- Add optional `namespace` passthrough on `<LayerSurface>` (`LayerSurfacePlacement` /
  `LayerSurfaceProps`) in `@hewel/zgui` through `zgui-host` (`openLayer`, `updateLayer`, and
  `--layer-config`) to Wayland `zwlr_layer_shell_v1.get_layer_surface` (falling back to
  `application_id`), recreating the shell surface on reactive `namespace` changes and exposing
  `frame.layer` metadata (including effective `namespace`) in headless preview `*.frame.json`.
- Vendor `zgui-render-wgpu` and add Linux Vulkan DMA-BUF zero-copy `FrameStreamHandle` binding on
  `<Image>`/`<image>` (RGBA8, BGRA8, and GPU-converted NV12 with memfd/Fixture software fallback)
  plus opt-in `SCM_RIGHTS` frame submission and GPU-completion release credits over Unix sockets.
- Add capability-gated `systemFiles` service (`lstat`, paginated ICU4X-collated `readDir`,
  streaming file/trash jobs with interactive `onConflict`, volume discovery, Freedesktop thumbnail
  `ImageHandle` lookup, and `openExternal`), Wayland `Copy | Move` and directory/outbound file DnD,
  `Breadcrumb`, and headless `createSelectionModel` / `createMarqueeSelection` primitives.
- Upgrade per-app grant store to schema v2 (`listGrants`, `revokeGrant`, `clearGrants`), add
  `Calendar` and `DatePicker` components, add single-instance Activation forwarding with inbound
  Wayland `XDG_ACTIVATION_TOKEN` redemption (`getInitialActivation`, `onActivation`), bridge
  Jiff/ICU4X timezone/locale/collation APIs and `URL`/`URLSearchParams`/`TextEncoder`/`TextDecoder`
  globals, and add variable-height `VirtualList`, `AlertDialog`, `Table`, and `DataTable`.

### Fixed

- Harden Wave 4 host, service, and UI contracts: preserve explicit `--host-root` working directory
  in `zgui-js dev`, verify primary `appId` greeting before single-instance Activation handoff,
  enforce runtime user-authorization provenance on `persistSystemGrant`, deliver confined
  `ev.systemEntries` on external drops, wire native `ariaSort` and scoped `Table` selectors, and
  reseal `preview/baselines/counter` metadata.

## [0.1.3] - 2026-11-27

### Added

- Export Wayland `xdg_activation_v1` external focus-handoff tokens (`requestActivationToken`,
  `ActivationTokenOptions`) and lock-modifier queries (`getCapsLock`, `useCapsLock`, plus
  queue-time `NativeEvent.modifiers.{capsLock,numLock}` and `SessionLockSurface` focus modifier
  replay) from `@hewel/zgui`, backed by unforgeable physical-input grants and fail-closed session
  revocation.
- Add `zgui-js typecheck` CLI command (enabled by default in `zgui-js init` templates), export
  multi-root `syncCssModuleTypes` and `SyncCssModuleTypesOptions` from `@hewel/zgui-build`, add
  `BuildApplicationOptions.cssModuleInclude`, and watch external `tsconfig.json` CSS Module roots
  during `zgui-js dev`.
- Integrate `eslint-plugin-solid` (`0.20.1`) into `@hewel/zgui/oxlint`, add the
  `@hewel/zgui/oxlint/strict` preset (`zgui/prefer-stylesheet`), enable
  `zgui/no-static-component-stylesheet` and `zgui/no-getter-object-spread` in
  `@hewel/zgui/oxlint/recommended`, and ship precompiled TypeScript Oxlint plugins under
  `@hewel/zgui` (`dist/oxlint/*.js`).

### Fixed

- Pure shells apply CSS-only live reloads only in dev mode. A failed live stylesheet swap falls
  back to staged replacement, preserving the last healthy generation if the candidate is rejected.
- Prioritize explicit `--host-root` ahead of installed `@hewel/zgui-host-*` packages (failing
  closed when missing or invalid) and report conflicting module paths when duplicate Solid runtimes
  are detected.
- Surface plain `Error` `name`/`message` properties and QuickJS `[[ErrorData]]` stack traces in
  `console.*` without invoking user-defined getters.
- Rasterize SVG images at exact device-pixel demand so scaled vector assets stay crisp on HiDPI
  outputs.
- Resolve Solid 2.0 dev-mode reactive scope warnings, Cargo checkout ABI probing and single-pass
  bytecode packaging in the host runner, and managed `ShellSurface` theme and reduced-motion signal
  propagation.

## [0.1.2] - 2026-11-26

> Note: `0.1.1` was skipped prior to npm publication (`v0.1.2` includes all changes since `0.1.0`).

### Added

- Align `schemas/package.schema.json` and `zgui-host` capability parsing (`clipboard`, `sqlite`,
  `files`, `network`), add lossless per-origin `NetworkCapabilities.originRules` enforced by
  `host-services` `OriginPolicy`, and add the base-shell `@hewel/zgui/shell-host` runtime extension
  overlay host (`--extensions-overlay` watcher with lock-screen deferral, bounded `<= 64 KiB`
  session handoff, 3-phase commit with observation window, anti-resurrection Safe Base Shell
  rollback, and `--bytecode-cache-dir` `ZSQB` LRU cache).
- Support Wayland `niri-blur-shape-v1` rounded blur masks with continuous opacity, disjoint strip
  fallback, commit-boundary protocol reconciliation, lock-surface isolation, and opt-in preview
  evidence.

### Fixed

- CLI and npm release tests use the workspace and packed versions instead of a fixed release number.
- Preview teardown leaves time for native shutdown and telemetry without removing the per-disposer
  JavaScript timeout. Debug builds default to 200 ms per disposer; release builds default to 20 ms.
- Heavy menu, dialog, and Markdown editor preview tests use the functional profile on Lavapipe.
  Timing-sensitive suites keep the default profile.
- Serialize headless `wgpu`/Lavapipe `Driver` unit tests inside `zgui-host` to prevent Mesa 24
  concurrent-teardown `SIGSEGV` and 4-vCPU startup budget exhaustion.

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

[Unreleased]: https://github.com/hewel/zgui-js/compare/v0.1.4...HEAD
[0.1.4]: https://github.com/hewel/zgui-js/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/hewel/zgui-js/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/hewel/zgui-js/compare/v0.1.0...v0.1.2
[0.1.0]: https://github.com/hewel/zgui-js/releases/tag/v0.1.0
