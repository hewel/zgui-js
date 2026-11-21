# Replace Babel with Solid's Native Oxc Compiler (`@solidjs/compiler`) and Rolldown TypeScript Stripping

**Status:** accepted (supersedes the Babel-retention clause of ADR-0003 and the residual single Babel pass in ADR-0006)

ADR-0003 unified `packages/build` on Bun + Rolldown while keeping Babel only for `@stylexjs/babel-plugin`, `@solidjs/babel-plugin`, and `@babel/preset-typescript`. Once ADR-0006 (#78) replaces StyleX with Lightning CSS Modules, `@solidjs/babel-plugin` becomes the sole remaining reason `@babel/core`, `@babel/parser`, `@babel/traverse`, `@babel/preset-typescript`, and `@babel/plugin-syntax-jsx` exist in `zgui-js`.

Solid 2.0 ships `@solidjs/compiler` (`packages/compiler` in `solidjs/solid`), an official Rust/Oxc native JSX compiler supporting `generate: "universal"`, `moduleName`, `dev`, `sourceMap: true`, prebuilt NAPI-RS binaries across Linux/macOS/Windows, and an automatic `@solidjs/compiler-wasm32-wasi` fallback—delivering 23x–355x faster JSX transforms than `@solidjs/babel-plugin`. Meanwhile, our pinned `rolldown@1.3.2` already embeds Oxc for both native TypeScript stripping (`moduleType: "ts"`) and AST parsing (`parseSync` from `rolldown/utils`).

We will migrate `packages/build` completely off Babel onto `@solidjs/compiler` + Rolldown Oxc with **no dual-compiler Babel fallback** in production.

## Target Compiler Architecture

1. **JSX lowering (`@solidjs/compiler`):**
   - Pin `@solidjs/compiler` to the exact Solid runtime version (`2.0.1`, upgrading in lockstep with Solid).
   - Run `transform(source, { filename, generate: "universal", moduleName: "@hewel/zgui/renderer", dev: development, sourceMap: true })` on JSX/TSX modules only; plain non-JSX `.ts`/`.js` modules bypass JSX compilation while still passing module validation.
   - Pass the emitted `code` and JSON-string `map` into Rolldown with `moduleType: "ts"` for TypeScript sources and `"js"` for JavaScript sources, keeping the ES2022 output target.
2. **TypeScript stripping (`Rolldown`):**
   - Explicitly configure `transform.typescript.onlyRemoveTypeImports: true` in Rolldown so non-type imports used only in type positions preserve side-effect semantics identically to `@babel/preset-typescript`.
   - Keep `packages/build/src/typecheck.ts` unchanged (`tsrs --noEmit` remains the authoritative type checker).
3. **Zero-Babel AST validation and static sheet extraction:**
   - Replace `@babel/parser` + `@babel/traverse` in `packages/build/src/validate.ts` and `extractStaticComponentSheet` (`packages/build/src/compiler.ts`) using `parseSync` from `rolldown/utils` (reusing Rolldown's embedded Oxc parser with zero extra parser binary).
   - Keep **both** validation stages: (a) TS-aware pre-tree-shaking module validation distinguishing erased type imports from forbidden runtime imports, `require`, and top-level await; and (b) final-chunk JS validation with full ESTree scope analysis (free globals, `typeof` guards, lexical `arguments`, shadowed bindings, and alias/write tracking for `hostApiVersion` and synchronous `mount`).
4. **Provenance & dependency cleanup:**
   - Remove `@solidjs/babel-plugin`, `@babel/core`, `@babel/parser`, `@babel/traverse`, `@babel/preset-typescript`, `@types/babel__core`, `@types/babel__traverse`, transitive `@babel/plugin-syntax-jsx` tracking, `packages/build/src/babel-plugins.d.ts`, and obsolete `bunfig.toml` exceptions.
   - Record `@solidjs/compiler` in `build-info.json` via `packageVersion()` (not subpath `./package.json`, which `@solidjs/compiler` does not export).

## Release Gates Required Before Deleting Babel

1. **Nix/Bun dual-backend verification:** Inside `shell.nix` with pinned Bun `1.4.4` and frozen isolated install, verify both the native Linux GNU `.node` addon and forced WASI (`NAPI_RS_FORCE_WASI=error`, `SOLID_COMPILER_NATIVE` unset) produce identical code and source maps.
2. **QuickJS end-to-end source-map symbolication:** Throw inside a compiled JSX getter, run the packaged bundle in QuickJS via `native-host`, and verify `crates/native-host/src/package.rs::remap_diagnostic` recovers the exact authored `.tsx` path, line, and column (plus two-checkout-root byte reproducibility).
3. **Universal renderer parity in QuickJS:** Verify `spread` proxy getter descriptors, callback/signal `ref` cleanup, forwarded refs, lazy children, `<For<T>>` generic JSX, `as` assertions, unbound control-flow built-ins, and `dev`/`prod` modes across `tests/build/` and `tests/preview/`.
4. **Import-elision parity:** Verify explicit `import type` is erased while a non-type import referenced only in a type position still executes its module side effect.

## Considered Options

- **Keep `@solidjs/babel-plugin` as a runtime/machine fallback alongside `@solidjs/compiler`:** Rejected because `@solidjs/compiler` already bundles a deterministic WASI fallback; shipping two independent JSX compilers creates silent machine-dependent divergence and doubles maintenance/test costs.
- **Add a standalone `oxc-parser` npm dependency for `validate.ts`:** Rejected because pinned `rolldown@1.3.2` already exports `parseSync` from `rolldown/utils` backed by the same Oxc parser.
- **Rely on Oxlint alone and drop `validate.ts` pre-tree-shaking checks:** Rejected because `validate.ts` is a mandatory package-publication security boundary enforcing forbidden runtime APIs, `hostApiVersion`, and synchronous `mount` contracts even when callers skip linting.

## Consequences

- `zgui-js`'s entire frontend/host build stack becomes 100% native-compiled (`Bun` + `Rolldown/Oxc` + `@solidjs/compiler` + `Lightning CSS` + `tsrs` + `Oxlint` + `Oxfmt` + `Cargo/native-host`), with zero Babel or Node.js dependencies.
- `tests/build/compiler.test.mjs` migrates transform-injection, missing-map, and watch-recovery tests off `babelTransform` onto `@solidjs/compiler` hooks and updates `build-info.json` provenance assertions.
