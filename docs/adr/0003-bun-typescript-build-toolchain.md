# Unify Host Build Tooling, Package Management, and Tests on Bun with Native TypeScript While Keeping Rolldown

**Status:** accepted

`packages/build` previously ran 16 untyped JavaScript modules (`1,707` lines) on Node.js alongside `node --test` and `pnpm` (which itself requires Node.js on this host), while the heavy lifting in the toolchain (`rolldown`, `tsrs`, `oxlint`, `oxfmt`, and `native-host`) is already implemented in native Rust. Rather than introducing a Rust or Gleam build orchestrator over a cross-process IPC boundary—or keeping Node.js around solely to run `pnpm`—we will unify host runtime, test execution, and workspace package management onto **Bun** (`>=1.4.4`) alongside Cargo, and rewrite `packages/build/src/` in idiomatic strict TypeScript (`.ts`) checked by `tsrs`, `oxlint` (`typeAware: true`), and `oxfmt`.

Migration will adopt Bun-native primitives where they simplify code cleanly (`Bun.spawn`/`Bun.spawnSync`, `Bun.file`/`Bun.write`, `Bun.CryptoHasher`, `bun test`, `bun install` with `linker = "isolated"` and `bun.lock`, and `bunfig.toml`) rather than transliterating Node wrappers verbatim, while strictly preserving POSIX process-group cleanup for `native-host`/Cargo, atomic `.zgui-generations` / `dist` symlink **Package publication**, canonical SHA-256 digests, and **Native acceptance** compatibility. We deliberately keep **`rolldown` + Babel** (`@stylexjs/babel-plugin`, `@solidjs/babel-plugin`, `@babel/preset-typescript`) as the compilation pipeline.

## Considered Options

- **Inverting build orchestration to Rust (`crates/zgui-build` + passive Node stdio NDJSON daemon):** Rejected because Babel's Solid 2 and StyleX transforms must remain in a JavaScript runtime; splitting orchestration into Rust introduces a new cross-process IPC protocol, dual-mode test/daemon complexity for `tests/build/native-fixture.mjs`, Cargo lock contention, and new `oxc_parser`/`notify` crates for marginal throughput gain over an already-Rust bundler and type checker.
- **Rewriting `packages/build` in Gleam (JavaScript target):** Rejected because ~80% of the build package interacts with Babel AST `NodePath` objects, Rolldown plugin hooks, and OS process/symlink calls, requiring extensive `@external(javascript, ...)` FFI glue and a third language toolchain.
- **Keeping `pnpm` alongside Bun:** Rejected because `bun 1.4.4` natively supports isolated `node_modules` linking (`linker = "isolated"`), release-age supply-chain gates (`minimumReleaseAge` / `minimumReleaseAgeExcludes`), `workspace:*` protocols, and frozen text lockfiles (`bun.lock`), whereas retaining `pnpm` keeps a residual Node.js dependency and duplicate workspace configuration.
- **Replacing `rolldown` with `Bun.build`:** Rejected because `compiler.ts` relies on two-pass Babel source-map composition and Rolldown plugin lifecycle hooks (`getModuleIds`, `generateBundle`, `addWatchFile`) to guarantee **StyleX profile** provenance (`style-origins.json`).

## Consequences

- The repository consolidates to a pure two-tool developer stack (`bun` + `cargo`), replacing `pnpm-workspace.yaml` and `pnpm-lock.yaml` with `package.json` `workspaces`, `bunfig.toml` (`linker = "isolated"`, release-age exclusions, test timeout, and Solid 2.0 `reportError()` test preload), and `bun.lock`.
- `packages/build` moves from untyped `.mjs` to strict `.ts` (`packages/build/tsconfig.json` wired into `bun run typecheck`), enabling full `tsrs` + `oxlint` (`typeAware`) + `oxfmt` enforcement across the build layer, removing JS-only lint relaxations on `packages/build/**`, and eliminating pnpm-specific sibling-path assumptions when resolving transitive package versions.
- Workspace scripts, `tests/build`, `tests/preview`, and Rust integration tests invoke `bun` / `bun test`; `build-info.json` records Bun runtime and `bun.lock` provenance while the packaged application runtime target remains QuickJS-NG.
- `preview-check` has zero JavaScript compilation dependencies and remains cleanly separable if we later choose to fold it into `native-host preview check` without touching the rest of `packages/build`.
