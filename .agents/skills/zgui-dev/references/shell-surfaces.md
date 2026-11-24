# Shell surfaces & Wayland lifecycle

Use when authoring multi-output desktop shell UIs (status bars, docks, launchers, popups, OSD overlays, or lockscreens) with `@hewel/zgui`.

## Pure-shell entry

- Mount a headless root with `renderShell(() => <Shell />)` when the app owns compositor surfaces without a default `main` window.
- Require `capabilities.shellSurfaces: true` (for `<LayerSurface>` / `<PopupSurface>`) and/or `capabilities.sessionLock: true` (for `requestSessionLock()` / `<SessionLockSurface>`) in `package.json` under `zgui.capabilities`.
- Launch live with `zgui-host --pure-shell` (or set `"pureShell": true` in a preview profile).
- Track hotplug outputs reactively with `useMonitors(): Accessor<readonly Monitor[]>` inside a Solid owner.
- Read the monotonic session generation (safe even during module preflight) via `getSessionGeneration()`.

## Shared shell stylesheets

Use `mountShellStylesheet(name, css)` from `@hewel/zgui` for theme CSS shared by all mount regions in the session, including surfaces that attach later or reattach after hotplug. Install it during mount or a later update, not during module preflight. Reinstalling an active name replaces its CSS in cascade order; its old disposer cannot remove the replacement. Region-local sheets have a separate namespace.

Solid 2.0.1 runs `createEffect(compute, apply)`'s `apply` callback without an owner. The shell API needs no mount region. Keep the latest disposer for owner cleanup; disposing before each replacement would move the sheet to the end of the cascade.

```tsx
let disposeTheme: (() => void) | undefined;
createEffect(themeCss, css => { disposeTheme = mountShellStylesheet("shell-theme", css); });
onCleanup(() => disposeTheme?.());
```

For CSS that belongs to one surface, use [a region-local stylesheet](styling-and-motion.md#region-local-stylesheets).

## `<LayerSurface>`

Owns a named `zwlr_layer_surface_v1` compositor surface without inserting a node in the parent tree:

```tsx
<LayerSurface
  name={`bar-${monitor.id}`}
  output={monitor.id}
  layer="top"                  // "background" | "bottom" | "top" | "overlay"
  anchor={["top", "left", "right"]}
  height={36}                  // width defaults to 0 when both left & right are anchored
  exclusiveZone={36}
  margin={{ top: 6, left: 10, right: 10 }}
  keyboard="none"              // "none" | "exclusive" | "on-demand"
  inputRegion="full"           // "full" | "empty" | (NativeWindowRect | NativeNode)[]
  blurRegion="none"            // "none" | "full" | readonly NativeWindowRect[] | readonly NativeNode[] | BlurRegionNodes
>
  <BarContent />
</LayerSurface>
```

- **Live reconfiguration**: changing `layer`, `anchor`, `width`, `height`, `margin`, `exclusiveZone`, or `keyboard` updates the existing surface in place without destroying its Solid subtree. Changing `name` or `output` closes the old surface and opens a new one.
- **Click-through overlays**: pass `inputRegion="empty"` for non-interactive OSDs, or an array of interactive `NativeNode` refs / `NativeWindowRect`s so only active islands accept pointer hits. Passing `NativeNode` refs automatically tracks layout changes via `observeGeometry`.
- **Background blur (`blurRegion`) & live capability (`getBackgroundBlurSupported()`)**: supported on `<WindowSurface>`, `<LayerSurface>`, and `<PopupSurface>` via Wayland `ext_background_effect_v1` (no extra manifest capability beyond the surface's own authority). Defaults to `"none"` (omitted, `[]`, or `{ nodes: [] }`). Passing `"full"` requests blur across the entire surface extent (surviving resizes); passing `NativeWindowRect[]` blurs explicit surface-local rectangles; passing `NativeNode[]` or `BlurRegionNodes` (`{ nodes: readonly NativeNode[]; minOpacity?: number }`) registers target nodes directly with the native runtime (without JS `observeGeometry` callbacks or 1-frame lag). Native resolution scan-converts each target's border box with its `border-radius`, `--zgui-corner-shape` (including `superellipse(...)`), live transform animations, and full ancestor clip chain into integer logical strips (capped safely within 256 rectangles, conservatively omitting targets with CSS `clip-path`, `visibility: hidden`, or non-axis-aligned transforms). `minOpacity` defaults to `0.5` and thresholds accumulated ancestor/self `opacity` (ignoring `background-color` alpha; `minOpacity: 0` skips only actual `opacity: 0` factors and `visibility: hidden`). Passing mixed `[rect, node]` arrays, non-node values, foreign/stale nodes, or invalid `minOpacity` (non-number / `NaN` / `±Infinity` / outside `[0, 1]`) throws atomically while preserving the prior registration. Upgrade `@hewel/zgui` and `zgui-host` in lockstep (`hostApiVersion` remains `1`, so older hosts reject `{ nodes }` payloads). `blurRegion` never alters `inputRegion`, opaque regions, or surface transparency (`transparent={true}` and translucent CSS backgrounds remain explicit), and unsupported compositors ignore it cleanly without blocking surface creation. Read the read-only reactive accessor `getBackgroundBlurSupported(): boolean` from `@hewel/zgui` (works in both `renderApp` and rootless zero-surface `renderShell`) to switch between translucent glass styling and solid fallback live as the compositor advertises or withdraws `Capability::Blur`. `<SessionLockSurface>` is strictly excluded from `blurRegion` at both the TypeScript and protocol layers.

## `<PopupSurface>`

Owns an `xdg_popup` attached to a named `<LayerSurface>` or parent `<PopupSurface>` in the same session:

```tsx
<PopupSurface
  name="wifi-menu"
  parent={`bar-${monitor.id}`}
  anchorNode={buttonRef()}     // or anchorRect={{ x, y, width, height }}
  width={280}
  height={320}
  geometryInsets={28}          // optional outer drawable bleed (whole nonnegative CSS px scalar or { top?, right?, bottom?, left? })
  placement={{ anchor: "bottom-left", gravity: "bottom-right", flip: true, slide: true }}
  offset={{ x: 0, y: 6 }}
  grab={true}
  onDismiss={() => setOpen(false)}
>
  <WifiMenu />
</PopupSurface>
```

- `anchorNode` must belong to the exact `parent` surface named in `props.parent`.
- **Outer shadow bleed (`geometryInsets`)**: `width`, numeric `height`, `minHeight`, and `maxHeight` always describe the visual panel box `G`. Setting `geometryInsets` reserves `I = (L, T, R, B)` outer drawable bleed around `G` so the native surface allocates `S = (G.w + L + R, G.h + T + B)` while declaring `xdg_surface.set_window_geometry(L, T, G.w, G.h)`. All DOM layout, `observeGeometry`, `getWindowRect`, `inputRegion`, and `blurRegion` remain in surface-local drawable coordinates `S` (wrap the visual card in matching outer padding and pass `inputRegion={[card]}` / `blurRegion={[card]}` if the shadow gutter should pass clicks through or avoid blur). When `height="auto"`, the runtime subtracts `top + bottom` bleed before clamping to `[minHeight, maxHeight]`. Nested child popups anchored against an inset parent keep parent surface-local `anchorNode`/`anchorRect` coordinates; the Wayland backend subtracts the parent's `(L, T)` automatically.
- **Cascade teardown**: closing a parent `<LayerSurface>` or `<PopupSurface>` closes descendant popups topmost-first (child before parent) and invokes `onDismiss`.
- Keep `<PopupSurface>` mounted inside `<Show when={open()}>` so dismissing and reopening always creates a clean owner-scoped surface.
- **Nested popup submenus (`@hewel/zgui/primitives`)**: compose `MenuRootPrimitive` + `MenuListPrimitive` inside the parent `<PopupSurface>` and `MenuSubPrimitive` + `MenuSubTriggerPrimitive` + `<Show when={open()}><PopupSurface parent="..."><MenuSubContentPrimitive>` for child submenus. Cross-document node links (`ariaControls`, `ariaPopupFor`, dismiss `exclude`) are omitted automatically to prevent foreign-handle errors; hover-open avoids demanding compositor keyboard focus and uses a 180 ms leave grace timer canceled on child/trigger `pointerenter` instead of fabricated cross-surface `headingToward` coordinates; `Escape` delivered to the parent closes only the deepest open child branch first.

## Session locking (`ext-session-lock-v1`)

Locking is fail-closed and security-critical:

1. Call `const lock = requestSessionLock()` inside a Solid owner (requires `capabilities.sessionLock: true`).
2. `lock.id` is a non-configurable monotonic number; `lock.state()` transitions `"unlocked" -> "locking" -> "locked" | "finished"`.
3. Render one `<SessionLockSurface name={`lock-${m.id}`} output={m.id}>` per output in `useMonitors()` while locked.
4. `SessionLockSurface` always paints an opaque black compositor backdrop beneath author content—author `:root { background-color: transparent }` never exposes desktop windows beneath the lock.
5. Call `lock.unlock()` only from an explicit authorized user action (for example after PAM/auth verification succeeds). Session retirement or JS exceptions never release the platform lock; Rust maintains opaque black fallback coverage on every output until explicit unlock.
