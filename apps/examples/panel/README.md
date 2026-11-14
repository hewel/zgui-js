# niri panel

A read-only native TSX view of the real compositor. It imports `@zgui/ui` and
`@zgui/services`; it does not use browser APIs, synthetic data, input handlers,
polling, shell commands or a service replacement. The host must inherit a valid
`NIRI_SOCKET` from the niri session.

Build with `pnpm ui:build --app panel`, or watch with
`pnpm ui:dev --app panel`. The builder packages `src/app.css` separately;
it is not a runtime CSS import. The ES2022-only tsconfig extends the same native
SDK configuration as the other examples.

## Native shell profile

```sh
ZGUI_PLATFORM=wayland cargo run -p native-host -- \
  --bundle apps/examples/panel/dist \
  --layer-config apps/examples/panel/layer.json
```

`layer.json` explicitly targets `DP-1`; select an existing output name from
`niri msg --json outputs` before launching on another desktop. An unavailable
initial output is an error, not implicit placement on a different monitor.
The profile requests the top layer, top/left/right anchors, stretched width,
360 CSS-pixel height and exclusive zone, zero margins, and no keyboard focus.
The compositor determines physical dimensions at its current fractional scale.
The transparent native surface permits alpha rendering; this example's CSS
deliberately paints an opaque background.

The host configures the native primary surface before opening it. A close event
with the selected monitor absent suspends the session and releases subscriptions.
While suspended, bounded 500 ms probes wait for that same output; its return
creates a fresh native document and JS generation. This does not reconnect a
failed IPC subscription inside an otherwise live session.

The ignored `shell::proof::real_panel_layer` gate verifies real service delivery
and zero-resource shutdown. Setting `ZGUI_SHELL_LIFECYCLE=1` gives an external
controller 40 seconds to exercise output removal/return and requires a second
live generation; it then allows three seconds for capture before shutdown.
Do not change physical output configuration without explicit permission.

### Popups and menus

This profile does **not** support out-of-surface popups. The pinned native
Wayland popup path requires an `xdg_surface` parent and rejects a layer parent.
There is no public SDK popup constructor, and the panel does not substitute a
normal window to hide that limitation. Any future in-surface menu must remain
inside the panel's native bounds; clipping and focus differ from a shell popup.
An out-of-surface menu requires a separate upstream layer-parent popup change
and actual compositor acceptance before it can be exposed.

## Visible states

- **Waiting:** no snapshot yet; the panel explains that both initial workspace
  and window state are needed. No empty desktop is fabricated during startup.
- **Live:** separate workspace and window lists show the current snapshot.
  Focused rows have a contrasting background. Focus, activation, floating and
  urgency are also named in text, not conveyed by color alone. Optional values
  have explicit labels, including unnamed workspaces and untitled windows.
- **Disconnected:** startup failures or a terminal service error are visible.
  IPC errors display their code and message. If a live snapshot already existed,
  its rows remain visible and are explicitly identified as the last snapshot.
  Check the compositor/socket and remount to reconnect; there is no hidden retry.
- Empty live snapshots distinguish "No workspaces reported" and "No open
  windows" from waiting and failure.

The compact two-column layout inherits the native examples' dark surface and
system sans. Lists scroll independently in the available surface height; long
window titles can wrap. This panel is observational and never takes keyboard
input or requests compositor changes.

## Identity and cleanup

Each delivery replaces readonly snapshot data. Memos index workspace/window
objects by their lossless decimal string IDs. `For` receives arrays of those
strings, never arrays of newly allocated DTO objects. Each retained row reads
its current object reactively from the relevant map, including cross-list
workspace labels, so title/focus changes update existing rows and reorderings
move them instead of rebuilding them. Missing IDs remove their rows normally.

`onMount` starts exactly one real `watchNiri` subscription. `onCleanup` cancels
and releases it. The disposer returned by `renderApp` owns that cleanup on
unmount/reload. There are no recurring timers, frame callbacks or module-level
subscriptions; idle updates arrive through the service's native wakeup path.

## Integration verification

These are recommended checks, not recorded results:

```sh
pnpm exec tsc --project packages/services/tsconfig.json
pnpm exec tsc --project apps/examples/panel/tsconfig.json
pnpm ui:build --app panel
```

The build checks the native bundle boundary, the single Solid runtime and the
absence of browser/Node runtime dependencies. Native integration should also
cover these observable service and list boundaries:

1. Import the services module without a host binding; import must succeed.
   With the real binding installed, require both callbacks, reject preflight
   and disposal registration, and enforce the four-subscription limit.
2. Deliver actual IPC snapshots with IDs above `Number.MAX_SAFE_INTEGER`; IDs
   and nullable references must remain exact strings. Type fixtures should
   reject snapshot/DTO writes, numeric IDs and omitted error callbacks.
3. Retain native handles for rows with unchanged IDs across multiple snapshots;
   change titles, focus and workspace names and confirm both retained handles
   and updated text. Verify reordering/removal and an empty live snapshot.
4. Cancel twice and then process pending service work; neither callback may
   run after cancellation. Unmount/reload must release the subscription and
   persistent callbacks, including terminal-error and startup-failure paths.
5. After the desktop is unlocked and native launch is authorized, inspect real
   waiting/live/disconnected rendering, scrolling, long titles and the normal
   shell surface dimensions. Compare workspace/window data to the real desktop
   and observe naturally occurring updates while the panel itself stays idle.
   Verify layer placement, monitor/scale changes and cleanup through the host's
   integration launch. No keyboard imitation or compositor capture is needed
   while the desktop is locked.
