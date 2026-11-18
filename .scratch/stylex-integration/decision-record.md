# StyleX Integration Decision Record

**Status:** implemented and verified
**Date:** 2026-11-18
**Scope:** SX01–SX19 implemented and verified with SX10/SX17 amendments; SX20 remains explicitly deferred.

**Evidence:** [Verification report](verification.md); [upstream documentation audit](upstream-doc-audit.md); pinned local zgui/native-host/ui-bridge source; `/home/hewel/Data/Downloads/STYLEX_INTEGRATION_SPEC.md`.

## 1. Baseline and scope

- Lock `@stylexjs/stylex@0.19.2`, `@stylexjs/babel-plugin@0.19.2`, and `styleq@0.2.1`; change the audited StyleX baseline from commit `6d7464d` to the `0.19.2` release tag `d7e6174`.
- Keep Solid `1.9.19` and `babel-preset-solid@1.9.19`. Solid is the only accepted frontend; the native props adapter remains renderer-neutral.
- Support native elements and explicitly root-forwarding SDK components. Do not invent a styling target for providers or multi-root composites.
- Run the StyleX pass over every reachable application, workspace, and dependency source module; collect first-pass metadata before tree shaking and transform token modules themselves.

## 2. Compiler and admitted StyleX surface

- Retain the two-pass StyleX → Solid Universal/TypeScript pipeline, composed maps, direct `@stylexjs/stylex` imports, and official `processStylexRules` finalization.
- Keep `runtimeInjection:false`, `test:false`, `styleResolution:"property-specificity"`, `sxPropName:false`, and explicit module resolution. Preview enables source metadata; release may enable it only through an explicit diagnostic option in `configDigest`.
- Keep `useLayers:false`; native acceptance must prove `:root`, escaped `:not(#\#)` chains and their cascade ordering, doubled classes, and `.x.x:root`.
- Include plain `stylex.create`, `stylex.props`, dynamic functions, `defineVars`, `createTheme`, and `keyframes` with native fixtures.
- Exclude `attrs()` as a guaranteed composition path, typed variables/`stylex.types.*`, typed/general `@property`, `defineConsts`, and every feature not admitted by the native profile. Implementation evidence requires one narrow exception: preserve and natively consume the official dynamic-function `@property --* { syntax:"*"; inherits:false }` rules, including replacement/removal and descendant noninheritance; no output rewriting.
- Reject finalizer output containing `html[dir]`/`html:not([dir])` in v1. No selector rewriting or fake HTML ancestor. This narrows the directional portion of SX10; unsupported RTL output is diagnosed under SX11.
- The native-owned, versioned capability profile covers properties/selectors already consumed by the SDK plus admitted StyleX features. Applications may narrow it but cannot override native support. Its digest is matched at activation.

## 3. Props, ownership, and runtime failures

- `class` remains canonical; `className` is the StyleX alias. Normalize empty/whitespace-only values to no contribution before conflict detection.
- If both aliases contribute tokens, emit `CLASS_ALIAS_CONFLICT`, retain the previous application class contribution, and apply unrelated props. Internal classes are not part of this conflict.
- Add an SDK-private ownership channel for internal classes and inline declarations; migrate every root-forwarding component. Do not encode structural classes in the application alias and do not give components different conflict semantics.
- Guarantee override tiers: layered component rules < application classes < application inline declarations < documented native invariants.
- Use one numeric rule regardless of compiler provenance: finite numbers are allowed for custom properties and the explicit unitless-property allowlist; other numeric properties are rejected. String values require native grammar validation, including cases such as `NaNpx`.
- Application style maps are atomic through native parse-before-commit. On normalization or native parse failure, Preview fails; development/release retain the whole previous map, emit structured diagnostics, and apply unrelated props.
- Preserve serialized `style="…"`: it replaces only application-owned declarations, preserves internal ownership, and uses the same validate-before-commit rule. Test string/object transitions and shorthand/longhand overlap.
- `data-style-src` supports update/removal in every profile because explicit release diagnostic builds may emit it.

## 4. Tokens, themes, and motion

- Preserve official plain-token CSS and native tag-agnostic `:root` behavior. Do not require an artificial varGroup class on every consumer.
- `ThemeProvider` exclusively owns SDK `--zui-*` tokens; StyleX themes own application tokens. Explicit application mapping feeds values into `ThemeProvider`; StyleX selectors never target SDK tokens directly.
- Add an explicit host/profile reduced-motion preference that drives shared motion-duration tokens to zero. SDK and admitted StyleX animations use those tokens; no reduced-motion media-query emulation.
- Reactive SDK stylesheet updates are preflighted atomically. On failure, Preview fails; development/release retain the previous sheet/theme state and diagnostics. Accepted install/replace/remove of any application, StyleX, SDK component, or SDK theme sheet advances `styleRevision`.

## 5. Package identity and stylesheet lifecycle

- Move builder and Rust loader together to package schema v2; keep the JS mount `hostApiVersion` unchanged unless its contract changes.
- Preserve handwritten `src/app.css` as an ordered base sheet (for example `base.css`) and publish generated StyleX `app.css` after it. Validate and digest both.
- `buildId` covers the complete declared artifact set and canonical metadata, including build info and provenance, with self-hash cycles excluded. Remove machine-specific temporary/absolute paths from reproducible identities.
- Release defaults to generated locations plus verified module-level origins; exact original positions remain absent unless verified. Preview/full diagnostic builds may add per-prop source metadata.
- Keep publication separate from native acceptance: `published` identifies immutable candidate bytes; `accepted`/`rejected` records preflight/mount outcome and build/profile identity. The last successful application is not merely the latest publication.
- Use stable ordered stylesheet slots and successor-safe teardown. Native candidate preflight must prevent partial replacement; current upstream replacement otherwise installs the parseable subset.
- Make minimal, tested vendored-zgui patches for structured diagnostics/preflight, validate-before-commit inline updates, and frame/style revision exposure. Do not build a parallel host-side CSS engine.

## 6. Preview and reload

- Full application reload only in v1. CSS-only activation and SX20 are explicitly deferred.
- Add consumed `styleRevision` to frame evidence; retain the existing document/metadata/resource revision tuple instead of adding a duplicate `stateRevision`. Requested revisions appear in failure/timeout diagnostics.
- Frame freshness covers stylesheet-only SDK theme changes, class/inline changes, resources, configuration, presentation, and undamaged frame reuse. Reused pixels retain their original presentation/style provenance while receiving a new validation-frame identity.

## 7. Assets and SX17

- Rewrite SX17 around native package assets: declared `<image src>` resources with existing readiness evidence, plus schema-v2 manifest font-family/resource mappings registered before mount.
- Reject CSS `url()`, `@import`, `image-set`, and `@font-face` in v1. Native URL painting and font-face consumption do not exist.
- Unregistered/misspelled font families must produce diagnostics rather than silent fallback. Capture must wait for required image/font readiness.

## 8. Acceptance amendments and resolved unknowns

- SX01–SX19 use these amendments: SX10 excludes RTL success and tests its diagnostic under SX11; SX17 tests native package images/fonts rather than CSS URLs; SX20 is deferred.
- Explicit fixtures cover keyframes/reduced motion, empty aliases, native parse-before-commit, string/object transitions, dynamic SDK sheet rejection, publication versus acceptance, and all-sheet `styleRevision` barriers.
- Escaped-ID chains, doubled-class matching, and priority cascade ordering are covered by direct native evidence and Preview scenarios.
- Native stylesheet preflight, atomic inline-map transactions, deferred admission, registration cleanup, and revision-aware style evidence are implemented.
- SX01–SX19 passed the acceptance evidence in [verification.md](verification.md); SX20 is deferred, not passed.
