# 03: Deepen bundle compiler, shared Package publication, and watch test seam

**What to build:** Deepen `packages/build/src/compiler.mjs` into one-shot `compileBundle` and persistent `watchBundle` entries (retiring `compilerOptions`, `outputOptions`, and `compilationResult`), extract atomic **Package publication** (`dist` symlink swap and `.zgui-build-status.json` writes) into `packages/build/src/publication.mjs` shared by `build.mjs`, `watch.mjs`, and tests, and add an optional third-argument lifecycle adapter seam to `watchApplication` matching `devApplication`.

**Blocked by:** none

**Status:** ready-for-agent

- [ ] `packages/build/src/compiler.mjs` hides Rolldown plugin assembly, StyleX pass/profile validation, `sourcemapPathTransform`, `delete output[filename]` output suppression, persistent Babel/StyleX transform caching, module pruning, and bundle/compiler handle cleanup behind `compileBundle` and `watchBundle`, retiring `compilerOptions`, `outputOptions`, and `compilationResult`.
- [ ] New `packages/build/src/publication.mjs` owns `BUILD_STATUS_FILE`, `NATIVE_STATUS_FILE`, atomic `publishGeneration`, and bounded atomic `publishBuildStatus` for `packages/build/src/build.mjs`, `packages/build/src/watch.mjs`, and build/preview tests; `build.mjs` preserves existing exports (`buildApplication`, `bundle`, `publishGeneration`, `workspaceRoot`, `publishApplication`, `contentDigest`, `buildInfo`, `dependencyVersions`, `optionalFile`) for external callers.
- [ ] `watchApplication` in `packages/build/src/watch.mjs` accepts an optional third-argument lifecycle adapter object (`{ watchBundle, typecheck, publish, publishStatus }`, matching `devApplication` in `packages/build/src/dev.mjs`) while keeping its two-argument production behavior unchanged.
- [ ] `tests/build/compiler.test.mjs` and `tests/preview/watch.test.mjs` migrate off `compilerOptions`/`outputOptions`/`compilationResult` and hand-rolled `.dist-*` / `.zgui-build-status.json.*.tmp` helpers to use `compiler.mjs` and `publication.mjs`, and new fast deterministic `watchApplication` adapter tests cover `building -> failed -> published` transitions, `previousDirectory` retention, and status-write failures.
- [ ] `pnpm check` (`lint`, `typecheck`, `test:build`, `test:rust`) and `node --test tests/preview/watch.test.mjs` pass after the final edit.
