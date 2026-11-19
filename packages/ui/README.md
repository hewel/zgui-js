# @hewel/zgui

Native TSX components and reactive primitives for the zgui-js host. Use the package's
`tsconfig.native.json` and `@hewel/zgui` JSX import source; these are native nodes,
not browser elements. `renderApp` owns the mounted subtree and returns its
idempotent disposer.

The UI runtime is authored in strict TypeScript. This private workspace package
exports its checked source directly, so application types and component
implementations share one contract without a separate declaration build.
Babel/Rolldown erase types and bundle ES2022 JavaScript for QuickJS; the native host
remains Rust.

The package-local `#solid` import uses a native-only type contract for the pinned
Solid runtime and resolves to `solid-js` when bundled. This intentionally avoids
Solid's DOM JSX declarations. Native bridge types remain internal; application
typechecking does not acquire browser or Node globals.

## Wayland shell surfaces

Declare the `shellSurfaces` capability in the application manifest. A pure shell
exports `mount()` without a primary root and returns the disposer from `renderShell`.
An application with a primary window can also use `LayerSurface` inside `renderApp`.
Each named shell surface has its own mount region, but its children inherit the
surrounding Solid context and can share signals with other surfaces.
Names must be unique within the session, start with a lowercase ASCII letter or
digit, use at most 128 lowercase ASCII letters, digits, dots, hyphens or
underscores, and must not be `main`.

```tsx
import { For, LayerSurface, renderShell, useMonitors } from "@hewel/zgui";

export const hostApiVersion = 1;
export function mount() {
  return renderShell(() => {
    const monitors = useMonitors();
    return (
      <For each={monitors()} keyed={monitor => monitor.id}>
        {monitor => (
          <LayerSurface name={`bar-${monitor().id.toLowerCase()}`} output={monitor().id}
            anchor={["top", "left", "right"]} height={32} exclusiveZone={32}>
            <text>{monitor().name}</text>
          </LayerSurface>
        )}
      </For>
    );
  });
}
```

`useMonitors()` returns an accessor to a read-only output snapshot that updates
on hotplug and geometry changes. Use `id` to target an output and key a list;
`name` is for display. Output geometry is in physical pixels, scale is the ratio
of physical to logical pixels, and refresh is in millihertz or `null`.

Omit `output` to let the compositor choose. The surface attaches asynchronously;
a change of name or output closes that generation and mounts a fresh one.
Disconnecting a named output detaches its mount region; a still-owned request
mounts fresh children if the output returns.
Other placement changes reconfigure the existing surface. The defaults are
the `top` layer, no anchors, zero margins and exclusive zone, and `none` keyboard
interactivity. An omitted width or height stretches (zero) if both opposing
edges are anchored; otherwise it defaults to one logical pixel.

`inputRegion` defaults to `"full"`. Use `"empty"` for pointer passthrough,
pass surface-local logical rectangles, or pass native nodes from the surface
to track their laid-out border boxes. An empty array accepts no input.
A node with no laid-out box contributes no input. Removing a bound node clears
its input rectangle during the same bridge turn, even if your app still holds
the ref. A detached handle cannot be used to register a new binding in a later turn.

## Wayland session lock

Declare `zgui.capabilities.sessionLock: true` in the application package.
The grant is separate from `shellSurfaces`, denied by default, and also
requires a backend that supports session locking. Unauthorized calls throw an
error with `code: "E_CAPABILITY"`.

Call `requestSessionLock()` from mounted code. Its `state()` accessor reports
the compositor decision: only `"locked"` confirms that ordinary content is
hidden across all outputs; `"refused"` means acquisition was denied. Use
`<SessionLockSurface name="lock-dp-1" output="DP-1">…</SessionLockSurface>`
to supply content for an output. Build one declaration per desired output
with `useMonitors()` and a keyed `For`. Each declaration gets a distinct
mount region; missing content and hotplugged outputs are covered by an
opaque native fallback.

Call `lock.unlock()` only after application-controlled authentication or an
equivalent authorized action. The SDK does not authenticate users. `dispose()`
invalidates the handle and its subscription, not the lock. A Solid owner
also disposes the subscription on cleanup. Closing a surface, replacing or
retiring a generation, or losing the JS runtime never unlocks the compositor.
A rejected replacement leaves the accepted lock UI mounted. If the previous
generation's teardown faults during a locked reload, the compositor keeps the
native fallback and only a subsequent clean, authorized generation can unlock.
An unhandled Promise rejection while the lock is retained, or a fault in any
surface cleanup triggered by unlock, cancels the unlock. Surface disposers
cannot register timers or animation-frame callbacks.
If the process disconnects while locked, recovery is compositor policy.

Preview advertises `session-lock`. `monitors.set` changes output inventory;
set `services.sessionLock.outcome` to `"finished"` in the fixture to refuse
the next request (default: `"locked"`). Preview records covered and fallback
outputs and explicit unlock counts in readiness and `services.json`.

## Node queries

`getWindowRect(node)` reads the last completed native frame and returns a copied
window-relative, axis-aligned rectangle in CSS pixels — the same coordinate
space `pointer.position` reports. It returns `null` when the node has no placed
box, accounts for ancestor origins, scroll and transforms through the native
host, and never forces layout. `precedes(first, second)` answers the current
native tree order and is `false` for a node against itself or for disconnected
nodes. Fabricated, foreign-session, foreign-generation, or stale handles are
rejected by the bridge for both queries.

`getTextContent(node)` reads the current native subtree text in native tree
order and returns it as an owned string — no trimming, no visibility or
accessibility filtering. Detached but live nodes read fine, an empty subtree
yields `""`, and stale or foreign handles are rejected as for the other
queries. On `NativeKeyEvent`, `character` carries the exact
`Key::Character` text — including multi-codepoint and supplementary-plane
strings — and is `null` for named, dead, unidentified, and other keys,
including named Space. The `ariaKeyShortcuts` element prop sets
accessibility metadata only; it does not register a keyboard shortcut.

`observeGeometry(node, kind, callback)` streams copied geometry snapshots in
CSS pixels. Node quantities are `borderBox` (window-relative), `borderSize`,
`contentSize`, and `scrollPosition` (an `offset` point plus `contentSize` and
`scrollport` sizes). `viewport` reports the owning window's rectangle and its
`scale` in device pixels per CSS pixel. It also reports density changes when
the CSS rectangle stays equal. The source node owns this subscription; the
window root is not exposed. Callbacks run after layout and before paint, never
during registration; the first real sample arrives even when the box is
zero-sized, and later calls report value changes. A node quantity with no
layout box waits for one. The returned cancellation is idempotent; registration inside a
Solid owner (including an effect rerun or a removed branch) cleans itself up,
permanent native removal and session disposal retire the subscription, and
callers outside any owner cancel manually. The pool is bounded at four times
the native node limit. Mutating a snapshot does not affect native geometry or
later snapshots.

```tsx
<box ref={node => observeGeometry(node, "borderSize", size => setWidth(size.width))} />
```

`isAnimating(node)` reads the native runtime's latest animation snapshot —
refreshed after styling and before lifecycle callbacks — and answers whether
the node's own box or generated-content pseudos still have running animations
or transitions; descendants do not count, and the answer is `false` when
nothing is active. Invalid node handles are rejected as for the other
queries.

All eight lifecycle phases are exposed as handlers — `onAnimationStart`,
`onAnimationIteration`, `onAnimationEnd`, `onAnimationCancel`,
`onTransitionRun`, `onTransitionStart`, `onTransitionEnd`,
`onTransitionCancel` — each with a `Capture` variant. The event is an owned
snapshot: `animationName`/`propertyName`, `elapsedTime` in **seconds** (the
event `timestamp` stays monotonic milliseconds), and `pseudoElement` — an empty
string for the node's own box, `::before`/`::after` for generated content.

`trapFocus(node, options?)` returns an idempotent release function. `wrap`,
`autoFocus`, and `restore` default to `true`. The native runtime enters the
subtree after layout; nested traps select the innermost scope. Releases follow
the current Solid owner when one exists, and node removal/session teardown
release outstanding traps. Use the returned function for earlier release or
when outside a Solid owner.

`lockScroll(node)` returns an idempotent release function and freezes the
native window's root scroll container — scroll position, layout, and gutter
are preserved, inner scrollers keep scrolling, and no DOM overflow style is
mutated. The node identifies the lease's lifetime, not a scroll target:
leases are shared across JavaScript owners mounted in the same window and
bounded by the native node limit. Releases follow the current Solid owner
when one exists, and node removal/session teardown release outstanding
leases. Use the returned function for earlier release or when outside a
Solid owner.

`Portal` renders children on the native `content`, `popover`, `modal`, or
`toast` band (`popover` by default). The band is fixed for that mount. Solid
ownership and context stay at the written position. Native layout, style
inheritance, and event propagation follow the overlay tree; component theme
wrappers must carry scoped themes across this move. The marker and content
are reclaimed together, and shared window layer roots remain private to the
host.

`registerDismissLayer(node, callback, options?)` registers a dismiss layer on
the native `content`, `popover`, `modal`, or `toast` band (`popover` by
default) and returns a `NativeDismissLayer` with `update` and `dispose`. The
`layer` chooses dismissal priority only; registration never mounts or moves
the node, and Portal handles placement. `dismissOnOutsidePress` and
`dismissOnEscape` default to `true`, `exclude` to `null`, and `leaving` to
`false`. `update` replaces the whole option snapshot and keeps the
registration's activation order. One shared window arbiter delivers each
event to at most the topmost registration, chosen by band and then
activation order; a declining layer shields lower layers from that event.
Escape skips `leaving` layers entirely, while a `leaving` layer still
shields lower layers from outside presses. A press inside the layer or
inside `exclude` is not outside. Outside presses are not consumed; Escape is
consumed only when a dismissal is delivered. The callback receives one
reason string, `"outside-press"` or `"escape"`, and decides whether to
close. Registrations inside a Solid owner release with that owner; a lease
created outside an owner lasts until explicit `dispose`, node removal, or
session teardown. `dispose` is idempotent and `update` after release throws
`ReferenceError`.

## Native images

`<Image src="assets/icon.png" />` and intrinsic `<image src="assets/icon.png" />`
use the production native image loader, including intrinsic sizing, decode demand,
device-scale variants and GPU attachment. Import `Image` and `ImageProps` from
`@hewel/zgui`. A reactive `src` change retires the previous request; null or undefined
clears it. Images do not accept children.

Live and preview packages resolve image sources as exact manifest-declared asset
paths backed by integrity-verified immutable bytes, independent of the host's
working directory. Native CSS supports gradients but not URL-backed backgrounds;
package policies reject CSS URL, import and image-set references rather than
silently showing blank pixels. Live images use ordinary native asynchronous
publication. Strict preview publication waits for a complete native worker batch
and never advances the application clock.
Live package reload retires old image requests, caches and replay leases before
mounting the next owner, so identical paths cannot reuse a previous generation's pixels.
Readiness additionally requires successful native texture upload and current atlas
residency for the demanded variant. A native atlas allocation refusal is an asset
failure, not a successfully rendered blank image; graphics-device upload errors
remain distinct graphics failures.
Only paint-visible resources block `required-visible` captures: native surface
bounds, ancestor clips, transforms and current opacity determine that visibility.
Offscreen source changes may remain decoded but unuploaded until brought into view.

## Native vector drawing

`<vector>` is a childless native drawing element. Use `d` for one or more
newline-separated SVG paths painted with the element's `color`, or `svg` for a
standalone SVG document with its own fills and strokes:

```tsx
<vector style="width: 24px; height: 24px" svg={`
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
    <circle cx="12" cy="12" r="8" fill="#f80" stroke="#246" stroke-width="2"/>
  </svg>
`} />
```

Use the vector's `viewBox` for path and canvas coordinates; a standalone SVG
uses its own view box. Drawing precedence is `canvas` > `svg` > `d`. Set a
drawing prop to `null` or `undefined` to remove that source.
Documents must have an `<svg>` root and be well-formed XML without a DTD.
SVG `<image>` and `<feImage>` elements are rejected; use the native image API
for media resources.

For frequent geometry changes, create a retained scene once and update it
without replacing the vector prop or rebuilding SVG text:

```tsx
import { createCanvasScene, renderApp, type NativeNode } from "@hewel/zgui";

export function mount(root: NativeNode) {
  return renderApp(() => {
    const scene = createCanvasScene();
    scene.update([{ kind: "rect", x: 4, y: 4, width: 12, height: 12, fill: "#09f" }]);
    return <vector canvas={scene} viewBox="0 0 24 24"
      style="width: 24px; height: 24px" />;
  }, root);
}
```

- `scene.update(commands, transform?)` replaces the geometry in paint order.
  Pass `null` to keep geometry (for example, when panning), or `[]` to clear
  the scene without revealing a fallback. `CanvasTransform` is a readonly
  affine tuple `[a, b, c, d, e, f]`; omit it to retain the current transform.
- Each `CanvasCommand` needs `fill` and/or `stroke: { color, width }`. Colors
  are hex strings (`#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`). Numbers must be
  finite with magnitude at most `10⁹`; dimensions, radii and stroke widths
  must be positive. Use `rect` (`x`, `y`, `width`, `height`), `roundedRect`
  (also `radius`), `circle` (`x`, `y`, `radius`, centered at `x`, `y`), or
  `arc` (also `startAngle` and nonzero `sweepAngle` in radians, at most `2π`
  in magnitude).
  A `path` takes `segments`: `moveTo` and `lineTo` take `x`, `y`; `quadTo`
  adds `cx`, `cy`; `cubicTo` adds `c1x`, `c1y`, `c2x`, `c2y`; `close`
  takes no coordinates. Begin the path and resume after a `close` with `moveTo`.
  One update accepts up to 4,096 shapes and 16,384 path segments in total.
  A session can hold up to 256 scenes, 8,192 shapes, and 32,768 path
  elements (including those generated for primitives) across all scenes.
  Exceeding a limit throws `RangeError` without changing the scene.
- Pass the opaque `CanvasScene` itself to `canvas`; multiple vectors may
  share it. A scene created inside a Solid owner is disposed with that owner.
  Outside an owner, dispose it explicitly. `dispose()` is idempotent, unsets
  `canvas` on every bound vector, and makes further updates throw. A vector
  with no `svg` or `d` fallback becomes blank.

## Native text controls

```tsx
import { TextField, TextArea, createSignal } from "@hewel/zgui";

function Form() {
  const [name, setName] = createSignal("A中B");
  return <column>
    <TextField ariaLabel="Name" value={name()}
      onInput={event => setName(event.value)} />
    <TextArea ariaLabel="Notes" defaultValue={"one\n二"} />
  </column>;
}
```

`TextField` creates a single-line native `field`; `TextArea` creates a multiline
native `editor`. Both default to `tabIndex={0}` and fix their semantic role to
`textInput` and `multilineTextInput`, respectively. A `field` additionally
accepts `secure`: the empty native `secure` attribute switches the native editor
into password mode, and `TextField` reports the `passwordInput` role while it is
set. Masking, clipboard and IME behavior stay native: native drawing and
accessibility values are masked, while application `value` and edit callbacks
still carry the actual text. Intrinsic `<field />` accepts `NativeFieldProps` (which adds
`secure` to `NativeTextControlProps`); intrinsic `<editor />` accepts
`NativeTextControlProps`. Component props are exported as `TextFieldProps` and
`TextAreaProps`.

The native host supplies `white-space: pre` for `field` and `pre-wrap` for
`editor`, matching zgui's native text controls. These defaults preserve spaces
and multiline layout across reloads; application CSS can override them. Other
visual styling remains application-owned.

- `value` reacts to its source and writes through the native value API, never
  through reconciled text children. Echo `event.value` into a controlled signal
  in `onInput`. The native model owns text, caret, selection, composition and
  editing behavior; the SDK does not implement another keyboard/IME model.
- `value={null}` explicitly sets an empty string. An initially `undefined` value
  is absent under Solid's initial property diff, so `defaultValue` may initialize
  the control. Changing a previously assigned value to `undefined` (or removing
  it from a spread) clears it. A direct renderer `setProp` assignment of
  `undefined` also clears it.
- `defaultValue` is initial-only: its first actual assignment initializes the
  native value, and later changes are ignored. Once any explicit `value`
  assignment occurs, `defaultValue` cannot overwrite it, regardless of prop
  order. Use `value` for programmatic resets.
- `readOnly` updates native read-only state and accessibility semantics. A
  read-only field remains focusable and selectable; `disabled` uses native
  disabled behavior. `disabled`, `ariaLabel` and `ariaDescription` remain reactive.
- `onInput` reports native edits; `onChange` separately reports committed changes.
  Programmatic value assignments do not synthesize input events. The native
  model decides caret preservation for equal-value controlled echoes.
- Event `value` and `selection` are owned snapshots. Selection `start` and `end`
  are **UTF-8 byte offsets**, not JavaScript UTF-16 indices. Selection is read-only;
  no selection setter or selection-geometry query is exposed. Event targets are native
  handles, not browser elements.
- Children are forbidden in the JSX types and at runtime. Components reject a
  `children` property before evaluating its getter. Native editor-owned
  descendants are opaque to the bridge and must not be reconciled or mutated by
  application code. Use `value`/`defaultValue`, not nested text.

Browser-only props such as `type`, `rows` and `selectionStart` are not accepted.
Use native `class` (or its StyleX alias `className`), native style strings/objects,
and the supported native event handlers.

Native style objects accept visual and typography properties such as `boxShadow`,
`textShadow`, `backdropFilter`, and `fontVariationSettings` in camelCase or
kebab-case. `scale` accepts a finite number; `translate` and `rotate` require
CSS strings, even for zero.

## Pointer capture

Pointer action handlers report `onPointerDown`, `onPointerMove`, `onPointerUp` and
`onPointerCancel` (plus their `Capture` variants). During a `pointerdown`
handler, `event.capturePointer()` routes subsequent pointer events to
`event.currentTarget`; during any of these handlers, `event.releasePointer()`
ends the current node's capture for the event's pointer. Both carry the same
synchronous authority lifetime as `preventDefault()`/`activateNow()`: they must
be called inside the handler, the last request in one handler wins, and calling
them after the handler returns throws. A capture is removed when its native
node is reclaimed.

`onPointerEnter` and `onPointerLeave` report entering or leaving a native subtree,
without bubbling when the pointer crosses between its descendants. Their
`Capture` variants observe descendant boundaries. `NativePointerBoundaryEvent`
carries the pointer snapshot without pointer-capture or activation methods.

## Wheel events

`onWheel` and `onWheelCapture` receive `NativeWheelEvent`. `delta` preserves
the device's `"pixels"` (CSS pixels) or `"lines"` unit. Positive deltas move
the scroll offset right and down. `scrollPhase` describes the gesture;
`phase` describes event propagation. Call `preventDefault()` inside the
handler to suppress native scrolling, independently of propagation.

## Spring motion

`createSpring(target, options?)` returns a numeric accessor initialized to
`target()`. Call it in the owner that uses the motion:

```tsx
const scale = createSpring(() => expanded() ? 1 : 0.9);
return <box style={{ scale: scale() }} />;
```

Retargeting preserves velocity. The spring snaps exactly to the target and
stops requesting frames once both displacement and speed are within
`precision` and `velocityPrecision`, or if the integration overflows.
Owner disposal cancels queued frames.
With reduced motion, the accessor follows the target without requesting any frames.

## Styled components

`@hewel/zgui` ships the low-level native elements above. `@hewel/zgui/components`
provides native Solid ports of upstream zgui components on the same renderer:
the upstream stylesheets and theme tokens install as native stylesheets, and
`ThemeProvider` applies a light/dark/system scheme:

```tsx
import { createSignal } from "@hewel/zgui";
import { Button, Input, ThemeProvider } from "@hewel/zgui/components";

function Form() {
  const [name, setName] = createSignal("");
  return <ThemeProvider scheme="system">
    <Input label="Name" value={name()}
      onValueChange={value => setName(value)} />
    <Button>Save</Button>
  </ThemeProvider>;
}
```

Controlled controls report edits through component callbacks such as
`onValueChange(value)` — the string itself, not `event.target.value`; the native
`onInput` event is still available with `event.value`. Caller props override
component defaults. SDK structural classes and inline invariants are privately owned.

`class` is canonical and `className` is its StyleX alias. Empty/whitespace-only
aliases contribute nothing. Supplying tokens through both produces
`CLASS_ALIAS_CONFLICT`, retains the last application class contribution, and
does not block unrelated props. This applies to native elements and components
that explicitly forward to one native root; providers and multi-root composites
do not invent a styling target. `data-style-src` updates and clears normally.

Style objects accept native kebab-case names and explicit camelCase aliases.
Custom-property names retain their case. Finite numbers are accepted only for
custom properties and `opacity`, `flexGrow`, `flexShrink`, `fontWeight`,
`lineHeight`, `zIndex`, `order`, and `animationIterationCount` (including their
kebab-case forms); all other numeric values require explicit CSS strings.
Native grammar validates strings, including rejecting `NaNpx`.

Each application style map is a whole-map transaction: omitted/null keys clear
application ownership and reveal SDK defaults again. Serialized style strings
remain whole application-map replacements, including their custom properties.
Both representations preserve SDK invariants and reject the entire candidate
before any native declaration changes. Preview throws on invalid updates;
live sessions retain the previous map and report structured diagnostics.

`ThemeProvider` exclusively emits SDK theme tokens. The host's reduced-motion
preference forces every shared `--zui-motion-duration-*` token to `0ms`, including
when a theme explicitly overrides a duration.

Component contracts worth knowing:

- `Avatar` keeps its `AvatarFallback` mounted underneath the native image. The
  native image loader marks a source that fails to decode with the custom state
  `zgui-image-broken`; `Avatar` adapts the pinned upstream `:broken` rule to the
  `:state(zgui-image-broken)` selector, so the failed image is hidden and the
  fallback shows without application state. A non-paint-visible decode failure
  does not block `required-visible` captures, while source-authority,
  graphics and evidence failures still fail the capture.
- `Item` gains the upstream `zui-item--described` layout class while at least
  one `ItemDescription` is mounted inside it; nested `Item`s track their own
  descriptions.
- `InputGroup`'s `disabled`/`invalid` describe the group frame (`data-disabled`
  / `data-invalid`); they do not cascade. Set `disabled`/`invalid` directly on
  `InputGroupInput`, `InputGroupTextarea` or `InputGroupButton` where needed.
- `RadioGroup` and `ToggleGroup` share a roving focus model: exactly one enabled
  item holds the group tab stop (it remembers the active item, initially
  preferring the selected radio then the first enabled item in live native
  tree order, including keyed moves), arrows move
  focus on the `orientation` axis (`horizontal` for `ToggleGroup`, `vertical`
  for `RadioGroup`, or `both`), and `Home`/`End` jump to the edges. A `RadioGroupItem`
  selects on focus or click; a `ToggleGroupItem` only toggles on click —
  `selection="single"` gives radio semantics, `"multiple"` keeps a sorted
  unique set. Both take `value`/`defaultValue`/`onValueChange` (a `string` for
  radio, a `readonly string[]` for toggles) and per-item `label` becomes
  `ariaLabel`; item values must be non-empty strings. Direction-key focus calls
  `focus(node, { reveal: true })`, which queues a native `IntoView` reveal
  rather than forcing layout. `RadioGroupItem` also publishes `ariaRadioGroup`,
  a native node-handle relation resolved like `ariaOwns`.
- `Slider` is the upstream horizontal single-thumb control. It takes
  `value`/`defaultValue`/`onValueChange` (a number) with `min`/`max`/`step`
  bounds; steps are `min`-relative, `step={0}` means continuous pointer
  movement with a keyboard step of 1, and `Home`/`End` (and far pointer
  positions) reach the exact endpoints even when `step` does not divide the
  interval. Keys map ArrowRight/ArrowUp `+1`, ArrowLeft/ArrowDown `−1`,
  PageUp/PageDown `±10` steps, clamped before applying. Pointer drags are
  bound to the capturing pointer id — other pointers cannot move or end the
  gesture — positions map through the track's window rect (CSS pixels).
  Matching release/cancel ends capture; disabling stops value updates.
  Displayed values and numeric semantics are normalized to current bounds;
  prop changes do not emit onValueChange. Invalid non-finite values, reversed
  ranges and negative or unrepresentable steps fail with
  `TypeError`/`RangeError`.

- `Tabs`, `TabsList`, `TabsTrigger` and `TabsContent` model upstream tabs.
  `Tabs` takes `value`/`defaultValue`/`onValueChange` (a string, empty by
  default); `defaultValue` seeds only the owned state and later prop updates
  never reset it. `activation="automatic"` selects on focus while `"manual"`
  selects on click/Enter/Space intent; `orientation` accepts horizontal,
  vertical or both axes for roving arrow-key navigation. `TabsList` takes a
  `default` or `line` `variant`. Triggers report `selected` semantics and a
  `data-state` of `active`/`inactive`, never checkbox `checked` semantics.
  Each trigger publishes `ariaControls` to its panel and each panel
  `ariaLabelledBy` back to its trigger as native node-handle relations that
  follow the current `value` — reordering, removal or renaming re-links
  automatically. `TabsContent` keeps a stable panel element, builds children
  lazily when selected, and `keepMounted` preserves an inactive branch;
  hidden panels report `ariaHidden`. Orientation and list variant stay local
  to each nested Tabs group, including parts inside layout wrappers. The
  both-axis mode uses a horizontal visual layout.
- `Collapsible`, `CollapsibleTrigger` and `CollapsibleContent` model an
  upstream disclosure. `Collapsible` takes `open`/`defaultOpen`/`onOpenChange`
  — `defaultOpen` seeds only the owned state. Content children stay mounted
  while closed: the clip measures the inner border size and reports it as the
  `--zui-collapsible-height` style variable (merged with caller `style`), and
  contents become hidden only once the clipping viewport height reaches zero.
  Closing while focus is inside restores focus to the enabled trigger, or to
  the disclosure root when no enabled trigger exists.
- `Accordion`, `AccordionItem`, `AccordionTrigger` and `AccordionContent` model
  upstream accordion on the same disclosure. `Accordion` takes
  `value`/`defaultValue`/`onValueChange` as sorted string arrays,
  `selection="single"` (default) or `"multiple"`, and `collapsible` (default
  true, gates emptying a single selection). `AccordionItem` takes a non-empty
  string `value`; arrow keys and Home/End rove focus between triggers without
  expanding items, while clicks or Enter/Space toggle them.
  `AccordionTrigger.level` sets its heading level (default 3), an integer from
  1 to 4294967295. Native elements expose the separate `ariaLevel` property;
  `null` or `undefined` clears it. Content is a `region` labelled by its
  trigger.
- `Dialog` is the compound modal: `Dialog`, `DialogTrigger`,
  `DialogContent`, `DialogHeader`, `DialogTitle`, `DialogDescription`,
  `DialogFooter`, `DialogClose` and `DialogDismiss`. `Dialog` takes
  `open`/`defaultOpen`/`onOpenChange` — `defaultOpen` seeds only the owned
  state and `open` makes the parent authoritative. `DialogContent` mounts
  lazily on first open and its caller attributes (`testId`, `class`,
  `style`, `ref`, handlers) land on the actual dialog surface, which reports
  `role="dialog"`, `ariaModal` and `data-state` open/closed. The surface
  points `ariaLabelledBy`/`ariaDescribedBy` at `DialogTitle`/
  `DialogDescription` and `ariaPopupFor` back at the trigger, while the
  trigger publishes `ariaHasPopup="dialog"`, `expanded` and `ariaControls`
  to the surface. A caller `onClick` on a trigger or close control vetoes
  the transition with `event.preventDefault()`; `disabled` stops it too.
  `DialogContent` takes `dismissOnOutsidePress`/`dismissOnEscape` (default
  true) and `dismissControl` (default true, a built-in `DialogDismiss` X);
  `DialogFooter` takes `closeControl` (default false, a `DialogClose`
  "Close"). During exit, the surface and its children stay mounted.
  Reopening during exit retains them; opening after unmount creates fresh content.
  While open the surface holds a focus trap and a root scroll lock; the
  lock follows `open` while the trap lives until actual unmount. Escape,
  Tab and outside presses are native; the surface registers a dismiss layer
  on the `modal` band or a higher enclosing band, and nested
  dialogs raise elevation depth and re-scope the theme inside the portal.
  Without an enclosing overlay root, triggers and close controls run only
  their caller handlers. Content stays unmounted.

`Popover` shares Dialog's controlled/default-open state and trigger/close behavior.
Compose `PopoverTrigger`, `PopoverContent`, `PopoverHeader`, `PopoverTitle`,
`PopoverDescription`, and `PopoverClose`. Content defaults to
`placement={{ side: "bottom", align: "center" }}` and `offset={4}`; it flips and
shifts within the window, follows the trigger through scrolling and resizing,
and keeps its exit animation mounted. Popovers confine keyboard focus and
restore it on exit, without a scrim or background scroll lock. Content forwards
native element props; positioning attributes live on the surrounding wrapper.
Focus restoration returns to the element focused before opening. A default-open
surface can have no previous focus; Tab still enters the remaining parent scope.

`Tooltip` and `HoverCard` are lightweight hover surfaces with the same
controlled/default-open state, portal elevation and lazy exit animation.
Compose `TooltipProvider`, `Tooltip`, `TooltipTrigger`, `TooltipContent` and
`TooltipArrow`, or `HoverCard`, `HoverCardTrigger` and `HoverCardContent`.
`delay` and `closeDelay` are milliseconds; a `Tooltip` defaults to `0`/`150`,
a `HoverCard` to `700`/`300`, and `TooltipProvider` overrides the defaults for
every member group it wraps. While one member is open, the next opens
immediately and hands a `data-instant` exit to the replaced surface. Hovering
the trigger or the mounted surface keeps it open. A `Tooltip` opens at once
only for keyboard focus, while a `HoverCard` opens for any focus cause and
closes on blur; neither surface traps focus, draws a scrim or locks scroll,
and outside presses do not dismiss them. `TooltipContent` renders a decorative
`TooltipArrow` by default (`arrow={false}` removes it); the arrow pins the
panel offset to at least the arrow reach plus a 4px tip gap
(~10.071 CSS pixels), so a smaller custom `offset` is clamped only while the
arrow is visible. `ariaDescribedBy` on a tooltip
trigger and `ariaControls` on a card trigger point at the mounted surface
only, so give icon-only controls their own accessible name for the
not-yet-open state.

`DropdownMenu` composes `DropdownMenuTrigger`, `DropdownMenuContent`,
`MenuItem`, `MenuCheckboxItem`, `MenuRadioGroup`, `MenuRadioItem`,
`MenuLabel`, `MenuGroup`, `MenuSeparator` and `MenuShortcut`; `MenuContent`
and `MenuTypeahead` are the shared surface parts other menu roots reuse.
The surface traps and restores focus, and clicking the trigger, pressing
`ArrowUp`/`ArrowDown`, or a caller-unvetoed activation opens it; a
`preventDefault` in a caller handler vetoes the open. Items form one
roving tab stop: arrows wrap and skip disabled rows in native tree order,
`Home`/`End` jump, hover focuses a row without selecting it, and a primary
pointer down activates the pressed row. Character keys drive typeahead
over live subtree text — prefix matching lowercases text, leading
whitespace is ignored, same-character repeats cycle, the buffer resets
after 1000ms, and only a match consumes the key. A `MenuItem` calls
`onSelect` before closing the root unless `closeOnSelect={false}`;
checkbox and radio items also close by default. Checkbox items toggle
`checked` and radios pick within their group on activation, and
controlled `open`/`checked`/`value` emit requests only. `MenuItem`'s
`shortcut` renders a `MenuShortcut` and sets `ariaKeyShortcuts` as
accessibility metadata without registering a hotkey.

`MenuSub` provides `MenuSubTrigger` and `MenuSubContent`. The trigger
opens on click or named `ArrowRight`, `Enter`, or `Space`, and after a
120 ms hover. Leaving the trigger arms a 180 ms close unless the pointer heads
through the submenu corridor; arrival at the trigger or panel cancels
that close. The panel starts at `right`/`start` with a -4 px offset and
can flip at the viewport edge. `ArrowLeft` closes the current branch and
focuses its trigger; `Escape` dismisses the top open branch. Selecting
a leaf item requests the root menu to close unless that item's
`closeOnSelect` is false. Submenu state supports the same controlled
and uncontrolled open props. `headingToward(from, to, target)` tests
the inclusive-edge triangle from `from` to the near vertical edge of
`target`; supply all three inputs in the same coordinate space.

Elements also support `onContextMenu` (`onContextMenuCapture`), receiving a
cancelable `NativeContextMenuEvent` pointer snapshot without
`releasePointer`, `capturePointer`, or `activateNow`. In the native runtime,
holding a touch contact still (~500 ms) synthesizes `contextmenu` using that
touch contact's pointer ID and CSS coordinates while canceling the active press
so releasing the finger does not emit `click`.

`ContextMenu` composes `ContextMenuTrigger`, `ContextMenuContent`, shared menu
items (`MenuItem`, `MenuCheckboxItem`, `MenuRadioGroup`, `MenuRadioItem`,
`MenuLabel`, `MenuGroup`, `MenuSeparator`, `MenuShortcut`, `MenuSub`), and
`useContextMenuPlace`. `ContextMenuTrigger` renders `.zui-context-menu__area`
with a zero-sized `.zui-context-menu__anchor` child excluded from outside-press
dismissal (so primary clicks elsewhere inside the area close the menu), and
opens on secondary `onPointerDown` or `onContextMenu` unless disabled or
canceled via `preventDefault()`. `useContextMenuPlace()?.openAt(position)`
positions the anchor at window-relative CSS coordinates (subtracting
`getWindowRect(area)` when mounted) and opens the menu; `ContextMenuContent`
defaults to `bottom`/`start` with `0` px offset.

`Menubar` composes `MenubarMenu`, `MenubarTrigger`, `MenubarContent`,
`MenubarArrows`, `MenubarItem`, `MenubarCheckboxItem`, `MenubarRadioGroup`,
`MenubarRadioItem`, `MenubarLabel`, `MenubarGroup`, `MenubarSeparator`, and
`MenubarShortcut`. `Menubar` owns one open `value` at a time and manages
horizontal roving `tabIndex` across triggers; closed triggers do not open on
hover or focus (opening on `Click` or `ArrowDown`), while once any menu is
open, focusing or hovering another enabled trigger or pressing
`ArrowLeft`/`ArrowRight` via `MenubarArrows` switches menus immediately
(unmounting the prior `MenubarContent` cleanly without `.zui-surface` exit fade
or focus trapping). `MenubarContent` focuses the first enabled item on open,
enables `MenuTypeahead`, and separates dismiss focus behavior: `Escape` or item
dismissal (`closeOnSelect` defaults to `false` on `MenubarCheckboxItem` and
`true` on `MenubarItem` / `MenubarRadioItem`) closes the menu and restores
focus to its trigger, whereas outside presses close without stealing focus.

```tsx
<Dialog open={open()} onOpenChange={setOpen}>
  <DialogTrigger>Rename</DialogTrigger>
  <DialogContent>
    <DialogHeader>
      <DialogTitle>Rename project</DialogTitle>
      <DialogDescription>Choose a new name.</DialogDescription>
    </DialogHeader>
    <Input label="Name" />
    <DialogFooter closeControl>
      <DialogClose variant="outline">Cancel</DialogClose>
    </DialogFooter>
  </DialogContent>
</Dialog>
```

The native semantic attributes behind the dialog relations are also
available on any element: `ariaHasPopup` accepts `"menu"`, `"listbox"`,
`"tree"`, `"grid"` or `"dialog"`, and `ariaPopupFor` accepts a single native
node handle — a foreign or stale node throws `TypeError`, an array is
rejected even empty, and `null` or `undefined` clears either attribute.

A composition example exercises the Avatar, Empty, Item and InputGroup
families: build it with `bun run build --app composition` and preview it with
`bun run preview --app composition --scenario preview/scenarios/composition-virtual.json`.
A selection example exercises RadioGroup, ToggleGroup and Slider with
reordering, pointer identity and remount: build it with
`bun run build --app selection` and preview it with
`bun run preview --app selection --scenario preview/scenarios/selection-virtual.json`.
A tabs example exercises automatic and manual activation, keepMounted and
trigger reordering: build it with `bun run build --app tabs` and preview it with
`bun run preview --app tabs --scenario preview/scenarios/tabs-virtual.json`.

## Editing example

Build with `bun run build --app editing`, or develop with
`bun run dev --app editing`. The example uses real native controls and ordinary
TSX event handlers, without test hooks or a simulated bridge.

| Class | Initial state / action |
| --- | --- |
| `editing-field` | Controlled `TextField`, `A中B` |
| `editing-area` | Controlled `TextArea`, `one\n二` |
| `editing-default` | Uncontrolled `TextField`, initial `seed` |
| `editing-readonly` | Read-only `TextField`, `Read only` |
| `editing-disabled` | Disabled `TextField`, `Disabled` |
| `editing-reset` | Button writes `Reset 中` to the controlled field |
| `editing-change-default` | Button changes the default source to `replacement`; current uncontrolled text stays unchanged |

The visible event report aggregates all five controls. Inputs in the two editable
controlled fields echo into their corresponding signals; every input increments
`editing-input-count`. Committed changes increment `editing-change-count`
separately. Both event kinds update `editing-last-value` and
`editing-last-selection` (`start:end`, UTF-8 bytes). `editing-target-valid` starts
as `true` and remains true only if every received input/change event has
`event.target === event.currentTarget`.

Report values are plain native text under those stable classes. Initial counts
are `0`, last value is empty, selection is `0:0`, and target validity is `true`.
Read-only and disabled controls have handlers too, so an unexpected edit is
observable rather than hidden. Native focus borders and disabled styles make
keyboard focus and non-editable states visible.
