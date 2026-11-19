# Styling, components, vectors & spring motion

Use when styling native nodes, composing UI primitives, rendering custom vectors/canvas scenes, or adding physics/wheel interactions in `@hewel/zgui`.

## Styling (`StyleX` & `NativeStyle`)

- **StyleX** (`@stylexjs/stylex`): compiled at build time against the `zgui-stylex-v1` native profile. Apply via `{...stylex.props(styles.card, active() && styles.active)}`.
- **Typed inline styles**: pass a `NativeStyle` object to `style={...}` for dynamic values.
- **Inline `style` string atomicity**: passing a raw `style="..."` string replaces all inline declarations on that node, including `--*` CSS custom properties. Emit dynamic properties and `--*` variables together in one string, or use a `NativeStyle` object.
- **Scoped part styles**: scope component part selectors to context-derived data attributes so ancestor selectors do not bleed across nested component roots.
- **Runtime stylesheets**: use `mountStylesheet(css)` from `@hewel/zgui` inside an owner when mounting dynamic theme or token sheets (auto-removed on disposal).

## Themed components (`@hewel/zgui/components`)

- Prefer `@hewel/zgui/components` primitives over hand-rolling focus traps, dismiss layers, poppers, or roving focus.
- Core low-level primitives from `@hewel/zgui`:
  - `<Button>`, `<TextField>`, `<TextArea>`, `<Image>`, `<Portal>`
  - `focus`, `trapFocus`, `registerDismissLayer`, `lockScroll`, `observeGeometry`, `getWindowRect`, `getTextContent`, `precedes`, `isAnimating`
- When modifying exports in `@hewel/zgui/components`, gallery `testId`s, or wrapped `generated/*Style.ts` sheets inside `zgui-js`, update `docs/development/component-map.md` to satisfy `tests/build/component-map.test.mjs`.

## Vector graphics (`<vector>` & `createCanvasScene`)

- **Static / reactive SVG**: `<vector svg={svgMarkup} width={24} height={24} />` renders vector paths natively without raster aliasing.
- **Imperative 2D canvas scenes**: create a bounded session-scoped scene with `const scene = createCanvasScene()` from `@hewel/zgui`, build paths/fills/strokes, and pass it to `<vector canvas={scene} />`. Dispose/update within an owner and stay within session geometry budgets.

## Spring motion & wheel input

- **Physics motion**: use `createSpring(targetAccessor, options)` from `@hewel/zgui` (`stiffness`, `damping`, `mass`, `precision`, `initial`) instead of raw `requestAnimationFrame` loops. It integrates with the native frame clock and honors `reducedMotion`.
- **Wheel / scroll events**: attach `onWheel` or `onWheelCapture` on `<box>` elements to handle smooth touchpad or discrete mouse wheel deltas (for example volume/brightness sliders or workspace switching on a Wayland bar).
- **Budgets**: keep reactive trees and event handlers within QuickJS stack and per-entry wall budgets (`--max-js-stack-bytes`, `maxJsEntryWallMs`), and avoid allocating deeply nested eager portals during initial synchronous mount.
