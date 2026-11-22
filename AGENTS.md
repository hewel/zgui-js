# zgui-js Agent Guide

## Environment Gotchas

- **Nix Linux devShell**: Run Rust builds, `test:vendor`, and headless preview tests inside [`shell.nix`](shell.nix) (`nix-shell --run "<cmd>"`) so `pkg-config`, native libraries, and Mesa Vulkan ICDs (`VK_DRIVER_FILES`) match.
- **Single full gate**: `bun run check` runs `fmt:check`, `lint`, `typecheck`, `test:build`, `test:rust`, and `test:vendor`. Run a single vendored crate with `bun run test:vendor <crate>` or a single app typecheck with `bun run typecheck --app <name>`.
- **No concurrent preview suites**: Never overlap multiple `tests/preview/*.test.mjs` runs on `llvmpipe`; CPU contention causes `E_WATCHDOG` false negatives.
- **New workspace packages or apps**: When adding a package under `packages/` or an app under `apps/`, check `package.json` `workspaces`, `.gitignore` build-artifact patterns, `.oxlintrc.json` overrides, and [`docs/development/crate-map.md`](docs/development/crate-map.md) (`typecheck.ts` discovers `packages/` and `apps/` tsconfigs automatically).

## Navigation Pointers

- **SDK & CLI API quick reference** (when writing app code, CLI commands, or adding public exports): read [`llms.txt`](llms.txt) (`tests/build/llms-txt.test.mjs` enforces export and CLI parity).
- **Crate & package layout** (when locating Rust/JS code or checking whether a `zgui-*` crate is vendored vs upstream): read [`docs/development/crate-map.md`](docs/development/crate-map.md) and [`vendor/README.md`](vendor/README.md).
- **Coding standards** (when reviewing or authoring components, overlays, `ui-bridge`, or `vendor/*` patches): read [`CODING_STANDARDS.md`](CODING_STANDARDS.md).
- **Component & primitive map** (when adding or renaming exports in `@hewel/zgui/components`, `@hewel/zgui/primitives`, `apps/examples/gallery` `testId`s, or `generated/*Style.ts`): update [`docs/development/component-map.md`](docs/development/component-map.md) (`tests/build/component-map.test.mjs` enforces parity).
- **Preview verification** (when writing or debugging headless visual tests): read [`docs/development/preview-verification.md`](docs/development/preview-verification.md) and use `preview/profiles/functional.json` for heavy debug apps while keeping `preview/profiles/default.json` for timing gates.
- **Domain vocabulary & ADRs** (when naming public APIs, shell surfaces, or preview concepts): read [`GLOSSARY.md`](GLOSSARY.md) and [`docs/adr/`](docs/adr/) (see [`docs/agents/domain.md`](docs/agents/domain.md)).
- **Issue tracker & triage** (when triaging, opening, or referencing GitHub issues): read [`docs/agents/issue-tracker.md`](docs/agents/issue-tracker.md) and [`docs/agents/triage-labels.md`](docs/agents/triage-labels.md).
