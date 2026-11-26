# Preview troubleshooting

Diagnose headless preview failures by error code (`result.json -> firstError.code`) or environment symptom.

## Structured preview error codes

| Code | Exit | Common cause | Fix |
|---|---|---|---|
| `E_PROFILE` | 2 | Invalid profile JSON, `opaque: true` with non-`ff` background alpha, or invalid hex length. | Use `#RRGGBBff` when `opaque: true`, or set `opaque: false` for `#RRGGBBAA` (e.g. `#00000000`). |
| `E_CAPABILITY` | 6 | App called a shell/service API omitted from `package.json -> zgui.capabilities`, or `pureShell: true` without `shellSurfaces`/`sessionLock`. | Add the missing capability to `zgui.capabilities` (or `extraCapabilities` for non-lock plugins). |
| `E_UNSUPPORTED_CAPABILITY` | 2 | Scenario uses `clock.advance` on a `wall` profile or declares an unsupported capability. | Switch profile `clock.mode` to `"virtual"`, or check `preview doctor` capabilities. |
| `E_GRAPHICS_UNAVAILABLE` | 7 | No Vulkan adapter/ICD reachable (`pixels: false`, `capture.transparent: false`). | On Nix-only hosts, run inside `shell.nix` (sets `VK_DRIVER_FILES` to Nix Mesa ICDs); on standard Linux distros ensure Mesa Vulkan drivers are installed and verify `preview doctor`. |
| `E_CAPTURE_NOT_READY` | 5 | Pending native animation/timer deadline without advancing virtual time, or surface never settled. | Insert `clock.advance` (`byMs`) and/or `settle` before `capture`. |
| `E_ASSET_TIMEOUT` | 5 | Visible image/asset did not finish loading before capture. | Check `fixture.services.images` mappings and `zgui.capabilities.imagePrefixes`. |
| `E_TEARDOWN` | 9 | Leaked socket, command, image handle, timer, or unconsumed fixture script at shutdown. | Use owner-scoped `@hewel/zgui-services` helpers (`loadImageAsset`, `loadImageBytes`, `connectUnixSocket`, `spawnCommand`) or release manual handles in `onCleanup`; match fixture scripts 1:1 with calls. |
| `E_BASELINE_INCOMPATIBLE` | 2 | `preview compare` detected mismatched `renderer.adapter` (e.g. `llvmpipe` vs RADV), `renderer.alphaMode` (`Opaque` vs `PreMultiplied`), `Cargo.lock`, or profile digest. | Compare runs captured under the same adapter, opacity policy, and profile; regenerate baselines only when intentionally updating environments. |

## Nix & Vulkan Linux pitfalls

- **Segfault (exit 139) in `bun` or `zgui-host`**: caused by pointing `LD_LIBRARY_PATH` at `/usr/lib/x86_64-linux-gnu` (loads host distro `libc.so.6` into a Nix glibc process). On Nix-only hosts, enter `zgui-js/shell.nix` instead of setting distro `LD_LIBRARY_PATH`.
- **Missing `xkbcommon.pc` / `fontconfig.pc` during `cargo build`**: `cargo` and `pkg-config` came from Nix without dev headers in `PKG_CONFIG_PATH`. On Nix-only hosts, run builds inside `zgui-js/shell.nix`.
- **Unexpected CPU `llvmpipe` instead of AMD/Intel hardware Vulkan**: check `ls -l /dev/dri/renderD128` and `id -nG`. If the user is not in the `render` group, hardware DRI render nodes are unreadable and Mesa falls back to `llvmpipe`; adding the user to `render` and re-logging unlocks RADV.
- **Transient test timeouts under high `bun test` parallelism**: `bun test` defaults to a 5s timeout; pass `--timeout=30000` (and `--isolate` for multi-file preview runs, matching `package.json`).
