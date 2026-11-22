> **Historical — Superseded by [`ADR-0006`](../adr/0006-lightning-css-modules-replace-stylex.md).** StyleX and `@stylexjs/*` have been removed from `zgui-js`. Use plain `src/app.css` and Lightning CSS Modules (`*.module.css`) with `cx(...)` instead (see `apps/examples/css-modules` and `llms.txt`).

# StyleX authoring and native packaging

zgui-js compiles [`@stylexjs/stylex` 0.19.2](https://www.npmjs.com/package/@stylexjs/stylex/v/0.19.2) ahead of execution. The QuickJS host receives the transformed application and StyleX merge runtime; native zgui receives the complete extracted stylesheet. There is no browser DOM, runtime stylesheet injector, or CSS string post-processing.

## Setup

Applications import StyleX directly and declare the same exact version as the build integration:

```json
{
  "dependencies": {
    "@hewel/zgui": "workspace:*",
    "@stylexjs/stylex": "0.19.2"
  },
  "devDependencies": {
    "@hewel/zgui-build": "workspace:*"
  }
}
```

Use the ordinary native TypeScript configuration and entry-point contract. `zgui-js build`, `dev`, and `preview` run StyleX before the Solid Universal/TypeScript pass and compose the resulting source maps. Every reachable source module participates, including `.stylex.ts` token modules and dependency sources; tree shaking does not determine stylesheet membership.

## Static, conditional, and dynamic styles

Spread `stylex.props()` onto native elements or SDK components documented as forwarding to one native root:

```tsx
import * as stylex from "@stylexjs/stylex";

const styles = stylex.create({
  panel: { padding: "16px", gap: "8px" },
  compact: { padding: "8px" },
  fill: { height: "8px", backgroundColor: "#315efb" },
  fillWidth: (width: number) => ({ width }),
});

function Progress(props: { compact: boolean; width: number }) {
  return (
    <column {...stylex.props(styles.panel, props.compact && styles.compact)}>
      <box {...stylex.props(styles.fill, styles.fillWidth(props.width))} />
    </column>
  );
}
```

Use the official merge API rather than joining generated class names or spreading StyleX definitions into ordinary objects. Class-token order is not cascade order. Dynamic values become local custom-property declarations, not one generated rule per value. The compiler also emits universal `@property` registrations with `inherits:false`; native zgui consumes those exact registrations so dynamic variables do not leak through descendant inheritance. Typed variables and every other `@property` form remain unsupported.

`class` remains the canonical native prop and `className` is the StyleX alias. Supplying non-empty tokens through both in one effective update is a `CLASS_ALIAS_CONFLICT`; Preview fails and live execution retains the previous application class contribution. Empty and whitespace-only values contribute no tokens. SDK structural classes have separate ownership and are not aliases.

Application style objects accept strings, finite numbers for custom properties, and finite numbers for explicitly unitless ordinary properties. Other numeric properties require authored or compiler-emitted units. Invalid values—including strings such as `NaNpx`—reject the whole incoming application declaration map before native mutation. Omitted keys remove their previous contribution; documented SDK defaults restore without erasing internal invariants.

Serialized native `style="…"` remains supported as an application-owned whole-map replacement. `stylex.attrs()` is not a supported StyleX composition interface in this profile.

## Tokens and themes

Define plain variables in a `.stylex.ts` module and apply themes through the official APIs:

```tsx
import * as stylex from "@stylexjs/stylex";

export const colors = stylex.defineVars({
  surface: "#ffffff",
  text: "#182230",
});

const dark = stylex.createTheme(colors, {
  surface: "#182230",
  text: "#f5f7fa",
});
```

Native `:root`, StyleX priority selectors, doubled theme classes, default inheritance, nested overrides, reset semantics, and theme removal are acceptance-tested. Do not add a browser `html` root or rewrite generated selectors.

`ThemeProvider` remains the only writer of SDK `--zui-*` tokens. Map application-selected values into its `light`/`dark` token props; StyleX themes own application variables and never target SDK variables directly. Omitted `createTheme` overrides follow StyleX reset semantics rather than merging arbitrary values from a parent theme.

`stylex.keyframes` is supported within the native animation profile. A desktop launch with `--reduced-motion`, or a Preview profile with `"reducedMotion": true`, makes the same host preference available to `ThemeProvider`, which drives shared motion-duration tokens to zero. SDK and application animations should reference those tokens. The `prefers-reduced-motion` media query is not emulated.

## Handwritten CSS, assets, and fonts

Optional `src/app.css` is preserved as `base.css` and installed before generated `app.css`. Both sheets use stable ordered native slots and the same artifact identity. At equal specificity, later generated StyleX rules win over the base sheet. SDK component defaults remain in their documented layers.

Use package-declared `<image src="assets/…">` resources for raster images. CSS `url()`, `image-set`, `@import`, and `@font-face` are rejected: the native painter has no CSS URL consumer and native fonts use programmatic registration.

Use `mountStylesheet` for region-local dynamic rules. An explicit target must be a live, attached native node; cleanup after its region retires is safe. `mountShellStylesheet` validates against the same CSS profile and resource URL rules even before any surfaces exist. A rejected replacement leaves the previous shell sheet installed, and strict Preview reports the rejection even if JavaScript catches it.

For external images, declare read-only directory grants in `package.json`, e.g.
`"zgui": { "capabilities": { "imagePrefixes": ["/srv/wallpapers"] } }`.
Use local `file:///srv/wallpapers/example.svg` or a raster file URI as `Image.src`.
Paths must be canonical and absolute. On Linux, the loader confines symlink
resolution to the granted directory and SVGs cannot load external resources.
Preview resolves only exact URIs mapped to verified package assets in
[`services.images.files`](preview-mode.md#65-native-resource-readiness).

`shellSurfaces`, `sessionLock`, and `controlChannel` are boolean declarations;
`unixSockets` and `commands` are exact string allowlists (absolute paths for sockets).
These declarations are reserved for the corresponding host primitives; they do not
implement those primitives. Omitted flags and lists grant nothing. All-default
capabilities are omitted from `manifest.json`; non-default declarations enter `buildId`.

Declare application font family/resource mappings in `package.json`; each path must also be an asset:

```json
{
  "zgui": {
    "fonts": [{ "family": "Example Sans", "path": "assets/ExampleSans.ttf" }]
  }
}
```

Reference the declared family through StyleX `fontFamily`. The host registers verified bytes before mount, keeps registrations generation-safe across reload, and diagnoses unknown families instead of treating fallback as successful font loading. Preview capture waits for required native image/font readiness.

## Package identity and diagnostics

Package schema v2 publishes immutable matching artifacts:

- `app.mjs` and its composed source map;
- ordered `base.css` (when present) and generated `app.css`;
- `style-origins.json` with generated rule ranges and verified module-relative provenance;
- `build-info.json` with locked compiler/runtime/native identities and configuration;
- declared image/font assets and font mappings;
- `manifest.json` with per-artifact digests, the native-owned [`stylex-profile.json`](../../schemas/stylex-profile.json) digest, compiler configuration digest, and a canonical `buildId`.

Machine-specific source and temporary paths do not enter reproducible identities; explicitly declared host paths are part of the package identity. Publication and native acceptance are separate: a published candidate can still be rejected by native preflight, and the last successful application remains distinct. Watch builds retain `.zgui-build-status.json` for publication; the native host atomically writes `.zgui-native-status.json` beside the bundle symlink and emits a structured `native-candidate` JSON event with the accepted/rejected build and generation identities.

An accepted reload records `predecessorLeak` with `E_TEARDOWN`, the retired
generation, and its post-cleanup resource counts if that generation leaked.
The successfully mounted successor remains accepted.

The Preview CLI enables StyleX source metadata automatically. A release-like diagnostic package requires `bun run build --app NAME --stylex-diagnostics`; ordinary builds omit per-prop `data-style-src` but retain generated/module provenance. Both modes produce different configuration and artifact identities.

Strict native preflight rejects unsupported or partially parsed CSS before activation. This includes browser-root RTL wrappers (`html[dir]`/`html:not([dir])`), typed variables and non-dynamic `@property` forms, container queries, reduced-motion media queries, and CSS resource URL constructs. `defineConsts`, `stylex.types.*`, and unlisted browser CSS features are outside the v1 profile. Directional properties that cause StyleX to emit RTL variants require a later native profile.

Reload is full-session reload. CSS-only activation and state-preserving HMR are not part of this profile. Preview evidence records the consumed build, generation, native document/metadata/resource revisions, and stylesheet revision so a stylesheet-only SDK theme change cannot reuse stale frame evidence.

See the [StyleX example](../../apps/examples/stylex/src/main.tsx), [component map](component-map.md), and [Preview verification](preview-verification.md).
