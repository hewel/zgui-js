# @hewel/zgui-build

Build, watch, develop, and preview native zgui applications from Bun (≥ 1.4.4). Import from `@hewel/zgui-build`; importing it does not start a compiler or host, install signal handlers, or run the CLI.

## Build and watch

```ts
import { buildApplication, resolveApplication, watchApplication } from "@hewel/zgui-build";

const app = await resolveApplication("./my-app");
const options = {
  ...app,
  extraCapabilities: { namedSockets: ["status"] },
  capabilitiesFile: "./generated/plugin-capabilities.json",
};

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

```sh
zgui-js build ./my-app --watch --capabilities-file ./generated/plugin-capabilities.json
```

## Outside the SDK workspace

Install or link the build package and SDK dependencies in the consumer repository. Declare `@hewel/zgui` and, if used, `@hewel/zgui-services` in the app. The app owns `src/main.tsx`, `tsconfig.json` (which can extend `@hewel/zgui/tsconfig.native.json`), `package.json`, and optional assets. Pass `appDirectory` or use `resolveApplication` to select it. No workspace-wide `node_modules` link is needed.

`root` selects the SDK's default preview inputs and `bun.lock`/`Cargo.lock` provenance, not the app's working directory. The default is the SDK checkout for a linked build package or the shipped SDK resources for an installed tarball. Install a compatible host separately: an installed tarball contains no Cargo workspace or executable. Pass a matching checkout as `hostRoot` for dev/preview, or pass its prebuilt native host as `hostExecutable`. The checkout and binary must match the package's host API and `lightningcss-v1` style profile.

For either CLI command, `--host-root <dir>` selects the checkout used to build the host and the working directory used to run it. `--host-executable <path>` skips Cargo and runs a prebuilt host; it can be combined with `--host-root` to set the host's working directory. Relative flag values resolve from the caller's current directory.

```sh
zgui-js dev ./my-app --host-root ../zgui-js
zgui-js preview ./my-app --watch --host-executable ../zgui-js/target/debug/zgui-host
```
