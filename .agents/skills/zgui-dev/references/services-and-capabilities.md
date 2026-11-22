# Capabilities, host services & programmatic build

Use when declaring `zgui.capabilities`, calling `@hewel/zgui-services`, or integrating `@hewel/zgui-build` from an external repository (such as `z-shell`).

## Manifest capabilities (`package.json`)

```json
{
  "zgui": {
    "id": "my-shell",
    "entry": "src/main.tsx",
    "capabilities": {
      "shellSurfaces": true,
      "sessionLock": false,
      "imagePrefixes": ["$XDG_DATA_HOME/icons", "$HOME/.cache/thumbnails"],
      "unixSockets": ["$XDG_RUNTIME_DIR/my-daemon.sock"],
      "namedSockets": ["NIRI_SOCKET", "SHELL_BUS"],
      "commands": ["/usr/bin/wpctl", "/usr/bin/playerctl"],
      "controlChannel": true
    }
  }
}
```

- **Constrained path templates**: `imagePrefixes` and `unixSockets` accept literal absolute paths or single leading prefixes `$XDG_RUNTIME_DIR`, `$HOME`, `$XDG_DATA_HOME`, and `${WAYLAND_DISPLAY}` (resolved once at session launch from host environment or preview `fixture.runtimeEnvironment`).
- **Named sockets**: declare logical endpoint names in `namedSockets`, bind them at launch via `--bind-socket NAME=/path` (or env fallback such as `NIRI_SOCKET`), and look them up at runtime with `getAuthorizedEndpoint(name)` (returns `string | null`, throws `E_CAPABILITY` for undeclared names) or inspect all resolved paths with `getResolvedCapabilities()`.
- **Fail-closed elevation rule**: build-time `extraCapabilities` / `--capabilities-file` can supply plugin capabilities during composition, but never elevates `sessionLock`.

## `@hewel/zgui-services`

Call host service APIs during mount, effects, or event handlers (never at module top-level evaluation). Owner-scoped helpers register `onCleanup` automatically when called inside a Solid owner:

| Purpose | Owner-scoped helper | Manual-lifetime variant |
|---|---|---|
| Package text asset | `loadTextAsset(path)` | `readTextAsset(path)` (`{ promise, cancel }`) |
| Package image asset | `loadImageAsset(path)` | `registerImageAsset(path)` (`{ src, release }`) |
| Dynamic encoded bytes (PNG/JPEG/WebP/SVG) | `loadImageBytes(uint8Array)` | `registerImageBytes(uint8Array)` (`{ src, release }`) |
| Unix domain socket (`"line"` or `"length"` framing) | `connectUnixSocket(path, opts)` | — (also returns `.close()`) |
| Subprocess (direct `exec`, no shell) | `spawnCommand(program, args, opts)` | — (buffered returns `{ promise, cancel }`; `stream: true` returns `{ cancel, kill }`) |
| Inbound CLI control messages (`zgui-host msg`) | `onControlMessage(handler)` | — (returns unsubscribe `() => void`) |

- **`length` framing binary images**: in `connectUnixSocket(path, { framing: "length", onMessage, onBinaryImage })`, frames prefixed with kind byte `0x00` deliver UTF-8 to `onMessage`, and frames prefixed with `0x01` deliver a session-scoped `ImageHandle` to `onBinaryImage` (release it explicitly when replaced or disposed).
- **Dynamic image bytes**: `loadImageBytes(bytes)` / `registerImageBytes(bytes)` enforces session memory/count budgets and returns an opaque `zgui-blob://` `src` accepted by `<Image src={handle.src} />`.
- **Clean teardown**: release every manual `ImageHandle` and cancel/close handles outside Solid owners so preview teardown reports `counts` all zero (`clean: true`).

## `@hewel/zgui-build` programmatic SDK

External repositories consume `@hewel/zgui-build` directly without CLI side effects:

```ts
import {
  buildApplication,
  watchApplication,
  devApplication,
  previewApplication,
  prepareNativeHost,
  runNativeHost,
} from "@hewel/zgui-build";

const { dist, manifest } = await buildApplication("shell", {
  appDirectory: "/abs/path/to/app",
  extraCapabilities: { unixSockets: ["$XDG_RUNTIME_DIR/z-shell.sock"] },
});
```

- Pass `hostExecutable` to `devApplication` / `previewApplication` when pointing at a prebuilt `zgui-host` binary from another workspace.
- Pass `capabilitiesFile` (or `extraCapabilities`) when composing plugin capabilities dynamically at build time.
