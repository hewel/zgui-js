# Replace StyleX with Lightning CSS and CSS Modules as the Single First-Class Styling Pipeline

**Status:** accepted (supersedes `docs/development/stylex-integration/decision-record.md`)

`zgui-js` previously integrated `@stylexjs/stylex@0.19.2`, `@stylexjs/babel-plugin@0.19.2`, and `styleq@0.2.1` under the `zgui-stylex-v1` profile. Real-world authoring across all 30+ components in `@hewel/zgui/components` and every complex showcase application (`md-editor`, `workspace`, `sysmon`, `motion-lab`, `gallery`) demonstrated zero adoption of StyleX outside its own fixture (`apps/examples/stylex`): authors consistently preferred native CSS with `--zui-*` custom properties and `[data-*]` state selectors for multi-part desktop workbench states, while StyleX imposed a mandatory first-pass Babel transform on every module, a parallel theme system (`stylex.createTheme`), `style-origins.json` provenance bookkeeping, `CLASS_ALIAS_CONFLICT` (`class` vs `className`), and a special-cased `@property --* { syntax: "*"; inherits: false }` exception.

We will remove StyleX entirely and standardize `zgui-js` on **Lightning CSS** (`parcel-bundler/lightningcss`) driving **native CSS (`app.css`) and locally scoped CSS Modules (`*.module.css`)** as the single application styling pipeline.

## Unified Styling Architecture

1. **Single deterministic cascade stack:**
   - `@layer zgui.components` (`components.css`, statically hoisted from `@hewel/zgui/components` per #49)
   - Global authored `app.css` (optional, unlayered)
   - Locally scoped `*.module.css` (unlayered, deterministically ordered across the entire reachable module graph—including ADR-0005 lazy JS chunk edges—and statically merged at build time so Stylo cascade order stays 100% deterministic with zero FOUC)
   - Typed inline `style={{ ... }}` objects for continuous dynamic values only (e.g. split ratios, spring transforms, progress widths)
   - `ThemeProvider` remains the sole owner of SDK `--zui-*` tokens; applications consume `--zui-*` and define their own `--*` variables.
2. **Lightning CSS transform & strict Stylo guardrails:**
   - Admit compile-time `*.module.css` imports in `packages/build/src/validate.ts` while keeping runtime CSS/asset URL bans intact.
   - Run Lightning CSS with **no Browserslist targets** (Stylo is not a browser target; avoid unwanted vendor prefixes or static `oklch`/`color-mix` RGB fallbacks that break dynamic theme variables). Flatten modern CSS nesting (`&`) cleanly during transform where needed.
   - Explicitly **disable `dashedIdents`** (preserving literal `--zui-*` custom properties across nested and portal theme scopes) and **reject `composes` / ICSS `:import`/`:export` / `@value`** in v1 so every exported identifier maps 1:1 to a single static class string with zero runtime merging helper.
   - Replace regex-based `validateStylesheet` checks with Lightning CSS AST/rule validation rejecting unsupported native features (`@import`, `url()`, `image-set()`, `@font-face`, `@container`, `prefers-reduced-motion`, all `@property` rules, and `html[dir]`), while retaining native Rust stylesheet preflight as authoritative.
3. **Reproducible hashing & TypeScript declarations:**
   - Feed stable `packages/build/src/identity.ts` `moduleIdentity` values (never checkout/symlink/Nix-store absolute paths) into CSS Modules class hashing so two checkout roots produce byte-identical CSS, JS, and `buildId`.
   - Emit exact `*.module.css.d.ts` declarations from Lightning CSS's `exports` map before cold `tsrs --noEmit` typechecks, prune stale declarations on file rename/delete, and skip rewriting unchanged `.d.ts` files during watch mode.
   - Verify `lightningcss` loading under pinned Bun inside `shell.nix` first (falling back to `lightningcss-wasm` or a thin Rust host/CLI transform if the `.node` N-API binary has Nix libc friction).
4. **Clean removal of StyleX baggage while retaining native Stylo investments:**
   - Remove `@stylexjs/stylex`, `@stylexjs/babel-plugin`, `styleq`, `packages/build/src/stylex.ts`, `schemas/stylex-profile.json`, `style-origins.json`, `data-style-src`, `className`/`CLASS_ALIAS_CONFLICT` alias handling, and `apps/examples/stylex`.
   - Collapse `packages/build/src/compiler.ts` from two Babel passes down to a single Solid Universal + TypeScript pass.
   - Keep all underlying native investments built during the StyleX effort: native stylesheet preflight, atomic validate-before-commit inline updates, `styleRevision` frame freshness tracking, SDK-private vs application class channel separation, and `#49` static `components.css` hoisting.
   - Replace `apps/examples/stylex` with `apps/examples/css-modules` and corresponding build/preview regression suites, plus a clean `variants()` / `cx()` static class composition pattern documented in `CODING_STANDARDS.md` and `zgui-dev`.

## Considered Options

- **Keep StyleX alongside Lightning CSS Modules indefinitely:** Rejected because maintaining two parallel compilation pipelines, two theme abstractions, dual class aliases (`class`/`className`), and dead StyleX Babel passes for zero active consumers adds permanent maintenance tax.
- **Adopt Tailwind CSS v4 or Panda CSS:** Rejected because both target modern browser feature sets (typed `@property`, preflight HTML resets, `@container`, `prefers-reduced-motion`) that conflict with `zgui-js`'s Stylo subset and existing `@layer zgui.components` + `--zui-*` design system.
- **Adopt Vanilla Extract (`.css.ts`):** Rejected for the primary app styling pipeline because it requires build-time TypeScript module evaluation (`esbuild`/VM) inside our custom non-Vite Bun+Rolldown toolchain, whereas Lightning CSS transforms `.module.css` directly in native Rust with zero eval machinery and full native CSS selector ergonomics.
- **Adopt Devup UI or Linaria (`@wyw-in-js`):** Rejected because both assume React component/JSX transforms and lack first-class Solid 2.0 Universal support.

## Consequences

- `package.json`, `packages/build/package.json`, `schemas/package.schema.json`, and `crates/native-host/src/package.rs` drop `zgui-stylex-v1` / `style-origins.json` in favor of a lean `lightningcss-v1` style manifest identity (breaking change acceptable in v0.1).
- `class` becomes the sole application class attribute on `NativeElementProps` (no `className` alias or `CLASS_ALIAS_CONFLICT`).
- Every application gets locally scoped CSS, zero QuickJS style-runtime overhead, deterministic build-time merged stylesheets compatible with ADR-0005 code-splitting, and strict `tsrs` class-name typechecking out of the box.
