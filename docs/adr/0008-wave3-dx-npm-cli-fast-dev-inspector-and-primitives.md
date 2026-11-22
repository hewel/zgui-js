# Wave 3 Developer Experience: Pure-npm CLI & Host Distribution, Sub-Second Dev Loop, Self-Hosted Visual Inspector, Bytecode Packaging, and Selective Headless Primitives

**Status:** accepted

Wave 1 (ADR-0004) closed the local-first desktop workbench loop, Wave 2 (ADR-0005) established authority- and generation-isolated networking, DnD, rich document selection, canvas extensions, code-splitting, and background workers, and ADR-0006 + ADR-0007 eliminated StyleX and Babel so `zgui-js`'s entire frontend/host stack is 100% native-compiled (`Bun` + `Rolldown/Oxc` + `@solidjs/compiler` + `Lightning CSS` + `tsrs` + `Oxlint` + `Oxfmt` + `Cargo/native-host`).

However, the end-to-end developer experience still carries four structural friction points compared to mainstream desktop/web frameworks:

1. External JS/TS users cannot scaffold, run `dev`, or package an app without a full `zgui-js` Cargo checkout (`prepareNativeHost` rejects installed SDKs with no Rust workspace), while CLI responsibilities are bundled inside `@hewel/zgui-build`.
2. `watchApplication` blocks every dev rebuild on synchronous `tsrs`, restarts QuickJS even for CSS-only edits, and leaves the native window silently frozen on build errors even though `.zgui-build-status.json` is already written.
3. `#73`'s `--dev-inspect` socket is CLI-only with no visual element picker, box-model highlight, source-location jump, or self-hosted Inspector UI, and external developers/agents lack a concise `llms.txt` API map.
4. Large apps lack an opt-in precompiled QuickJS bytecode packaging path, and Wayland shell consumers (such as `../z-shell`'s `PopupSurface` menus) cannot reuse `packages/ui`'s battle-tested menu/roving/presence state machines because `menu-core.ts` and `overlay.ts` hardcode `Portal`, `Popper`, and generated component stylesheets.

Because `zgui-js` is the JS/TS framework layered over the upstream Rust `zgui` toolkit, we intentionally keep CLI and host distribution **purely within the npm/Bun ecosystem** (`zgui-js` binary, `@hewel/zgui-cli`, platform `optionalDependencies`) rather than squatting on the `zgui` command name or shipping a separate global Rust CLI wrapper.

We will execute Wave 3 across four ordered pillars (`DX-1` through `DX-4`).

## Pillar 1 (`DX-1`): Pure-npm CLI (`@hewel/zgui-cli`) and Zero-Rust Host Distribution

1. **Package boundary split:**
   - Extract the CLI (`bin: { "zgui-js": "./src/cli.ts" }`) out of `@hewel/zgui-build` into a dedicated workspace package **`packages/cli` (`@hewel/zgui-cli`)**.
   - Keep **`@hewel/zgui-build`** purely programmatic (build/watch/preview/typecheck/css/packaging APIs with no CLI entrypoint side effects). Root repo scripts (`bun run dev|build|preview|smoke`) delegate cleanly to `@hewel/zgui-cli`.
2. **Prebuilt `native-host` distribution via npm `optionalDependencies`:**
   - Follow the `esbuild`/`rolldown`/`lightningcss`/`oxlint` npm distribution pattern: define platform host packages (`@hewel/zgui-host-linux-x64`, `@hewel/zgui-host-linux-arm64`) as `optionalDependencies` of `@hewel/zgui-cli` (with packaging/staging tooling and release-manifest verification in-repo).
   - Resolve the `native-host` executable in deterministic order:
     1. Explicit `--host-executable` flag or `ZGUI_JS_HOST_EXECUTABLE` environment variable.
     2. Installed platform package (`@hewel/zgui-host-<os>-<arch>`), verified against `@hewel/zgui-build`'s expected `hostApiVersion` / protocol handshake.
     3. Local Cargo workspace build (`--host-root` or detected `zgui-js` checkout containing `crates/native-host`) via `cargo build -p native-host`.
     4. Actionable error explaining both installing the platform host package and passing `--host-root` / `--host-executable`.
   - External JS/TS developers require **zero Rust toolchain** to run `dev`, `build`, `preview`, `inspect`, or `package`; repo and custom-host Rust developers retain seamless incremental Cargo builds.
3. **Complete npm-native CLI workflow + scaffolding:**
   - `zgui-js init [dir]` + thin **`create-zgui-js`** package (`bun create zgui-js`): scaffolds `desktop-app` or `wayland-shell` templates pre-wired with `@hewel/zgui`, CSS Modules (`*.module.css`), `@hewel/zgui/oxlint`, `tsconfig.native.json`, `zgui.capabilities.json`, and `package.json` scripts.
   - `zgui-js doctor`: validates Bun version, resolved `native-host` binary and ABI version, Wayland display / Vulkan / Mesa ICDs (`VK_DRIVER_FILES`), XDG portal reachability, and `tsrs`/`oxlint` availability with concrete remediation hints.
   - `zgui-js inspect [...]`: forwards directly to the resolved `native-host inspect` subcommand so developers never hunt for binary paths.
   - `zgui-js package`: assembles a self-contained runnable Linux application directory (`<out>/bin/native-host`, `<out>/bundle/` sealed package, executable launcher wrapper, and optional `.desktop` file) without invoking Cargo for JS-only projects.

## Pillar 2 (`DX-2`): Sub-Second Dev Loop

1. **Async non-blocking `tsrs` in `dev` mode (`packages/build/src/watch.ts`):**
   - Decouple `tsrs --noEmit` from the critical `compile -> publish` path during `dev`: publish compiled bundles immediately on save while running `tsrs` concurrently via an `AbortController` that cancels superseded runs on newer edits.
   - Emit structured typecheck status into terminal output and `.zgui-build-status.json` (`typecheck: { status, diagnostics }`) so stale errors never linger.
   - Keep `zgui-js build`, `zgui-js preview`, CI, and `zgui-js dev --strict-typecheck` strictly typecheck-gated prior to publication.
2. **CSS-only live reload without QuickJS restart (`crates/native-host` + `packages/build`):**
   - Detect when a new published generation preserves identical `js` entry, `chunks`, `workers`, `capabilities`, and `hostApiVersion` digests and differs **only** in stylesheet assets (`styles.css` / `components.css`).
   - Add a hot-swap stylesheet reload path in `SessionHost` that parses and replaces the Stylo stylesheets on the live `Session` in place, triggering restyle/relayout/repaint without recreating QuickJS, tearing down services, or losing signal state, focus, selection, or scroll positions.
   - Fail closed to normal staged JS replacement on any non-CSS diff, stylesheet parse error, or live-swap rejection.
3. **Native dev status & error banner (`crates/native-host`, `--watch` dev-only):**
   - Watch `.zgui-build-status.json` in desktop `--watch` mode alongside the bundle symlink.
   - When `status === "failed"` (compile/CSS/validation error), background `typecheck` fails, or a candidate reload/runtime error occurs, render a compact, non-blocking, dismissible native overlay banner at the edge of the dev window displaying the remapped `.tsx`/`.css` path, line/column, and error message while keeping the last healthy frame visible.
   - Automatically clear the banner on the next clean publication. Strictly compiled/gated off in non-dev, release, and `SessionLockSurface` sessions.

## Pillar 3 (`DX-3`): Self-Hosted Visual Dev Inspector & Agent/Docs Discoverability

1. **Visual Element Picker & Box-Model Highlight Overlay (`crates/native-host` + `vendor/zgui-runtime` if needed):**
   - Extend `#73`'s dev-only read-only `--dev-inspect` socket and native paint overlay with an interactive element picker (`Ctrl+Shift+I` or `zgui-js inspect pick`).
   - Hovering performs native hit-testing and paints content/padding/margin bounds plus a concise HUD tag (`<tag>`, classes, dimensions, source location); clicking locks selection and emits the selected node ID over the inspect socket.
   - Strictly preserve `#73`'s security invariants: read-only, never invokes app JS getters, redacts secure text inputs, and hard-forbids inspection on `SessionLockSurface`.
2. **Dev-only JSX source-location metadata (`packages/build` + `@hewel/zgui`):**
   - In `development: true` builds only, attach lightweight static source coordinates (`file:line:col`) to created elements (or a compact sidecar map) and strip them completely from release builds (`development: false`).
   - Expose source coordinates through `--dev-inspect` snapshots so the picker and Inspector jump straight to authored `.tsx` locations.
3. **Self-hosted lightweight Inspector auxiliary window (`apps/devtools/inspector` or `@hewel/zgui-cli` bundled dev app):**
   - Dogfood `zgui-js` itself (`<WindowSurface>` from `#58` + `SplitPane`/`TreeView` from `#62`) to provide a zero-browser, zero-WebView auxiliary Inspector window launched via `zgui-js inspect --ui` (or dev hotkey).
   - Connects strictly over `#73`'s `0600` `--dev-inspect` Unix socket to display: live DOM/native node tree synchronized with the in-window picker, computed Stylo styles & Taffy box metrics, `FramePhaseTimings` sparkline/table, and live resource/teardown counts.
4. **Agent & developer discoverability (`llms.txt`):**
   - Generate and test-verify a concise `llms.txt` (and linked recipe/API summary) aligned with `docs/development/component-map.md`, `@hewel/zgui`, `@hewel/zgui-services`, and `@hewel/zgui-cli`, and ensure `zgui-js init` enables `@hewel/zgui/oxlint` out of the box.

## Pillar 4 (`DX-4`): Optional QuickJS Bytecode Packaging & Selective `@hewel/zgui/primitives`

1. **Optional QuickJS bytecode packaging (`--bytecode` on `build` / `package`):**
   - Add a `native-host compile-bytecode` subcommand using the exact pinned `rquickjs` (`Module::write_object`) linked into `native-host` so bytecode is guaranteed ABI-compatible with zero external `qjs` version drift.
   - When `--bytecode` is enabled on `zgui-js build` or `zgui-js package`, compile entry, lazy chunks, and worker modules into deterministic `.qjsb` payloads, seal `bytecodeFormat`, `quickjsAbi`, and per-artifact SHA-256 digests in `manifest.json`, and allow omitting raw JS source in release bundles while keeping sidecar sourcemap remapping functional.
   - `crates/native-host` and `crates/js-runtime` verify `quickjsAbi` and SHA-256 digests fail-closed before calling `Module::read_object`, preserving single-VM memory/stack budgets and existing JS-source packaging as the default.
2. **Selective `@hewel/zgui/primitives` (Rule of Two, driven by `../z-shell`):**
   - Add a `./primitives` subpath export to `@hewel/zgui` governed by a strict **Rule of Two**: a primitive is extracted only when at least two distinct styled consumers need the exact behavior core (`@hewel/zgui/components` and `../z-shell` Wayland surfaces).
   - Extract Phase 1 primitives directly from existing internal `packages/ui/src/components/{overlay,roving,delay,menu-core}.ts` without duplicating state machines:
     - **Overlay & Presence**: `createPresence`, `createOverlayState`, `createDismissLayer` (decoupled from `Portal`, `Popper`, and `OverlayStyle` so they work identically in `<WindowSurface>`, `<LayerSurface>`, and `<PopupSurface>`).
     - **Roving Focus & Typeahead**: `createRovingFocus`, `createDelayed`.
     - **Headless Menu**: `MenuRootPrimitive`, `MenuListPrimitive`, `MenuItemPrimitive`, `MenuCheckboxItemPrimitive`, `MenuRadioGroupPrimitive`, `MenuRadioItemPrimitive`, `MenuSubPrimitive` with explicit `data-state` / `data-highlighted` / `data-disabled` styling hooks and zero stylesheet or `Portal`/`Popper` coupling.
   - Reject React-style `asChild` cloning and runtime `unstyled?: boolean` flags on styled components.
   - Refactor `@hewel/zgui/components` (`Menu`, `DropdownMenu`, `ContextMenu`, `Menubar`) to sit cleanly atop `@hewel/zgui/primitives` with full visual/behavioral parity, and add a `PopupSurface` headless-menu preview test covering the `../z-shell` `BarPopups` pattern.

## Considered Options

- **Ship a standalone global Rust `zgui` CLI binary (similar to `nsh`):** Rejected because `zgui` is the upstream Rust UI toolkit; `zgui-js` belongs in the npm/Bun ecosystem (`@hewel/zgui-cli` + `zgui-js` bin + `optionalDependencies` host packages), which fits JS/TS workflows without colliding with `zgui`.
- **Keep CLI inside `@hewel/zgui-build`:** Rejected in favor of splitting `@hewel/zgui-cli` from `@hewel/zgui-build` so programmatic build consumers do not pull CLI/scaffolding/optional host binary dependencies.
- **Serve a browser-based HTTP/WebSocket DevTools page for Inspector:** Rejected because launching a browser or bundling a WebView contradicts `zgui-js`'s native-first lightweight footprint and introduces extra network surface; self-hosting the Inspector window in `zgui-js` over `#73`'s `0600` Unix socket dogfoods our own stack.
- **Extract every component in `@hewel/zgui/components` into headless primitives upfront:** Rejected as premature abstraction; Rule of Two starting with Overlay/Presence + Menu solves the proven `../z-shell` `PopupSurface` coupling without doubling API surface across simple styled controls.
- **Make QuickJS bytecode the mandatory default package format:** Rejected because plain JS remains faster to debug, ABI-independent across host rebuilds, and already compact; opt-in `--bytecode` captures cold-start and release-packaging gains cleanly.

## Consequences

- `package.json` workspaces add `packages/cli` (`@hewel/zgui-cli`), `packages/create-zgui-js` (`create-zgui-js`), and platform host package manifests (`packages/host-linux-x64`, `packages/host-linux-arm64`), while `packages/build` (`@hewel/zgui-build`) drops its `bin` field and becomes strictly programmatic.
- `crates/native-host` extends `--watch` with CSS-only live reload and a dev-only error banner, extends `--dev-inspect` with visual element picking and box-model highlights, and adds `compile-bytecode` + ABI-verified bytecode loading.
- `@hewel/zgui` gains a `./primitives` export and `docs/development/component-map.md` + `llms.txt` stay enforced by build tests.
