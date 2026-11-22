> **Historical — Superseded by [`ADR-0006`](../../adr/0006-lightning-css-modules-replace-stylex.md).** Retained strictly as archival audit evidence.

# Upstream Doc Audit — StyleX + Solid Assumptions in STYLEX_INTEGRATION_SPEC.md

**Date:** 2026-11-18
**Spec audited:** `/home/hewel/Data/Downloads/STYLEX_INTEGRATION_SPEC.md` (v1.0, 2026-11-15)
**Method:** Source-level verification against pinned upstream commits. StyleX cloned at `6d7464dee9374f3ab99ddd37ae4de3accdc53ba3` ("[eslint-plugin] Declare eslint as a peer dependency (#1977)"); in-repo docs at that commit treated as the pinned official documentation. Solid cloned at `99714d4e2875d2d7ef8023e82f4ef6aa63ff0752` ("chore: version packages for 1.9.19 release"). Published npm metadata checked via registry. zgui (ZG refs) is out of scope for this note.

Citation shorthand: `stylex@6d7464d:<path>:<lines>` = file at the pinned commit, viewable at `https://github.com/meta/stylex/blob/6d7464dee9374f3ab99ddd37ae4de3accdc53ba3/<path>`; `solid@99714d4:<path>` likewise at `https://github.com/solidjs/solid/blob/99714d4e2875d2d7ef8023e82f4ef6aa63ff0752/<path>`.

---

## 1. Findings table (ordered by integration risk)

| #   | Finding                                                                                                                                                               | Verdict vs spec                                                   | Risk     |
| --- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------- | -------- |
| F1  | Output CSS depends on `:root` matching whenever tokens are used; untyped variables have **no** `@property` fallback                                                   | Spec understates (§5 flags it, but it is deeper than a test note) | **High** |
| F2  | Finalizer emits `html:not([dir='rtl'])` / `html[dir='rtl']` ancestors around RTL rules                                                                                | Not mentioned in spec                                             | **High** |
| F3  | `useLayers:false` output uses `:not(#\#)` specificity chains and doubled theme selectors `.x.x, .x.x:root`                                                            | Spec correct; native support is a hard gate                       | **High** |
| F4  | Pinned StyleX commit ≠ installable 0.19.2; pinned Solid commit = 1.9.19 but npm `latest` is now 2.0.0                                                                 | Spec caveat (§1) confirmed and sharpened                          | **High** |
| F5  | Dynamic numeric values are unit-suffixed **by compiled code** for length/time properties; NaN becomes `'NaNpx'` before the adapter                                    | Spec §4.3 correct but narrower than implied                       | Medium   |
| F6  | styleq needs `WeakMap` and `console.error`; drops `undefined` inline values unvalidated                                                                               | Not mentioned; host surface for rquickjs                          | Medium   |
| F7  | `dev` docs contradict source on runtime injection                                                                                                                     | Spec follows **source**, contradicts pinned docs                  | Medium   |
| F8  | Two-pass Babel order is safe; JSX preservation is only mandatory for the `sx` prop transform (disabled here)                                                          | Spec correct; one constraint relaxed                              | Medium   |
| F9  | Token modules must be transformed too; cross-file value resolution reads source from disk (Node fs at build time)                                                     | Spec correct (§2.3)                                               | Medium   |
| F10 | All option names/types/defaults in §2.1 verified                                                                                                                      | Correct                                                           | Low      |
| F11 | `processStylexRules` signature and layer behavior verified; README's older name is `processStyles`                                                                    | Correct                                                           | Low      |
| F12 | `props()`/`attrs()` shapes, empty-field omission, `styleq@0.2.1` as sole runtime dep; styleq is host-neutral pure JS                                                  | Correct                                                           | Low      |
| F13 | `.stylex.ts` supported; `themeFileExtension` default `.stylex`                                                                                                        | Correct                                                           | Low      |
| F14 | Omitted theme overrides revert to defaults; themes for the same VarGroup do not merge                                                                                 | Correct (docs + mechanism)                                        | Low      |
| F15 | Solid Universal `generate:"universal"` + `moduleName` passthrough; renderer `setProperty` receives object `style` as-is; Solid does not normalize `class`/`className` | Correct; typing TODO confirmed                                    | Low      |
| F16 | `stylex-inject` is `document`/`window`-based; runtime `stylex.js` has no host deps                                                                                    | Correct                                                           | Low      |

---

## 2. Detailed findings

### F1 — `:root` dependency of token CSS (High)

`defineVars` always emits the default-value rule with a `:root` selector (stylex@6d7464d:packages/@stylexjs/babel-plugin/src/shared/stylex-define-vars.js:113):

```js
const selector = `:root, .${varGroupHash}`;
```

`createTheme` likewise emits `.<override>, .<override>:root` (stylex@6d7464d:.../shared/stylex-create-theme.js:71):

```js
const rule = `.${overrideClassName}, .${overrideClassName}:root{${decls}}`;
```

`@property` rules with `initial-value` are emitted **only for typed variables** (`stylex.types.*`) — stylex@6d7464d:.../shared/stylex-define-vars.js:88-95. Plain string variables get no `@property`, so their default values exist **only** in the `:root, .xH` rule. A component using `colors.surface` with **no theme class applied** receives token values solely via `:root` matching; the `.varGroupHash` half of the selector only matches when a theme class is present (theme classes are `"<override> <varGroupHash>"`, stylex@6d7464d:.../shared/stylex-create-theme.js:84). Spec §5 ("Test default-token selectors against the actual native document root … Do not assume an HTML root exists") points the right direction but frames it as a test; it is an **output-shape dependency**: either zgui must match `:root` against the native document root, or every token consumer must carry a theme/varGroup class, or the SDK must inject defaults another way. Note `@property` (`syntax`, `inherits:true`, `initial-value`) is itself a native capability question for typed tokens.

### F2 — `html[dir=...]` ancestor selectors in finalized CSS (High)

When a rule has an `rtl` variant and `enableLTRRTLComments` is false (the default), the finalizer wraps both directions in browser-root ancestor selectors (stylex@6d7464d:packages/@stylexjs/babel-plugin/src/index.js:966-985):

```js
: [addAncestorSelector(ltrRule, "html:not([dir='rtl'])"),
   addAncestorSelector(rtlRule, "html[dir='rtl']")]
```

`rtl` variants arise from physical-property LTR/RTL flipping. Spec §8 mentions "directional output / reject browser-root assumptions" as a profile row but does not record that **the official finalizer itself bakes `html` into the selector** — no authoring-level avoidance is possible short of never producing `rtl` rules or post-processing (which §2.3 forbids). This needs a native decision (map document direction into a matching mechanism, or gate such output as unsupported with diagnostics).

### F3 — Priority selector shapes with `useLayers: false` (High)

With `useLayers:false, legacyDisableLayers:false`, each priority group gets `:not(#\#)` specificity chains appended (stylex@6d7464d:.../babel-plugin/src/index.js:1039-1052):

```js
// Adds :not(#\#) to bump up specificity. as a polyfill for @layer
const pseudo = Array.from({ length: index })
  .map(() => ":not(#\\#)")
  .join("");
```

and theme-like rules matching `/\.([a-zA-Z0-9]+), \.([a-zA-Z0-9]+):root/` are rewritten to doubled classes (stylex@6d7464d:.../babel-plugin/src/index.js:946-956: `.$1.$1, .$1.$1:root`). Spec §2.3 ("retains the official specificity mechanism, including generated priority selectors … Native support for that exact output is an acceptance gate") is exactly right; the exact selector atoms zgui must support are `:not()` with an ID argument and repeated-class selectors.

### F4 — Pinned commits vs installable packages (High)

- **StyleX:** manifests at the pinned commit declare `0.19.2` for `@stylexjs/babel-plugin`, `@stylexjs/stylex`, and `@stylexjs/shared` (stylex@6d7464d:packages/@stylexjs/babel-plugin/package.json:3, .../stylex/package.json:3, .../shared/package.json:3). But the upstream tag `0.19.2` points at `d7e6174c5384c64b87bc5fde1f57eedd7c11c5aa`, **not** at `6d7464d`; the pinned commit is one commit after the release. `git diff d7e6174..6d7464d -- packages/@stylexjs/babel-plugin/src/index.js` shows ~40 changed lines in `processStylexRules` (single-pass const-var inlining rewrite, `VAR_USAGE_REGEX`/`OVERRIDE_KEY_REGEX`). So `npm i @stylexjs/babel-plugin@0.19.2` does **not** reproduce the audited source exactly; the delta only affects output when `defineConsts` is used (absent from the spec), but the installable artifact is not identical to the audit baseline.
- **npm:** 0.19.2 is currently the latest published version of both packages (registry check 2026-11-18); `styleq` latest is 0.2.1, matching the runtime dependency `styleq: 0.2.1` (stylex@6d7464d:packages/@stylexjs/stylex/package.json).
- **Solid:** the pinned commit is the 1.9.19 release commit; `solid-js` and `babel-preset-solid` are 1.9.19 there (solid@99714d4:packages/solid/package.json, .../babel-preset-solid/package.json). Registry caveat: `solid-js` latest is now **2.0.0**, so an unpinned install diverges immediately; `babel-preset-solid` latest is still 1.9.19. Published `babel-preset-solid@1.9.19` depends on `babel-plugin-jsx-dom-expressions ^0.40.14`, matching the repo pin (solid@99714d4:package.json). Published `solid-js@1.9.19` deps (`csstype`, `seroval`) are pure JS; its `universal` entry is pre-bundled and imports only from `solid-js` (verified from the published tarball), so `dom-expressions` does not need vendoring into rquickjs.

Spec §1's caveat row ("Source manifests declare 0.19.2; verify and lock installable compiler/runtime packages before implementation") is confirmed as a real, material caveat.

### F5 — Dynamic-style numeric behavior (Medium)

For non-confident (dynamic) values, the plugin compiles a runtime wrapper (stylex@6d7464d:packages/@stylexjs/babel-plugin/src/visitors/parse-stylex-create-arg.js:218-249): for properties in `timeUnits`/`lengthUnits` the emitted code is effectively `val => typeof val === 'number' ? val + 'px' /* or 'ms' */ : val != null ? val : undefined`; for everything else `val != null ? val : undefined`. Consequences:

- The adapter **will receive unit-bearing strings** (e.g. `'100px'`) for dynamic length/time custom properties; raw numbers arrive only for unitless properties (the `unitlessNumberProperties` set, stylex@6d7464d:.../shared/utils/transform-value.js:125-166) and custom properties (`getNumberSuffix` returns `''` for `--*` keys, lines 110-112). Spec §4.3's "finite number in a compiler-produced custom property → serialize as a CSS number token without adding units" is correct within that scope.
- `null`/absent dynamic values become `undefined` and are dropped from the `style` object by styleq (see F6), so adapter removal-by-omission (§4.3 "missing key … remove") matches upstream behavior.
- The wrapper does no NaN/Infinity check: `typeof NaN === 'number'`, so a NaN dynamic length arrives at the adapter as the **string** `'NaNpx'` — §4.3's "reject NaN" can only be enforced by per-property grammar validation, not by type detection.
- Spec §3.1's advice "Prefer explicit units in authored dynamic values" is compatible but slightly overstated as a necessity for length/time props (numbers are auto-suffixed by the compiler); it remains necessary for unitless/custom props and for non-length values.

Static numeric values are compile-time suffixed via the same `getNumberSuffix` (`px` default, `ms` for time properties, none for unitless/custom). Zero-value normalization (`normalizeZeroDimensions`) explicitly **skips** custom properties (stylex@6d7464d:.../shared/utils/normalizers/zero-dimensions.js:25-27), supporting §4.3's zero-handling rule.

### F6 — styleq host surface (Medium)

`styleq@0.2.1` (published tarball, `dist/styleq.js`) is pure JS with no imports — host-neutral in principle, but:

- Uses `new WeakMap()` for its cache (line 8): rquickjs/QuickJS must expose `WeakMap`.
- On compiled (`$$css`) entries whose values are not `string`/`null`, it calls `console.error("styleq: …")` and skips the entry (lines 64-66) — a `console` binding must exist or the merge throws.
- Inline values pass through unvalidated except `undefined` (dropped); numbers/booleans/objects flow into the `style` object as-is (lines 90-110). This is why §4.3's adapter-side validation of the `style` map is required, and why §2.4's "no broad substring bans, use real-engine smoke execution" is the right call.
- Merge order: first-seen property wins via a shared `definedProperties` list across class and inline branches; dynamic-function results arrive as `[compiledObj, inlineObj]` tuples that styleq flattens (lines 34-38). Spec §3.2's "official merge API, not class-token order" is consistent with this.

### F7 — `dev` docs vs source contradiction (Medium)

Pinned official docs (stylex@6d7464d:packages/docs/content/docs/api/configuration/babel-plugin.mdx, `dev` entry) say: "When `true`, StyleX will insert function calls to inject the CSS rules at runtime, making it possible to use StyleX without setting up CSS file extraction." The source at the same commit contradicts this: injection is driven solely by `runtimeInjection` (`options.runtimeInjection ?? false`, stylex@6d7464d:.../babel-plugin/src/utils/state-manager.js:319-334; `DEFAULT_INJECT_PATH = '@stylexjs/stylex/lib/stylex-inject'`, line 161; `runtimeInjection` getter returns null when unset, lines 522-529). `dev` instead gates dev-readable class names (`injectDevClassNames`, stylex@6d7464d:.../visitors/stylex-create.js:273-276) and `debug` defaults to `dev` (state-manager.js:223-229). Spec §2.1 ("At the pinned source, injection configuration is independent of `dev`; set both flags deliberately") follows the **source** and is correct; treat the docs sentence as stale.

### F8 — Two-pass Babel order (Medium)

- The plugin's JSX visitor exists only to rewrite the `sx` prop and returns early when `sxPropName === false` (stylex@6d7464d:packages/@stylexjs/babel-plugin/src/index.js:309-312). With the spec's profile, the plugin never inspects JSX, so "Run StyleX … while preserving JSX" (§2.2 step 2) is sufficient but not strictly necessary for the plugin to function; the hard ordering rule in upstream materials applies to the `sx` shorthand (docs `sxPropName` entry) and to not letting a React JSX transform run before a React-mode StyleX setup — irrelevant here since pass 4 is Solid universal, which expects intact JSX anyway (F15). No hidden constraint contradicts the spec's two-pass plan.
- Real constraints to keep: pass 1 must pass `filename` (debug provenance falls back to `UnknownFile`, stylex@6d7464d:.../utils/dev-classname.js:34; module-resolution hashing returns null without it, state-manager.js:557-558); `metadata.stylex` must be read from pass 1's result before pass 2 (spec step 3 does this; metadata shape is `[classNameHash, {ltr, rtl?} | {constKey, constVal, rtl?, ltr}, number]`, state-manager.js:702-716); source-map chaining is ordinary Babel input-map composition, and `addSourceMapData` consumes pass 1's `codeMap` for original line numbers in `data-style-src` (stylex@6d7464d:.../utils/add-sourcemap-data.js:100-118, 137-142), so pass 1 should map back to the original TSX, not to an intermediate.
- The plugin build host needs Node (`node:path`, `node:fs`, `node:url`, `@dual-bundle/import-meta-resolve` in state-manager.js:28-33); the repo ships a `lib/index.browser.js` build that stubs `node:fs` to `{}` (stylex@6d7464d:packages/@stylexjs/babel-plugin/rollup.config.mjs:74-84), which silently breaks token-file resolution — the native build should run the compiler on Node, not in rquickjs.

### F9 — Token module compilation and cross-file resolution (Medium)

Cross-file token references are resolved by parsing and evaluating the imported file at build time (`evaluateImportedFile` via `importPathResolver`, stylex@6d7464d:.../babel-plugin/src/utils/evaluate-path.js:745-762; resolver in state-manager.js:644-690 gated on `unstable_moduleResolution` and the `.stylex` suffix, default extension at state-manager.js:560-561). The `:root`/variable rules for a group enter `metadata.stylex` **only** from the token file's own transform. So spec §2.3 ("Include imported `.stylex.ts` token definitions and any dependency sources") is not just good practice — omitting the token module from the transform set loses its CSS even though consumer value resolution would still succeed. Also: file-based hash identity (`fileNameForHashing`, state-manager.js:549-577) requires `unstable_moduleResolution` to be set, and `commonJS` canonical paths are derived by walking up to the nearest `package.json` on disk (state-manager.js:594-618) — reproducible hashes therefore depend on the on-disk layout, matching §6's "keep machine-specific temporary paths out of reproducible identities" concern (use stable package roots).

### F10 — Compiler options (Low; spec correct)

Verified against stylex@6d7464d:.../babel-plugin/src/utils/state-manager.js (option parsing) and the pinned docs page (packages/docs/content/docs/api/configuration/babel-plugin.mdx):

| Option                      | Spec value                   | Source behavior                                                                                                                                                                                                                                          | Match                                                                             |
| --------------------------- | ---------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------- |
| `dev`                       | `false`                      | bool, default `false` (l.216-221)                                                                                                                                                                                                                        | yes                                                                               |
| `runtimeInjection`          | `false`                      | bool                                                                                                                                                                                                                                                     | string                                                                            | {from,as}, default `false`; `true`→`'@stylexjs/stylex/lib/stylex-inject'` (l.319-334, 161)             | yes |
| `test`                      | `false`                      | bool, default `false` (l.312-317); emits `{$$css:true}` + `file__namespace` dev classes, no rules (dev-classname.js:51-62; docs: "will _not_ generate any styles or functional classNames") — spec's "unsuitable for real pixel evidence" correct        | yes                                                                               |
| `debug`                     | conditional `true`           | bool, default `dev`; gates `data-style-src` via `enableDebugDataProp` (default `debug`) + `addSourceMapData` emitting `$$css: "<shortFilename>:<line>"` (stylex-create.js:268-272, add-sourcemap-data.js:135-144); consumed by styleq into `debugString` | yes ("can add" is accurate — separately disableable)                              |
| `styleResolution`           | `"property-specificity"`     | enum `'application-order'                                                                                                                                                                                                                                | 'property-specificity'                                                            | 'legacy-expand-shorthands'`(docs list only the first two), default`'property-specificity'` (l.355-369) | yes |
| `sxPropName`                | `false`                      | string                                                                                                                                                                                                                                                   | false, default `'sx'`; `false` disables the JSX visitor (l.305-310; index.js:311) | yes                                                                                                    |
| `unstable_moduleResolution` | `{type:"commonJS", rootDir}` | union of `commonJS`/`haste`/`custom`/`experimental_crossFileParsing` (l.37-56); docs literally say of `commonJS`: "Use this value when using `ESModules`" — spec's "does not require CommonJS output" correct                                            | yes                                                                               |
| `importSources`             | implied default              | default `[]`, always prepended with package name and `'stylex'` (l.341-353); accepts `string                                                                                                                                                             | {from, as}` (l.134-135). SDK re-export needs an explicit entry — spec correct     | yes                                                                                                    |

### F11 — `processStylexRules` (Low; spec correct)

Signature (stylex@6d7464d:.../babel-plugin/src/index.js:686-705): `processStylexRules(rules: Array<Rule>, config?: boolean | { useLayers?: boolean | {before?, after?, prefix?}, enableLTRRTLComments?, legacyDisableLayers?, useLegacyClassnamesSort? }): string`. Spec's call with `{useLayers:false, legacyDisableLayers:false}` is valid. Behavior: `useLayers` defaults false (l.712-713); with layers off and `legacyDisableLayers:false`, `addSpecificityLevel` applies (l.943-945); `legacyDisableLayers:true` skips the priority bumps entirely (legacy raw output). The older name is real: the babel-plugin README intro says to use "the `processStyles` function" (stylex@6d7464d:packages/@stylexjs/babel-plugin/README.md:5) while the actual export is `styleXTransform.processStylexRules` (index.js:1009) — spec §2.3's warning is accurate. Dedup is by rule key per priority group (`new Map(rules.map(([a,b]) => [a,b]))`, index.js:919); spec's "owns rule deduplication and priority handling" and "do not concatenate raw rule text in async order" are consistent with the implementation (rules are sorted by priority/width keys before grouping, index.js:880-906).

### F12 — Runtime props/attrs (Low; spec correct)

stylex@6d7464d:packages/@stylexjs/stylex/src/stylex.js:144-172 (`props`): returns `{className?, 'data-style-src'?, style?}`, omitting empty/`''` fields, delegating to `styleq(styles)` → `[className, style, dataStyleSrc]`. Lines 175-216 (`attrs`): returns `{class?, 'data-style-src'?, style: string}` with kebab-cased, `;`-joined declarations — spec §3.2's deferral rationale ("returns a serialized style string") is accurate. Untransformed compile-time APIs throw: `Unexpected 'stylex.create' call at runtime. Styles must be compiled by '@stylexjs/babel-plugin'.` (stylex.js:33-36, 42-44, 70-72; also `createTheme`, `defineVars`, `defineConsts`, `keyframes`, `firstThatWorks`, `positionTry`, `types.*`) — spec §2.4 correct. Sole runtime dependency: `styleq: 0.2.1` (stylex/package.json); package exports `./lib/stylex-inject` separately (stylex/package.json), and `inject.js` + `stylesheet/createSheet.js` use `document`/`window` (stylex@6d7464d:.../stylex/src/stylesheet/createSheet.js:27, utils.js:11-13) — browser-only, correctly rejected by §2.4. `props`/`attrs`/`styleq` themselves have no DOM usage.

### F13 — `.stylex.ts` and variable definitions (Low; spec correct)

`.stylex.ts` is a documented valid extension (stylex@6d7464d:packages/docs/content/docs/learn/theming/defining-variables.mdx:97-104; loader suffix list `['.js','.ts','.tsx','.jsx','.mjs','.cjs']`, state-manager.js:923; `themeFileExtension` default `'.stylex'`, state-manager.js:560-561). `defineVars` returns `{'key': 'var(--xHash)', …, __varGroupHash__}` (stylex-define-vars.js:73-78, 116-121) — spec §5's usage shape matches.

### F14 — Theme reset/inheritance semantics (Low; spec correct)

Official docs (stylex@6d7464d:packages/docs/content/docs/learn/theming/creating-themes.mdx:60-61): "Any variables that are not overridden will revert back to their default value that was set in the `defineVars` declaration." The reset-themes recipe adds: "Any variable in a VarGroup that is not explicitly overridden in a Theme for that VarGroup is set to its default value" and "`Theme`s for the same `VarGroup` are mutually exclusive and do not merge" (…/learn/recipes/reset-themes.mdx). Mechanism, from source: the theme class is `"<overrideHash> <varGroupHash>"` (stylex-create-theme.js:84), so applying a theme also applies the varGroup class whose `:root, .varGroupHash` rule carries the defaults; the override rule only declares overridden variables. Net effect matches spec §5 ("Overrides omitted … follow StyleX's default/reset semantics; … not an arbitrary merge with a parent theme"). Caveat for the native side: this reset behavior **depends on the same `:root`/varGroup selector matching as F1**.

### F15 — Solid Universal configuration (Low; spec correct)

- `babel-preset-solid` is a thin wrapper that forwards options to `babel-plugin-jsx-dom-expressions` (solid@99714d4:packages/babel-preset-solid/index.js:5-29); `generate` defaults to `"dom"` and `moduleName` to `"solid-js/web"`, so the spec's `generate: "universal"` + project renderer module is exactly the supported knob (confirmed by the Universal README, solid@99714d4:packages/solid/universal/README.md, Babel section).
- Prop delivery: renderer authors implement `setProperty(node, name, value)`; the README's example special-cases `style` as an object (`if (name === "style") Object.assign(node.style, value)`) — i.e. **Solid universal delivers the `style` object as-is and performs no diffing/merging**; per-key reconciliation (spec §4.4) is entirely the renderer/adapter's job, as the spec assumes. Spread props route through the renderer's exported `spread`/`setProperty`; there is no DOM-style `class`→`className` normalization in universal mode — the README's own `PROPERTIES` set contains `className`, meaning a `class` prop would fall through to `setAttribute` unless the project renderer handles both. Spec §4.2's class/className aliasing therefore must live in the project's `setProperty`, which is consistent with the spec's design.
- Typing: "For TypeScript support of non-standard JSX you will need to provide your own types at a jsx-runtime entry on your package so that it can be set as the `jsxImportSource`" (README, closing note) — a real TODO for the project's renderer and for spec §2.2 step 1 (type-check against the native SDK).
- The preset does not strip TypeScript; pass 4 must pair it with TS syntax handling, matching spec §2.2 step 4's "then remove TypeScript syntax."

### F16 — Host assumptions summary (Low; spec correct)

Browser-only or host-sensitive surfaces found: `stylex-inject`/`stylesheet/*` (`document`, `window`); the **compiler** (not runtime) requires Node built-ins and on-disk token files (F8/F9); finalized CSS may contain `:root`, `html[dir]`, `@layer` (if enabled), `@property`, `:not(#\#)`, and doubled-class selectors (F1-F3); styleq needs `WeakMap`/`console` (F6). The runtime merge path (`@stylexjs/stylex` `props`/`attrs` + `styleq`) is pure JS with no imports beyond `styleq` — compatible with rquickjs given those two globals. Nothing in the audited source requires a DOM at runtime under the spec's profile (`runtimeInjection:false`, `dev:false`, `test:false`).

---

## 3. Explicit confirmations (spec is correct)

1. §1/§2.1/§2.2: full compiler-option table, including `sxPropName: false`, `styleResolution: "property-specificity"`, and the `commonJS`-resolver-doesn't-require-CJS-output note (F10).
2. §2.1: "injection configuration is independent of `dev`" — matches source, **against** the pinned docs (F7).
3. §2.1: "`test: true` creates test-oriented compiled classes … unsuitable for real pixel evidence" — matches source and docs (F10).
4. §2.2: pass-1 metadata snapshot, `importSources` requirement for SDK re-exports, parser-plugin selection (F8, F10).
5. §2.3: `processStylexRules` naming (vs README's `processStyles`), signature, `useLayers:false/legacyDisableLayers:false` behavior, dedup/priority ownership, ban on raw concatenation and selector post-processing (F11).
6. §2.4: compile-time API stubs throw at runtime; reject `stylex-inject` (document-based); bundle real merge runtime only as needed (F12, F16).
7. §3.1/§3.2: dynamic styles compile to static class + runtime-set CSS variable (docs, defining-styles.mdx:263-266); merge via official API; `attrs()` deferred due to serialized string output (F5, F12).
8. §4.1: `props()` output shape and empty-field omission exactly as spec's `CompiledStyleProps` (F12).
9. §5: `.stylex.ts` modules, `defineVars`/`createTheme` shapes, omitted-override reset semantics, non-merge of same-group themes (F13, F14).
10. §2.2/§6: `unstable_moduleResolution` needed for theming APIs and stable file-based hashing; reproducibility depends on on-disk layout (F9, F10).
11. §2.2 (BUILD1): Solid universal `generate:"universal"` + renderer `moduleName`; JSX must not be lowered before pass 4 (F15).

## 4. Contradictions and missing evidence (separated from product choices)

**Contradictions found (upstream-internal):**

- C1: Official docs say `dev:true` inserts runtime injection calls; source at the same commit says injection is independent of `dev` (F7). Spec follows source. Treat docs as stale.
- C2: Official docs list only two `styleResolution` values; source accepts a third, `'legacy-expand-shorthands'` (state-manager.js:356-360). No spec impact (spec uses a documented value).
- C3: Official docs list only `commonJS`/`haste` for `unstable_moduleResolution`; source also accepts `custom` and `experimental_crossFileParsing` (state-manager.js:37-56). No spec impact.

**Missing / not fully evidenced (requires implementation-time verification, not more reading):**

- M1: Whether zgui (revision `80baa3c…`) matches `:root`, `html[dir='rtl']`, `:not(#\#)` with ID args, doubled classes, and `@property` — cross-reference the zgui audit (ZG refs); cannot be resolved from StyleX/Solid sources.
- M2: Whether rquickjs's QuickJS build exposes `WeakMap` and a usable `console` (F6).
- M3: `babel-plugin-jsx-dom-expressions@0.40.14`'s exact universal spread codegen was not read (dependency not vendored in the Solid repo; the universal README example at the pinned commit is the authoritative statement, F15). Published `babel-preset-solid@1.9.19` dependency range `^0.40.14` verified from registry.
- M4: The exact delta between npm `0.19.2` and the pinned commit is limited to `processStylexRules` const-inlining (only exercised by `defineConsts`, unused by the spec) plus non-runtime package metadata — verified by direct diff; a from-source build or accept-the-delta decision is still the project's to make (F4).
- M5: Live stylexjs.com docs may drift from the pinned in-repo docs; all doc claims above were verified at the pinned commit only.

**Product choices (not upstream facts — listed to keep the audit honest):**

- P1: Deferring `attrs()`; P2: `useLayers:false` over `@layer` output; P3: `class` canonical + `className` alias with `CLASS_ALIAS_CONFLICT`; P4: adapter-side validation/rejection table in §4.3; P5: Preview identity/revision scheme in §9; P6: manifest fields in §6. These are internally consistent with upstream behavior but are project decisions; no upstream confirmation is expected or needed.

## 5. Decisions that genuinely require the user

1. **Installable vs audited StyleX:** accept `npm @stylexjs/stylex@0.19.2 + @stylexjs/babel-plugin@0.19.2` (release tag `d7e6174`, tiny delta from the audit), or build/vendor from pinned commit `6d7464d` to reproduce the audited source exactly? (F4, M4)
2. **Solid version lock:** confirm `solid-js@1.9.19` + `babel-preset-solid@1.9.19` (npm `solid-js` latest is 2.0.0 and will diverge). (F4)
3. **Native root strategy for tokens:** should zgui-style native output emulate `:root` (and `@property` initial-value) for StyleX token rules, or should the SDK guarantee a varGroup/theme class on every token consumer so defaults never depend on `:root`? (F1, F14)
4. **Directional/RTL handling:** how should the native host make `html:not([dir='rtl'])`-wrapped finalizer output match — a native direction attribute mapped into selector matching, or declare flipped-property output unsupported at this baseline (diagnostic per §8)? (F2)
5. **Priority selector contract:** confirm acceptance of `:not(#\#)` chains and doubled-class theme selectors as permanent native requirements (vs a future `@layer` profile revision), since §2.3 makes this an acceptance gate. (F3)
6. **`data-style-src` in release:** keep `debug` Preview-only (spec §2.1 leans this way) or allow diagnostic builds in production profiles — affects the adapter's required handling of the prop outside Preview. (F10)
