# StyleX Integration Verification

**Status:** implemented and verified
**Date:** 2026-11-18
**Decision baseline:** [decision-record.md](decision-record.md)
**Upstream evidence:** [upstream-doc-audit.md](upstream-doc-audit.md)

## Locked integration

- StyleX compiler/runtime: `@stylexjs/babel-plugin` and `@stylexjs/stylex` 0.19.2; `styleq` 0.2.1.
- Solid and `babel-preset-solid`: 1.9.19. Node.js 26.13.0; pnpm 12.18.0; TypeScript 7.0.2.
- Native profile: `zgui-stylex-v1`, canonical descriptor in `schemas/stylex-profile.json` with distinct compiler/native-profile digests.
- Package schema v2: canonical manifest/build/artifact identities, ordered `base.css` → `app.css`, module-relative provenance, and full `base.css` → `app.css` integrity checks.

## Required gates

| Check | Observed result |
| --- | --- |
| `pnpm check` | Passed: oxlint, all 15 typecheck projects, 230 build tests, Rust workspace tests, and Clippy with warnings denied. |
| `pnpm test:vendor` | Passed: required vendored runtime/style/dom/scroll/paint/view suites, including StyleX selectors/preflight, dynamic registrations, revisions, and inline transactions. |
| `pnpm test:preview` | **84/84 scenarios passed** with real Vulkan/wgpu frames. Includes all six StyleX scenarios and Gallery fuzz with the explicit 30 ms runtime entry profile. |
| `cargo test -p native-host valid_mount` | Passed ordered valid mount and rejected mount preservation, including no rejected command reaching the host. |
| `cargo test -p ui-bridge --test stylesheets` | 6/6 passed, including removal, cascade position, deferred activation, and removal-borrow lifetime. |
| `cargo doc --workspace --no-deps --offline` | Passed. |
| Independent Standards and Spec reviews | Both report no remaining findings after fixes and targeted re-review. |

The Preview environment reports AMD Radeon RX 7900 XTX (RADV NAVI31), Vulkan, and Mesa 26.2.6. Results are not cross-GPU identity claims. Nested Disclosure has an explicit 300,000-byte stack bound; Gallery fuzz has a 240 KiB stack cap and a 30 ms entry bound. Both are asserted in `metrics.json`.

## SX acceptance matrix

| Case | Evidence and result |
| --- | --- |
| SX01 | **Pass.** Real TSX compiles without an injector and produces native pixels from extracted CSS. |
| SX02 | **Pass.** Imported token modules and multiple reachable/dependency style sources survive before tree shaking. |
| SX03 | **Pass.** Watch compilation replaces changed rules and prunes deleted/unreachable modules. |
| SX04 | **Pass.** Conditional StyleX classes toggle on/off while SDK structural classes survive. |
| SX05 | **Pass.** Empty aliases contribute nothing; real conflicts diagnose and Preview fails while previous application classes remain. Turn-end switching has no transient conflict. |
| SX06 | **Pass.** Dynamic zero, omission, valid values, and invalid values—including `NaNpx`—retain/reject atomically as appropriate. |
| SX07 | **Pass.** Layered SDK defaults, application overrides, removal, and documented invariant restoration behave correctly. |
| SX08 | **Pass.** Property-specificity, official priority selectors, shorthand/longhand ownership, and native cascade are preserved. |
| SX09 | **Pass.** Default, nested, and removed themes inherit/reset correctly through native `:root` and doubled theme selectors. |
| SX10 | **Pass with amendment.** Focus, hover, and viewport media affect native results. Browser-root RTL output is an SX11 diagnostic, not an SX10 success. |
| SX11 | **Pass.** Unsupported RTL wrappers, typed/general `@property`, container/reduced-motion media, URL resources, and invalid declarations produce located diagnostics. Handwritten `.html` classes remain valid. |
| SX12 | **Pass.** Functional release CSS and explicit diagnostic builds are supported; source metadata updates and clears. |
| SX13 | **Pass.** Runtime injection/test output and untransformed compile-time calls are rejected by graph/engine validation. |
| SX14 | **Pass.** Mixed JS/CSS/provenance/build identities and native-profile mismatches fail before activation. |
| SX15 | **Pass.** Consumed `styleRevision` and the existing document/metadata/resource revision tuple gate frames, including sheet-only SDK theme changes and undamaged reuse. |
| SX16 | **Pass.** Ordered stable slots, replacement/reload failure preservation, deferred mount admission, and successor-safe teardown hold. Rejected commands are never queued. |
| SX17 | **Pass with amendment.** Declared `<image src>` assets and manifest-registered fonts are ready before capture; scoped leases are successor-safe. CSS URLs and `@font-face` are rejected. Configuration and color-scheme font failures diagnose correctly. |
| SX18 | **Pass.** Generated rule ranges and verified module origins remain available when original positions are absent. |
| SX19 | **Pass.** Native actions target stable test IDs rather than generated hashes. |
| SX20 | **Deferred by contract.** CSS-only activation is not implemented; full application reload is the only v1 path. |

## Implementation-time corrections proven during integration

- Official dynamic functions emit universal `@property { syntax:"*"; inherits:false }`. These exact registrations use Stylo’s registration machinery, including noninheritance and replacement/removal cleanup; typed/other forms remain rejected.
- SDK preflight runs after the JS mount turn and in the native command phase, not inside the 25 ms JS entry budget. Deferred candidates install only after admission and in first-install order.
- Handwritten `src/app.css` remains the base sheet. CSS `url()`, `image-set`, `@import`, and `@font-face` are excluded; package images use native readiness and fonts use scoped registration.
- `className` aliases reconcile once per outer bridge turn, retaining previous application classes on conflict and avoiding transient alias conflicts.
- Font validation includes document, stylesheet, surface-configuration, and color-scheme identities so computed family changes cannot silently fall back.
- `.zgui-native-status.json` and its temporary siblings are excluded from rebuild triggers. Symlink and resolved-generation loads share the same acceptance destination.
- `cargo fmt --all -- --check` reports repo-wide formatting drift, including pre-existing code. No bulk reformat was performed; required lint, typecheck, tests, Clippy, and docs gates passed instead.
