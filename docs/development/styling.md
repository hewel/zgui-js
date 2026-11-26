# Styling Guide & Stylo Fast Paths

Canonical styling guide for `zgui-js` applications and Wayland desktop shells (`@hewel/zgui`, `lightningcss-v1`, `ui-bridge`, and `zgui-style`).

**Core Rule:** _Put the Rule in CSS; Keep Runtime Values Inline._

Static layout, tokens, finite states, and interaction pseudo-classes belong in `src/app.css` and locally scoped `*.module.css` files. Inline `style={{ ... }}` (`NativeStyle`) is reserved for continuous runtime values that cannot be enumerated ahead of time.

## 1. Core Rule & Decision Table

| Use case                                                                                                                           | Where it belongs                                                                | Why                                                                                                     |
| ---------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| Static layout, spacing, typography, colors, borders, radii                                                                         | `src/app.css` / `*.module.css`                                                  | Enables Stylo sharing (`SharedStyles`, `CascadeMemo`) and CSS-only hot swap (`reload-css-swapped`)      |
| Theme & design tokens (`--zui-*`, static `--*`)                                                                                    | `src/app.css` / `*.module.css`                                                  | Preserved verbatim (`dashedIdents: false`); no QuickJS property writes                                  |
| Finite discrete states (active, selected, size, tone)                                                                              | `[data-*]` selectors or `cx(...)` classes in `*.module.css`                     | Even `active() ? "var(--a)" : "var(--b)"` selects between two fixed rules and belongs in CSS            |
| Hover, press, focus, disabled, checked                                                                                             | `:hover`, `:active`, `:focus`, `:focus-visible`, `:disabled`, `:checked` in CSS | Zero JS bridge traffic, filtered per-element by `StyleDependencies`, and not shadowed by inline `style` |
| State-driven transitions & keyframe animations                                                                                     | `transition` / `@keyframes` in `*.module.css`                                   | Runs natively in `zgui-style` without per-frame QuickJS turns                                           |
| Continuous physics motion (`createSpring` `transform` / `opacity` / `scale`, measured collapsible height)                          | Inline `style={{ ... }}` (`NativeStyle`)                                        | Value varies continuously per frame                                                                     |
| Continuous geometry & ratios (`SplitPane` ratio, progress `width: "${pct}%"`, `VirtualList` offsets, dynamic anchor/Popper coords) | Inline `style={{ ... }}` (`NativeStyle`)                                        | Arbitrary numeric geometry computed at runtime                                                          |
| Arbitrary user/runtime values (color-picker colors, dynamic numeric `--*` variables)                                               | Inline `style={{ ... }}` (`NativeStyle`)                                        | Unbounded value domain; cannot be listed in CSS                                                         |

### Concrete Before / After

**Anti-pattern:** putting static layout, discrete ternaries, or hoverable backgrounds inline on TSX nodes:

```tsx
// Bad: blocks Stylo style sharing, triggers JS reloads on edits, and shadows CSS :hover
<row
  style={{
    display: "flex",
    padding: "8px 12px",
    gap: "8px",
    backgroundColor: active() ? "var(--zui-color-accent)" : "var(--zui-color-card)",
    transform: `translateX(${offset()}px)`,
  }}
/>
```

**Idiomatic `zgui-js`:** keep static layout and discrete states in `*.module.css`, and split only the continuous runtime value onto `style`:

```css
/* item.module.css */
.row {
  display: flex;
  padding: 8px 12px;
  gap: 8px;
  background-color: var(--zui-color-card);

  &:hover {
    background-color: var(--zui-color-muted);
  }

  &[data-active="true"] {
    background-color: var(--zui-color-accent);
  }

  &:focus-visible {
    outline: 2px solid var(--zui-color-ring);
    outline-offset: -2px;
  }
}
```

```tsx
// Good: static rules & states in CSS; only the continuous transform stays inline
import { cx } from "@hewel/zgui";
import styles from "./item.module.css";

<row
  class={cx(styles.row)}
  data-active={active() ? "true" : "false"}
  style={{ transform: `translateX(${offset()}px)` }}
/>;
```

Separating `class={styles.row}` from `style={{ transform: ... }}` also avoids inline `backgroundColor` overriding `.row:hover` in the CSS cascade (inline `StyleOwner::Application` declarations always beat author stylesheet rules).

### Deterministic Cascade & `lightningcss-v1` Subset

`@hewel/zgui-build` (`packages/build/src/css.ts`, `packages/build/src/compiler.ts`, `packages/build/src/build.ts`) compiles stylesheets with Lightning CSS (`profile: "lightningcss-v1"`, `targets: null`, `dashedIdents: false`, `Features.Nesting` flattened) in a fixed cascade order:

1. `@layer zgui.components` — `components.css`, statically hoisted from `@hewel/zgui/components`
2. Global `src/app.css` — optional, unlayered
3. Locally scoped `*.module.css` — unlayered, deterministically merged across the reachable module graph (`CSS_MODULES_PATTERN = "[hash]_[local]"`)
4. Inline `style` (`StyleOwner::Application`, flanked by SDK `Defaults` and `Invariants`)

Because Stylo is not a browser engine, `targets` is `null` (no vendor prefixes or static `oklch`/`color-mix` fallbacks) and `dashedIdents` is `false` so literal `--zui-*` and `--*` variables resolve across nested `ThemeProvider` and `Portal` scopes. `packages/build/src/css.ts` rejects the following unsupported v1 features at build time:

- `@import`, `url()`, `image-set()`, `-webkit-image-set()` (use `assets/` manifest images via `<Image>` / `<image>`)
- `@font-face` (declare package fonts in `package.json` `zgui.fonts`)
- `@container`
- `@media (prefers-reduced-motion: ...)` (use `getReducedMotion()` or `--zui-motion-duration-*` tokens)
- `@property` (all `@property` rules are rejected)
- `html[dir]`
- CSS Modules `composes`, ICSS `:import` / `:export`, and `@value` (compose multiple classes in TSX with `cx(...)`)

## 2. Bridge & `NativeStyle` Contracts

Authoritative sources: [`crates/ui-bridge/src/properties.rs`](../../crates/ui-bridge/src/properties.rs), [`packages/ui/src/styles.d.ts`](../../packages/ui/src/styles.d.ts), and [`crates/zgui-host/src/session_host.rs`](../../crates/zgui-host/src/session_host.rs).

### Atomic Block Replacement (`StyleOwner::Application`)

- Both `style="..."` (`Style::Text`) and `style={{ ... }}` (`Style::Object`) write `StyleOwner::Application` atomically through `dom.replace_inline_style(node, &entries)`.
- **Whole-block replacement:** Every `style` update replaces all application inline declarations on that node and clears any prior direct `--*` property writes (`state.direct.clear()`). Setting a property to `null` or `undefined` clears it on the next commit.
- **SDK isolation:** Application inline styles never overwrite SDK `StyleOwner::Defaults` (`__zguiStyle`) or `StyleOwner::Invariants` (`__zguiInvariantStyle`).
- **Coexisting properties & variables:** When an element needs both dynamic ordinary properties and dynamic `--*` custom properties, emit them together in a single `NativeStyle` object (or a single string) so one update does not wipe the other.
- **Validate-before-commit:** Property names and values are parsed and validated before any native DOM mutation occurs (`STYLE_NAME_INVALID`, `STYLE_VALUE_INVALID`, `STYLE_PARSE_INVALID`).

### Unitless Number Allowlist

In `NativeStyle` objects (`packages/ui/src/styles.d.ts`, matching `UNITLESS` in `crates/ui-bridge/src/properties.rs`), finite JavaScript numbers are accepted **only** for `--*` custom properties and these ten unitless CSS properties (plus their camelCase aliases):

- `opacity`
- `flex`
- `flex-grow` (`flexGrow`)
- `flex-shrink` (`flexShrink`)
- `font-weight` (`fontWeight`)
- `line-height` (`lineHeight`)
- `z-index` (`zIndex`)
- `order`
- `animation-iteration-count` (`animationIterationCount`)
- `scale`

All length, angle, and distance properties require explicit CSS strings: passing `{ width: 100 }` throws `STYLE_VALUE_INVALID` — write `{ width: "100px" }`. Likewise, `translate` and `rotate` require CSS strings even for zero (for example `"0px"` or `"0deg"`).

### Dev-Loop Reload Behavior

- **CSS-only edits:** When a rebuild changes only packaged CSS (`is_css_only_successor` in `crates/zgui-host/src/session_host.rs` and `session_host/surfaces.rs`), `zgui-host` preflights and hot-swaps Stylo stylesheets in place via `replace_packaged_stylesheets` (`reload-css-swapped: generation=...`) without restarting QuickJS or losing local component state.
- **TSX inline style edits:** Editing `style={{ ... }}` in `.tsx` changes the compiled JS bundle and triggers a full module reload (`reload-mounted: generation=...`). Keeping styles in `src/app.css` and `*.module.css` preserves the fast CSS-only swap path.

## 3. Stylo 0.19 & `zgui-style` Fast Paths

Authoritative sources: [`vendor/zgui-style/`](../../vendor/zgui-style) and [`vendor/zgui-dom/src/side/readers.rs`](../../vendor/zgui-dom/src/side/readers.rs).

### Selector Matching & Invalidation

- **Rightmost key & Ancestor Bloom Filter:** Stylo indexes rules in `SelectorMap` by their rightmost (key) selector and filters ancestor combinators using a per-worker `bloom_filter` hashing ancestor tag names, IDs, and classes (`compute_style` in `vendor/zgui-style/src/driver/recalc.rs`).
- **Avoid Bloom-bypassing ancestor selectors:** Substring attribute selectors (`[attr*="..."]`, `[attr^="..."]`, `[attr$="..."]`) on ancestors cannot use the Bloom filter and force slower ancestor walks.
- **Keep key selectors specific:** Anchor key selectors on classes (`.row`, `.thumb`), prefer direct child combinators (`>`) over deep descendant combinators, and avoid universal `*` or bare `:hover` key selectors.
- **CSS Modules scope names, not invalidation:** Hashing `.row` to `.[hash]_row` prevents class collisions, but does not shrink invalidation subtrees. Prefer toggling state directly on the item (`.row[data-selected="true"]` or `cx(styles.row, selected() && styles.selected)`) rather than mutating a `data-*` attribute on a high ancestor and matching downward.

### Three-Tier Style & Cascade Sharing

`zgui-style` eliminates redundant cascade work across three tiers:

1. **Upstream `StyleSharingCache`:** `StyleSharingTarget::share_style_if_possible` shares resolved styles across sibling/cousin elements and compares inline `style` attributes by block pointer/value.
2. **`SharedStyles` (`vendor/zgui-style/src/driver/share.rs`):** Deduplicates equal `ComputedValues` per worker across traversals on an element's first cascade (up to `PER_CHAIN = 4` distinct styles per `StrongRuleNode`), while preserving each text-holding element's `InheritedText` allocation (`with_own_text`) so Parley keeps separate shaped paragraphs per box width.
3. **`CascadeMemo` (`vendor/zgui-style/src/driver/memo.rs`):** Memoizes eligible `CascadeOnly` cascades within a traversal (up to `PER_CHAIN = 4` results per `(StrongRuleNode, flags)`), skipping re-cascading identical rows/cells in lists when an inherited custom property or parent style moves.

> **Important:** Both `SharedStyles::share` and `CascadeMemo` (`Inputs::of`) explicitly bail out whenever `element.style_attribute().is_some()` — **even when two elements have identical static inline styles**. Moving static styles from `style={{ ... }}` into a CSS class unlocks both `SharedStyles` and `CascadeMemo`. Also avoid unique `#id` selectors for styling and heavy `:nth-child(...)` rules on large lists.

### Style Struct Inheritance & Custom Property Bloom Filter

- **Inherited vs. reset properties (`vendor/zgui-style/src/driver/recalc.rs`):** Put inherited typography and foreground properties (`font-family`, `font-size`, `line-height`, `color`) on `:root` or outer containers; never reset inherited properties on `*`.
- **Reset-only parent changes do not recascade ordinary children:** `narrowed_children_hint` strips `RestyleHint::RECASCADE_SELF` when only reset style structs (such as `background-color` or `border`) changed on the parent, and `note_children` clears `RECASCADE_SELF_IF_INHERIT_RESET_STYLE` unless a child explicitly uses `inherit` on a reset property.
- **64-bit `--*` reader Bloom filter:** When an element's inherited custom property map changes, `ChangedNames` records the changed variable names and tests each child's `zgui_dom::side::readers::below` 64-bit Bloom filter union, skipping subtrees that never reference those variables via `var(...)`.
- **Two wildcard fallbacks (`ChangedNames::named` in `recalc.rs`):**
  1. Changing **more than 16** custom properties at once (`NAMED_AT_MOST = 16`) sets `wildcard = true` (`readers::ALL`), recascading the whole subtree.
  2. Changing **any `--zgui-*` framework-read property** (`FRAMEWORK_PREFIX = "zgui-"`, such as `--zgui-corner-shape`, `--zgui-ring`, `--zgui-scrollbar-mode`, or `--zgui-shader`) sets `wildcard = true` because native layout/paint reads these directly without CSS `var(...)` declarations. Note that `--zui-*` theme variables do **not** match `FRAMEWORK_PREFIX = "zgui-"` and continue to use targeted reader filtering.
- **Scoping hot `--*` variables:** Do **not** recommend or write `@property { inherits: false }` — `packages/build/src/css.ts` rejects all `@property` rules in `lightningcss-v1`. Instead, declare frequently animated or fast-changing `--*` custom properties on the smallest leaf subtree that reads them.

### Dependency Filtering & Runtime Stylesheets

- **`StyleDependencies` (`vendor/zgui-style/src/deps/mod.rs`):** After flushing stylesheets, `StyleEngine` builds `class_set`, `attr_set`, and per-element `state_mask` (`StyleFilterView`). Mutations to classes, `data-*` attributes, or `ElementState` bits that no active selector references are skipped before touching the style tree. Prefer native `:hover` and `:focus-visible` over mirroring hover/focus into JS `data-*` attributes.
- **Cost of `mountStylesheet` / `mountShellStylesheet`:** Installing, updating, or removing a runtime stylesheet marks `sheets_have_changed()`, which triggers `disable_filters_if_sheets_changed()` for that frame (forcing every DOM mutation in that frame onto the full path) and rebuilds `Stylist` indexes and `StyleDependencies` at the end of the restyle (also clearing `RefsCache` and `SharedStyles`).
- **Static CSS applies to all surfaces automatically:** `Session::install_sheets` in `crates/zgui-host/src/session.rs` automatically installs packaged `components.css` and `app.css` (including all merged `*.module.css`) onto `WindowSurface`, `LayerSurface`, `PopupSurface`, and `SessionLockSurface`. Reserve `mountStylesheet` and `mountShellStylesheet` strictly for CSS generated dynamically at runtime (for example, live user-authored theme rules).

### Frame Damage & Taffy / Parley / Vello Pipeline

Authoritative sources: [`vendor/zgui-style/src/damage/translate.rs`](../../vendor/zgui-style/src/damage/translate.rs), [`vendor/zgui-style/src/damage/layout_damage.rs`](../../vendor/zgui-style/src/damage/layout_damage.rs), and [`vendor/zgui-style/src/damage/bits.rs`](../../vendor/zgui-style/src/damage/bits.rs).

Damage depends on which CSS properties changed, regardless of whether the change came from CSS or inline `style`:

| Property group                                                   | `zgui-style` / engine damage                                                        | Downstream pipeline work                                                                                               |
| ---------------------------------------------------------------- | ----------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `display`, `position`, generated content (`::before`, `::after`) | `Dirty::RELAYOUT \| Dirty::REBUILD_BOX` (`CONSTRUCT_BOX` / `CONSTRUCT_DESCENDANTS`) | Rebuilds layout boxes (`display: none` calls `clear_style_data_below()`, clearing descendant styles)                   |
| `font-*` (shaping properties)                                    | `Dirty::RELAYOUT \| Dirty::RESHAPE \| Dirty::REBREAK`                               | Taffy relayout + Parley text re-shaping (`TextWork::Reshape`) + line breaking                                          |
| `width`, `height`, `padding`, `margin`, `text-align`             | `Dirty::RELAYOUT \| Dirty::REBREAK` (`RELAYOUT_BOX` + `TextWork::Rebreak`)          | Taffy relayout + Parley line re-breaking (~28× cheaper than `Reshape` via `TextKeyStore`)                              |
| `transform`, `translate`, `scale`, `rotate`, `box-shadow`        | `Dirty::REFRAGMENT \| Dirty::REHIT \| Dirty::REPAINT`                               | Skips Taffy layout and Parley text work; updates fragments, hit-testing (`REHIT`), and Vello draw commands (`REPAINT`) |
| `z-index`, `isolation`                                           | `Dirty::RESTACK \| Dirty::REHIT \| Dirty::REPAINT`                                  | Skips Taffy layout and Parley text work; updates stacking order, hit-testing, and Vello draw commands                  |
| `opacity`, `color`, `background-color`                           | `Dirty::REPAINT` (plus `TextPaintUpdate` for text color)                            | Skips Taffy layout and Parley text shaping/breaking; updates Vello paint / brush table slot in place                   |

- **Taffy Flexbox/Grid & `:has(...)`:** Use Taffy Flexbox (`display: flex`) and Grid (`display: grid`) directly in CSS instead of fixed pixel positioning. When using `:has(...)`, scope it tightly with direct-child combinators (`:has(> .badge)`) rather than deep descendant scans.

## 4. Lint Enforcement Reference

`@hewel/zgui` ships two Oxlint presets so styling boundaries are enforced automatically:

- **`@hewel/zgui/oxlint/recommended`** (`packages/ui/oxlint/recommended.json`): enables Solid 2.0 reactivity rules (`eslint-plugin-solid` `0.20.1` via `solid.mjs`), `zgui/no-solid-web-imports`, and `zgui/no-static-component-stylesheet`, which errors on static `mountStylesheet(...)` / `mountShellStylesheet(...)` calls inside components or helper functions so `StyleDependencies` and `Stylist` indexes are not invalidated at runtime.
- **`@hewel/zgui/oxlint/strict`** (`packages/ui/oxlint/strict.json`): extends `recommended` and enables `zgui/prefer-stylesheet`, which flags static inline `style="..."` strings, purely static `style={{ ... }}` objects, static ordinary properties mixed into dynamic `style={{ ... }}` objects, and finite static ternaries on ordinary CSS properties (such as `color: active() ? "var(--a)" : "var(--b)"`) on the 12 native tags (`box`, `row`, `column`, `stack`, `text`, `label`, `scroll`, `control`, `field`, `editor`, `image`, `vector`) and `@hewel/zgui` components. Continuous dynamic values, dynamic templates, unknown spreads, forwarded `props.style`, and runtime `--*` custom properties are allowed.

### Incremental Rollout in Large Apps

Enable `@hewel/zgui/oxlint/strict` globally for new apps, or roll out `zgui/prefer-stylesheet` directory-by-directory in `.oxlintrc.json` using `overrides`:

```json
{
  "extends": ["./node_modules/@hewel/zgui/oxlint/recommended.json"],
  "overrides": [
    {
      "files": ["src/features/settings/**/*.tsx"],
      "rules": {
        "zgui/prefer-stylesheet": "error"
      }
    }
  ]
}
```
