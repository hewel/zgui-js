# Styling, components, vectors & spring motion

Use when styling native nodes, composing UI primitives, rendering custom vectors/canvas scenes, or adding physics/wheel interactions in `@hewel/zgui`.

## Styling (Lightning CSS Modules & `NativeStyle`)

- **CSS Modules (`*.module.css`) & `app.css`**: compiled at build time by Lightning CSS (`lightningcss-v1`) with CSS nesting (`&`) flattened, literal `--zui-*` theme variables preserved (`dashedIdents: false`), and deterministic cascade order (`@layer zgui.components` `components.css` → global `app.css` → `*.module.css`). Apply static classes on `class` with `cx(styles.card, active() && styles.active)` and drive states via `[data-*]` attributes.
- **Disallowed v1 CSS features**: `@import`, `url()`, `image-set()`, `@font-face`, `@container`, `prefers-reduced-motion`, `@property`, `html[dir]`, `composes`, ICSS `:import`/`:export`, and `@value`.
- **Typed inline styles**: pass a `NativeStyle` object to `style={...}` for continuous dynamic values (split sizes, spring transforms, progress widths).
- **Inline `style` string atomicity**: passing a raw `style="..."` string replaces all inline declarations on that node, including `--*` CSS custom properties. Emit dynamic properties and `--*` variables together in one string, or use a `NativeStyle` object.
- **Scoped part styles**: scope component part selectors to context-derived data attributes so ancestor selectors do not bleed across nested component roots.
- **Native overlay scrollbars**: set inherited `--zgui-scrollbar-mode: overlay` on `:root` or any scroll container (override back with `--zgui-scrollbar-mode: classic`; default is `classic`). Overlay scrollbars reserve zero layout gutter, paint above scrolled children only on overflowing axes as an auto-fading `4px` resting / `6px` hovered-or-dragged capsule (`800ms` hold + `200ms` fade, instantaneous under `reduced_motion`, passing clicks through when hidden), and reuse `--zgui-scrollbar-thumb`, `--zgui-scrollbar-track`, and `color-scheme`.

### Region-local stylesheets

Use `mountStylesheet(name, css, target?)` from `@hewel/zgui` for dynamic CSS in one mount region. Omit `target` inside a mounted region's Solid owner, or pass a live `NativeNode` to select that node's region. The returned disposer is registered automatically when called under an owner.

Solid 2.0.1 runs `createEffect(compute, apply)`'s `apply` callback without an owner. In a pure shell, pass a node ref and return the disposer from `apply` so Solid removes the previous sheet on rerun and cleans up when the effect is disposed:

```tsx
createEffect(
  () => ({ node: panelRef(), css: panelCss() }),
  ({ node, css }) => node ? mountStylesheet("panel-theme", css, node) : undefined,
);
```

Without a ref, capture `const regionOwner = getOwner()` inside the mounted region before creating the effect, then return `runWithOwner(regionOwner, () => mountStylesheet("panel-theme", css))` from `apply`. The headless shell root is not a mount region. For a theme shared by multiple surfaces, use [a shell stylesheet](shell-surfaces.md#shared-shell-stylesheets) instead.

## Themed components (`@hewel/zgui/components`)

- Prefer `@hewel/zgui/components` primitives over hand-rolling focus traps, dismiss layers, poppers, or roving focus.
- Core low-level primitives from `@hewel/zgui`:
  - `<Button>`, `<TextField>`, `<TextArea>`, `<Image>`, `<Portal>`
  - `focus`, `trapFocus`, `registerDismissLayer`, `lockScroll`, `observeGeometry`, `getWindowRect`, `getTextContent`, `precedes`, `isAnimating`
- When modifying exports in `@hewel/zgui/components`, gallery `testId`s, or wrapped `generated/*Style.ts` sheets inside `zgui-js`, update `docs/development/component-map.md` to satisfy `tests/build/component-map.test.mjs`.

## Vector graphics (`<vector>` & `createCanvasScene`)

- **Static / reactive SVG**: `<vector svg={svgMarkup} width={24} height={24} />` renders vector paths natively without raster aliasing.
- **Imperative 2D canvas scenes**: create a bounded session-scoped scene with `const scene = createCanvasScene()` from `@hewel/zgui`, build paths/fills/strokes, and pass it to `<vector canvas={scene} />`. Dispose/update within an owner and stay within session geometry budgets.

## Spring motion, retained presence & wheel input

- **Physics motion**: use `createSpring(targetAccessor, options)` from `@hewel/zgui` (`stiffness`, `damping`, `mass`, `precision`, `velocityPrecision`, plus mount-only `initial`) instead of raw `requestAnimationFrame` loops. Springs coalesce onto a private shared per-bridge rAF ticker (with rootless-shell parking across zero-surface intervals), honor `reducedMotion`, and expose an independent boolean `spring.settled` signal that never re-runs subscribers per frame.
- **Imperative spring controls**: `spring.jump(value)` snaps position, clears velocity, and re-animates toward the current target; `spring.impulse(velocityDelta)` adds velocity (in target units/second) without moving position while syncing same-turn target updates; `spring.configure(physics)` updates `SpringPhysicsOptions` live (rejecting `initial`). Reduced-motion and disposed springs treat valid jumps/impulses as no-ops; non-finite values throw `RangeError` in every mode.
- **Retained exit presence**: use `createRetainedPresence(sourceAccessor, { settled? })` from `@hewel/zgui/primitives` to keep the last non-nullish payload (`0`, `""`, `false`, and function values are present) mounted until both CSS exit animations (1,000 ms CSS deadline) and an optional reactive `settled` accessor (such as `spring.settled`, never overridden by the CSS deadline) complete, then unmount and clear `value()` atomically.
- **Wheel / scroll events**: attach `onWheel` or `onWheelCapture` on `<box>` elements to handle smooth touchpad or discrete mouse wheel deltas (for example volume/brightness sliders or workspace switching on a Wayland bar). For custom-styled sliders, compose `@hewel/zgui/primitives`'s `SliderPrimitive` (attaching `trackRef` to the stationary travel track, setting `pointerStep` independently of keyboard `step`, and layering wheel/spring polish in the skin).
- **Budgets**: keep reactive trees and event handlers within QuickJS stack and per-entry wall budgets (`--max-js-stack-bytes`, `maxJsEntryWallMs`), and avoid allocating deeply nested eager portals during initial synchronous mount.
