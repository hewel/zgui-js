# Shell surfaces & Wayland lifecycle

Use when authoring multi-output desktop shell UIs (status bars, docks, launchers, popups, OSD overlays, or lockscreens) with `@hewel/zgui`.

## Pure-shell entry

- Mount a headless root with `renderShell(() => <Shell />)` when the app owns compositor surfaces without a default `main` window.
- Require `capabilities.shellSurfaces: true` (for `<LayerSurface>` / `<PopupSurface>`) and/or `capabilities.sessionLock: true` (for `requestSessionLock()` / `<SessionLockSurface>`) in `package.json` under `zgui.capabilities`.
- Launch live with `native-host --pure-shell` (or set `"pureShell": true` in a preview profile).
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
  blurRegion="none"            // "none" | "full" | readonly NativeWindowRect[] | readonly NativeNode[]
>
  <BarContent />
</LayerSurface>
```

- **Live reconfiguration**: changing `layer`, `anchor`, `width`, `height`, `margin`, `exclusiveZone`, or `keyboard` updates the existing surface in place without destroying its Solid subtree. Changing `name` or `output` closes the old surface and opens a new one.
- **Click-through overlays**: pass `inputRegion="empty"` for non-interactive OSDs, or an array of interactive `NativeNode` refs / `NativeWindowRect`s so only active islands accept pointer hits. Passing `NativeNode` refs automatically tracks layout changes via `observeGeometry`.
- **Background blur (`blurRegion`)**: supported on `<WindowSurface>`, `<LayerSurface>`, and `<PopupSurface>` via Wayland `ext_background_effect_v1` (no extra manifest capability beyond the surface's own authority). Defaults to `"none"` (omitted or `[]`). Passing `"full"` requests blur across the entire surface extent (surviving resizes); passing `NativeWindowRect[]` or `NativeNode[]` blurs the union of positive surface-local boxes (nodes must belong to the owning surface and are tracked reactively via `observeGeometry`). `blurRegion` never alters `inputRegion`, opaque regions, or surface transparency (`transparent={true}` and translucent CSS backgrounds remain explicit), and unsupported compositors ignore it cleanly without blocking surface creation. `<SessionLockSurface>` is strictly excluded from `blurRegion` at both the TypeScript and protocol layers.

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

## Session locking (`ext-session-lock-v1`)

Locking is fail-closed and security-critical:

1. Call `const lock = requestSessionLock()` inside a Solid owner (requires `capabilities.sessionLock: true`).
2. `lock.id` is a non-configurable monotonic number; `lock.state()` transitions `"unlocked" -> "locking" -> "locked" | "finished"`.
3. Render one `<SessionLockSurface name={`lock-${m.id}`} output={m.id}>` per output in `useMonitors()` while locked.
4. `SessionLockSurface` always paints an opaque black compositor backdrop beneath author content—author `:root { background-color: transparent }` never exposes desktop windows beneath the lock.
5. Call `lock.unlock()` only from an explicit authorized user action (for example after PAM/auth verification succeeds). Session retirement or JS exceptions never release the platform lock; Rust maintains opaque black fallback coverage on every output until explicit unlock.
