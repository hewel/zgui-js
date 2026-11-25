# npm Release & Trusted Publishing Guide

This document defines the release process, initial bootstrap procedure, runtime baseline, and
failure-recovery protocol for publishing `zgui-js` packages to public **npmjs.org**.

## 1. Published Package Matrix (Topological Order)

All 6 public packages publish in lockstep at the exact same SemVer version as root `package.json`
and `Cargo.toml` (`[workspace.package].version`):

1. `@hewel/zgui` (`packages/ui`) — compiled `.js` + `.d.ts` native UI SDK, `#solid` zero-DOM
   isolation, Lightning CSS Modules `tsconfig.native.json`, and `oxlint` preset.
2. `@hewel/zgui-services` (`packages/services`) — compiled `.js` + `.d.ts` host service APIs.
3. `@hewel/zgui-build` (`packages/build`) — compiler, `tsrs` runner, preview fixtures, smoke
   fixture, and app packager.
4. `@hewel/zgui-host-linux-x64` (`packages/host-linux-x64`) — prebuilt Linux `x86_64` `glibc`
   `zgui-host` binary (`"os": ["linux"]`, `"cpu": ["x64"]`, `"libc": ["glibc"]`).
5. `@hewel/zgui-cli` (`packages/cli`) — `zgui-js` CLI with prebuilt read-only `inspector-bundle/`
   and `optionalDependencies: { "@hewel/zgui-host-linux-x64": "<version>" }`.
6. `create-zgui-js` (`packages/create-zgui-js`) — unscoped project scaffolder (`bun create zgui-js`).

`@hewel/zgui-host-linux-arm64` (`packages/host-linux-arm64`) remains `"private": true` until
Linux arm64 release binaries are built and verified.

## 2. Linux Runtime Baseline

- **Build runner:** `ubuntu-24.04` (`x86_64`, GNU `glibc >= 2.39`).
- **Package filter:** `@hewel/zgui-host-linux-x64` sets `"libc": ["glibc"]` so musl Linux systems
  do not install an incompatible binary.
- **Dynamic library audit:** `release.yml` runs `ldd target/release/zgui-host` and checks linked
  `GLIBC_*` symbol versions before staging and packing.

## 3. Maintainer Pre-Release Verification (One-Time Setup)

Before the first public release:

1. **Verify npm namespace & package names:**
   - Confirm you control the `@hewel` scope on `https://www.npmjs.com`.
   - Confirm unscoped `create-zgui-js` is available or already owned by your npm account/org.
2. **Configure GitHub Environments** in `hewel/zgui-js` repository settings:
   - Create environment `npm-release` restricted to deployment tags matching `v*`.
   - Create environment `npm-bootstrap` restricted to deployment tags matching `v*` (required
     reviewers recommended) for one-time initial package creation only.

## 4. First-Time Bootstrap & OIDC Trusted Publisher Binding

npm requires a package to exist before Trusted Publishing (OIDC) can be configured in the npm UI.

1. Generate a temporary, short-lived npm **Automation** token with publish access to `@hewel` and
   `create-zgui-js`, and store it as `NPM_TOKEN` **only** inside the protected `npm-bootstrap`
   GitHub Environment (never as a repository-wide secret).
2. Cut and push the initial release tag `vX.Y.Z` (see Section 5). Wait for `verify-rust-and-vendor`,
   `build-and-verify-linux-x64`, and `verify-installed-release` to succeed, or dispatch `release.yml`
   manually on tag `vX.Y.Z` with `mode=bootstrap`.
3. Immediately after all 6 packages appear on npmjs.org, open **Settings -> Publishing access ->
   Trusted Publisher** for **each** of the 6 packages and add GitHub Actions:
   - **Organization / user:** `hewel`
   - **Repository:** `zgui-js`
   - **Workflow filename:** `release.yml`
   - **Environment name:** `npm-release`
4. On each package's npm settings page, enable **"Require two-factor authentication and disallow
   tokens"** (or restrict publishing exclusively to Trusted Publishing), then **revoke** the
   temporary `NPM_TOKEN` and delete it from the `npm-bootstrap` environment.

From that point onward, standard `v*` tag pushes run `publish-oidc` using `id-token: write` and
`npm publish --provenance` with zero static npm secrets. `publish-oidc` never reads `NPM_TOKEN`
and never downgrades from OIDC provenance.

## 5. Cutting a Release

```sh
# 1. Bump all 7 workspace packages, root package.json, Cargo.toml, and Cargo.lock in lockstep
bun run release:version -- --set 0.2.0

# 2. Refresh bun.lock and rebuild SDK dist
bun install
bun run build:packages

# 3. Run read-only release preflight and full repository gate
bun run release:check -- --tag v0.2.0
bun run check

# 4. Commit and tag
git commit -am "chore(release): v0.2.0"
git tag v0.2.0
git push origin main v0.2.0
```

- Pushing tag `vX.Y.Z` triggers `.github/workflows/release.yml`:
  1. `verify-rust-and-vendor` (parallel): runs workspace Rust tests, Clippy, and all vendored crate
     test suites on `ubuntu-24.04`.
  2. `build-and-verify-linux-x64` (parallel): runs `release.ts check --tag`, JS/TS formatting,
     lint, typecheck, and build tests, builds release `zgui-host`, audits `ldd`, packs and audits
     all 6 tarballs, and uploads `release-artifacts`.
  3. `verify-installed-release`: clean `ubuntu-24.04` runner without Rust/Cargo that serves the
     exact uploaded tarballs over a local HTTP registry, installs via `bun install --linker isolated`
     (with no direct host dependency), and verifies `create-zgui-js`, `doctor`, `build` (including
     static `components.css` hoisting), `package --bytecode` (including `licenses/`), headless
     Vulkan `preview`, read-only `smoke` & `inspect --ui`, `oxlint`, `tsrs`, and pinned `tsc`.
  4. `publish-oidc`: gated on all three verification jobs, publishes the exact verified tarballs
     from `release-artifacts` using OIDC `--provenance`.
- Prerelease tags (e.g. `v0.2.0-rc.1`) automatically publish under their prerelease dist-tag
  (`rc`, `beta`, `alpha`, or `next`) and are explicitly forbidden from publishing to `latest`.
- Running `workflow_dispatch` on a non-tag branch only permits `mode=dry-run` with `contents: read`
  and never receives publish credentials.

## 6. Partial-Failure Idempotent Resume

If publishing fails midway (e.g. after 3 of 6 packages were uploaded due to transient registry
errors):

1. **Never rebuild fresh tarballs** for the same version — always reuse the exact `release-artifacts`
   bundle (including `release-manifest.json`) produced by the failed workflow run.
2. Re-run the `publish-oidc` job on that workflow run (or run
   `bun tools/release.ts publish --manifest release-artifacts/release-manifest.json --provenance`).
3. `tools/release.ts publish` queries each package version's `dist.integrity` (`sha512-...`) on the
   registry:
   - **Exact SRI match:** logs `SKIP` and proceeds to the next package in topological order.
   - **Mismatch or missing remote integrity:** aborts immediately (**fail-closed**). Never attempt
     to overwrite an existing version with different bytes; bump to a new patch version instead.

## 7. Licensing & Redistributed Notices

Every published tarball ships `LICENSE`, `NOTICE`, and `THIRD_PARTY_NOTICES.md` (covering
Apache-2.0, upstream `zortax/zgui` attributions, file-scoped MPL-2.0 source availability for
`vendor/zgui-style/src/driver/recalc.rs`, and SIL OFL 1.1 preview fonts). `zgui-js package`
copies all three notice files into `<out>/licenses/` and records them in `package-manifest.json`.
