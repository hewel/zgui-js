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

> **Status:** Initial bootstrap for the 6 packages above completed with `v0.1.0`. Routine releases
> follow Section 5 directly via `publish-oidc`. Retain this procedure when onboarding any future
> brand-new public package (such as `@hewel/zgui-host-linux-arm64`).

npm requires a package to exist before Trusted Publishing (OIDC) can be configured in the npm UI.

1. Generate a temporary, short-lived npm **Granular Access Token** (legacy classic Automation tokens
   are phased out) and store it as `NPM_TOKEN` **only** inside the protected `npm-bootstrap` GitHub
   Environment (never as a repository-wide secret):
   - **Packages and scopes:** select **All packages** (brand-new packages do not appear in the
     existing-package picker) with **`Read and write (publish and stage)`** — selecting
     `Read and write (stage only)` causes direct `npm publish` to fail with `E_STAGE_REQUIRED`.
   - **Two-factor authentication:** check **"Bypass two-factor authentication (2FA)"** at creation
     time (this flag cannot be toggled afterward; omitting it fails CI publishing with `EOTP`).
2. Cut and push the initial release tag `vX.Y.Z` (see Section 5), then dispatch `release.yml`
   manually on tag `vX.Y.Z` with `mode=bootstrap`:
   ```sh
   gh workflow run release.yml --ref vX.Y.Z -f mode=bootstrap
   ```
   - Because `.github/workflows/release.yml` serializes runs per ref (`concurrency: group: release-${{ github.ref }}`, `cancel-in-progress: false`), the automatic `push` run triggered by pushing `vX.Y.Z` (which will fail at `publish-oidc` before OIDC is bound) holds the concurrency lock. Cancel that automatic `push` run (`gh run cancel <push-run-id>`) so the `mode=bootstrap` run starts immediately.
   - Wait for `verify-rust-and-vendor`, `build-and-verify-linux-x64`, `verify-installed-release`, and `publish-bootstrap` to succeed.
3. Immediately after all new packages appear on npmjs.org, open **Settings -> Publishing access ->
   Trusted Publisher** for **each** package and add GitHub Actions:
   - **Organization / user:** `hewel`
   - **Repository:** `zgui-js`
   - **Workflow filename:** `release.yml`
   - **Environment name:** `npm-release`
4. On each package's npm settings page, enable **"Require two-factor authentication and disallow
   tokens"** (or restrict publishing exclusively to Trusted Publishing), then **revoke** the
   temporary Granular Access Token on npmjs.org and delete `NPM_TOKEN` from the `npm-bootstrap`
   environment (`gh secret delete NPM_TOKEN --env npm-bootstrap`).

From that point onward, standard `v*` tag pushes run `publish-oidc` using `id-token: write` and
`npm publish --provenance` with zero static npm secrets. `publish-oidc` never reads `NPM_TOKEN`
and never downgrades from OIDC provenance.

## 5. Cutting a Release

Optional pre-release cloud rehearsal on `main` (runs the full Ubuntu 24.04 build, Rust/vendor
tests, isolated install verification, and `--dry-run` publish with `contents: read` and zero
publish credentials):

```sh
gh workflow run release.yml --ref main -f mode=dry-run
```

Standard release cut:

```sh
# 1. Bump all 7 workspace packages, root package.json, Cargo.toml, and Cargo.lock in lockstep
bun run release:version -- --set 0.2.0

# 2. Refresh bun.lock and rebuild SDK dist
bun install
bun run build:packages

# 3. Run read-only release preflight and full repository gate (inside shell.nix on Nix Linux hosts)
bun run release:check -- --tag v0.2.0
bun run check

# 4. Update CHANGELOG.md, commit, and tag
git commit -am "chore(release): v0.2.0"
git tag v0.2.0
git push origin main v0.2.0
```

- Pushing tag `vX.Y.Z` triggers `.github/workflows/release.yml`:
  1. `verify-rust-and-vendor` (parallel, 60m timeout): runs workspace Rust tests, Clippy, and all
     vendored crate test suites on `ubuntu-24.04`.
  2. `build-and-verify-linux-x64` (parallel, 60m timeout): runs `release.ts check --tag`, JS/TS
     formatting, lint, typecheck, and build tests, builds release `zgui-host`, audits `ldd`, packs
     and audits all 6 tarballs, and uploads `release-artifacts`.
  3. `verify-installed-release`: clean `ubuntu-24.04` runner without Rust/Cargo that runs
     `tools/verify-release-artifacts.ts` over an in-process local HTTP registry serving the exact
     uploaded tarballs, installs via `bun install --linker isolated` (with no direct host
     dependency), and verifies `create-zgui-js`, `doctor`, `build` (including static
     `components.css` hoisting), `package --bytecode` (including `licenses/`), headless Lavapipe
     Vulkan `preview` (`status === "passed"`, `teardown.clean === true`), read-only `smoke` &
     `inspect --ui`, `oxlint`, `tsrs`, and pinned `tsc`.
  4. `publish-oidc`: gated on all three verification jobs, publishes the exact verified tarballs
     from `release-artifacts` using OIDC `--provenance`.
- Prerelease tags (e.g. `v0.2.0-rc.1`) automatically publish under their prerelease dist-tag
  (`rc`, `beta`, `alpha`, or `next`) and are explicitly forbidden from publishing to `latest`.
- Running `workflow_dispatch` on a non-tag branch only permits `mode=dry-run` with `contents: read`
  and never receives publish credentials.

## 6. Partial-Failure Idempotent Resume

If publishing fails midway (e.g. due to transient registry errors or credential/permission fixes):

1. **Never rebuild fresh tarballs** for the same version — always reuse the exact `release-artifacts`
   bundle (including `release-manifest.json`) produced by the failed workflow run.
2. Re-run only the failed publish job on that workflow run — via **Re-run failed jobs** in the
   GitHub Actions UI or:
   ```sh
   gh run rerun <run-id> --failed
   ```
   (Or locally against downloaded artifacts:
   `bun tools/release.ts publish --manifest release-artifacts/release-manifest.json --provenance`.)
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
