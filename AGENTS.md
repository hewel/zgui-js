# zgui-js Agent Guide

## Environment Gotchas

- **Nix Linux devShell**: Run Rust builds, `test:vendor`, and headless preview tests inside [`shell.nix`](shell.nix) (`nix-shell --run "<cmd>"`) so `pkg-config`, native libraries, and Mesa Vulkan ICDs (`VK_DRIVER_FILES`) match.
- **Vendored Rust tests**: `vendor/*` crates are excluded from the root Cargo workspace; run `bun run test:vendor` (or `bun run test:vendor <crate>` for one crate).

## Navigation Pointers

- **Crate & package layout** (when locating Rust/JS code or checking whether a `zgui-*` crate is vendored vs upstream): read [`docs/development/crate-map.md`](docs/development/crate-map.md) and [`vendor/README.md`](vendor/README.md).
- **Coding standards** (when authoring or reviewing components, overlays, `ui-bridge`, or `vendor/*` patches): read [`CODING_STANDARDS.md`](CODING_STANDARDS.md).
- **Component map** (when adding or renaming exports in `@hewel/zgui/components`, `apps/examples/gallery` `testId`s, or `generated/*Style.ts`): update [`docs/development/component-map.md`](docs/development/component-map.md) (`tests/build/component-map.test.mjs` enforces parity).
- **Preview verification** (when writing or debugging headless visual tests): read [`docs/development/preview-verification.md`](docs/development/preview-verification.md).
- **Domain vocabulary & ADRs** (when naming public APIs, shell surfaces, or preview concepts): read [`GLOSSARY.md`](GLOSSARY.md) and [`docs/adr/`](docs/adr/) (see [`docs/agents/domain.md`](docs/agents/domain.md)).
- **Issue tracker & triage** (when triaging, opening, or referencing GitHub issues): read [`docs/agents/issue-tracker.md`](docs/agents/issue-tracker.md) and [`docs/agents/triage-labels.md`](docs/agents/triage-labels.md).
