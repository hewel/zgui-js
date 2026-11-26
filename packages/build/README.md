# @hewel/zgui-build

Build, watch, develop, and preview native zgui applications from Bun (≥ 1.4.4). Import from `@hewel/zgui-build`; importing it does not start a compiler or host, install signal handlers, or run the CLI.

## Build and watch

```ts
import {
  buildApplication,
  resolveApplication,
  syncCssModuleTypes,
  watchApplication,
} from "@hewel/zgui-build";

const app = await resolveApplication("./my-app");
const options = {
  ...app,
  extraCapabilities: { namedSockets: ["status"] },
  capabilitiesFile: "./generated/plugin-capabilities.json",
  cssModuleInclude: ["../../packages/sdk/src", "../../plugins/*/src"],
};

await syncCssModuleTypes({ project: "./my-app/tsconfig.json", include: options.cssModuleInclude });
const { dist, manifest } = await buildApplication(app.app, options);
console.log(dist, manifest.id, manifest.buildId);

const watcher = watchApplication(app.app, {
  ...options,
  onSuccess: ({ manifest }) => console.log(manifest.buildId),
  onError: error => console.error(error),
});
// When the caller shuts down:
await watcher.close();
```

`buildApplication` type-checks and publishes once. `watchApplication` starts an initial build and then publishes successful rebuilds. A failed build does not replace the previous `dist`. Close the watcher to release its resources; a watch `signal` cancels pending publication but does not replace `close()`.

## Dev and preview

```ts
import { devApplication, previewApplication, resolveApplication } from "@hewel/zgui-build";

const app = await resolveApplication("./my-app");
const host = { hostRoot: "/path/to/matching/zgui-js" };

// Launch one desktop host and reload it on successful package publications.
await devApplication(app.app, { ...app, ...host, hostArgs: ["--exit-after-ms", "5000"] });

// Capture an offscreen frame. Set watch: true to rerun on successful rebuilds.
const status = await previewApplication(app.app, { ...app, ...host, out: "./preview-output" });
```

Both return the native process exit status and accept an `AbortSignal` for shutdown. One-shot preview rejects on a build or host preparation failure; `onOutput` reports the output path before the run. Preview accepts `profile`, `fixture`, `scenario`, and `out` overrides; `out` must not already exist. The default profile needs a usable graphics adapter.

`prepareNativeHost({ root, signal })` builds the host in the specified Cargo checkout and returns its executable path. `runNativeHost(executable, args, { root, signal })` runs a host with inherited stdio and returns its exit status.

## Package identity and capabilities

```json
{
  "name": "@example/status-app",
  "zgui": {
    "id": "org.example.status-app",
    "capabilities": { "shellSurfaces": true }
  }
}
```

If `zgui.id` is omitted, the ID stays `dev.example.<app>`. An explicit ID must be a non-empty string without leading or trailing whitespace or control characters. It need not be reverse-DNS.

All four application APIs accept `extraCapabilities` and `capabilitiesFile`. The CLI accepts `--capabilities-file <path>` for `build`, `dev`, and `preview`. The file contains a capabilities object, not a `package.json`; relative paths resolve from the caller's current working directory. Watch modes reread it after edits or atomic replacements of either the configured path or its symlink target.

Each source is validated independently. `shellSurfaces` and `controlChannel` are ORed; `imagePrefixes`, `unixSockets`, `namedSockets`, and `commands` are merged by exact string and sorted. Duplicates within one source are errors. Host path templates, such as `$XDG_RUNTIME_DIR/host-${WAYLAND_DISPLAY}.sock`, are sealed verbatim. False flags and empty arrays are omitted from the manifest. Use only trusted overlays: their merged grants are sealed into the build ID.

**Only the app's base `package.json` can declare `sessionLock: true`.** Either overlay source specifying it as `true` rejects the build even if the base already grants it. `false` or omission neither grants nor revokes the base declaration.

Build-time overlays may grant `shellSurfaces: true`. The runtime `--extensions-overlay`
is stricter: it cannot add either `shellSurfaces` or `sessionLock` authority.

`buildApplication(app, { bytecode: true })` resolves a compatible host through the
same prebuilt/Cargo lookup as `devApplication`. `packageApplication` compiles with
the same verified executable it ships. `resolveNativeHost` returns the absolute
`runRoot` for invoking the executable.

```sh
zgui-js build ./my-app --watch --capabilities-file ./generated/plugin-capabilities.json
```

## Outside the SDK workspace

Install or link the build package and SDK dependencies in the consumer repository. Declare `@hewel/zgui` and, if used, `@hewel/zgui-services` in the app. The app owns `src/main.tsx`, `tsconfig.json` (which can extend `@hewel/zgui/tsconfig.native.json`), `package.json`, and optional assets. Pass `appDirectory` or use `resolveApplication` to select it. No workspace-wide `node_modules` link is needed.

`root` selects the SDK's default preview inputs and `bun.lock`/`Cargo.lock` provenance, not the app's working directory. The default is the SDK checkout for a linked build package or the shipped SDK resources for an installed tarball.

On Linux `x86_64` (`glibc >= 2.39`), installing `@hewel/zgui-cli` automatically pulls the matching prebuilt `@hewel/zgui-host-linux-x64` binary via `optionalDependencies`, so `dev`, `preview`, and `package` resolve `zgui-host` with no Rust toolchain or explicit host flags. Host selection runs in deterministic precedence order:

1. `--host-executable` / `ZGUI_JS_HOST_EXECUTABLE`
2. Explicit `--host-root` (`hostRoot`)
3. Verified installed platform package (`@hewel/zgui-host-<os>-<arch>`)
4. Autodetected local Cargo checkout containing `crates/zgui-host`

For custom host builds or platforms without a prebuilt host package, pass a matching checkout as `hostRoot` (or `--host-root <dir>`) to compile via Cargo, or pass a prebuilt binary as `hostExecutable` (or `--host-executable <path>`). The checkout and binary must match the package's host API and `lightningcss-v1` style profile.

For either CLI command, `--host-root <dir>` selects the checkout used to build the host and the working directory used to run it. `--host-executable <path>` skips Cargo and runs a prebuilt host; it can be combined with `--host-root` to set the host's working directory. Relative flag values resolve from the caller's current directory.

```sh
zgui-js dev ./my-app --host-root ../zgui-js
zgui-js preview ./my-app --watch --host-executable ../zgui-js/target/debug/zgui-host
```

### Local two-repo edit loop (`link:`)

When actively developing `zgui-js` alongside an external consumer repo (such as a Wayland desktop shell):

1. Install `zgui-js` dependencies in the checkout (`bun install --frozen-lockfile`).
2. In the consumer repository, temporarily point `@hewel/zgui` and `@hewel/zgui-services` (plus `@hewel/zgui-build` if invoked programmatically, e.g. from a composer workspace package) at `link:../zgui-js/packages/<pkg>` across every workspace `package.json` that declares them, then run `bun install`. Leave `@hewel/zgui-cli` installed from npm or run `bun ../zgui-js/packages/cli/src/cli.ts ...` directly (its manifest uses `workspace:*` for internal packaging). Keep machine-local `link:` and lockfile edits uncommitted.
3. Because a linked `@hewel/zgui-build` (or checkout CLI) sets `workspaceRoot` to the checkout, `buildApplication` rebuilds checkout `packages/ui/dist` and `packages/services/dist` on demand and `watchApplication` / `zgui-js dev` watches `packages/{ui,services}/src` for hot reloads.
4. Select the local Rust host explicitly with `--host-root ../zgui-js` or `--host-executable` / `ZGUI_JS_HOST_EXECUTABLE` (`zgui-js doctor --json` reports the active executable and resolution source).
5. **Single Solid runtime rule:** import reactive APIs through `@hewel/zgui` (which uses a single physical `solid-js` tree inside the checkout). If an app or third-party dependency bare-imports another installed copy of `solid-js`, the compiler rejects the duplicate graph and prints the conflicting paths; switch to packed `.tgz` tarballs in that case rather than weakening singleton validation.

### Snapshot and pre-release verification (`.tgz` tarballs)

To verify exact npm-installed packaging without publishing to npmjs.org:

- **SDK-only snapshot:** run `bun pm pack --filename /tmp/zgui-local/zgui.tgz` inside `packages/ui` (and `packages/services` as needed) to run `prepack` builds cleanly and install those `.tgz` files into the consumer.
- **Full 6-package matrix:** run `cargo build -p zgui-host` then `bun run release:pack -- --out /tmp/zgui-local --host-binary ./target/debug/zgui-host`, and point consumer `dependencies`/`overrides` to the generated tarballs. Use distinct filenames or `bun install --force` when repacking the same SemVer version.
