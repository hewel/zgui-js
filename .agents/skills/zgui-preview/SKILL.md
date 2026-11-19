---
name: zgui-preview
description: Run, write, and debug headless zgui-js preview visual tests with native-host preview. Use when verifying UI renders or PNG pixels, testing transparent surfaces (capture.transparent), multi-surface LayerSurface/PopupSurface/SessionLockSurface scenarios, running preview doctor/compare, or troubleshooting Nix/Vulkan headless rendering and E_TEARDOWN/readiness faults.
---

Workflow for headless visual verification, pixel inspection, and regression testing with `native-host preview`.

## 1. Environment & preflight

- **Nix-based Linux hosts**: wrap every Cargo build and `native-host preview` / `bun test tests/preview/...` invocation in the `zgui-js` root devShell so `pkg-config`, native libraries (`libxkbcommon`, `fontconfig`, `freetype`, `vulkan-loader`, `wayland`), and Mesa Vulkan ICDs (`VK_DRIVER_FILES`) share the same glibc:
  ```sh
  nix-shell --run "cargo build --offline -p native-host"
  nix-shell --run "target/debug/native-host preview doctor --profile preview/profiles/default.json"
  ```
- **GPU vs CPU rendering**: when `/dev/dri/renderD128` is readable (`render` group), Vulkan uses hardware AMD/Intel RADV automatically; otherwise Mesa falls back to CPU `llvmpipe` (fully functional for pixel tests, slower on large surfaces).
- **Preflight criterion**: `preview doctor` exits `0` with `"status": "passed"`, `"pixels": true`, and `"capture.transparent": true`.

## 2. Core verification loop

1. **Build the bundle** (`dist/manifest.json`):
   - Inside `zgui-js`: `bun run build --app <name>` (or `previewApplication()` from `@hewel/zgui-build`).
   - From a downstream repo (such as `z-shell`): call `buildApplication(...)` or `previewApplication(..., { hostExecutable })` from `@hewel/zgui-build`.
2. **Author `profile.json`, `fixture.json`, and `scenario.json`**:
   - Use minimal copy-pasteable templates and op schemas in [`references/contracts.md`](references/contracts.md).
   - Keep profiles on `"clock": { "mode": "virtual", ... }` whenever driving timers, springs, animations, or deterministic captures; advance virtual time explicitly with `clock.advance`.
3. **Run headless preview**:
   ```sh
   nix-shell --run "target/debug/native-host preview run \
     --bundle <dist-dir> \
     --profile <profile.json> \
     --fixture <fixture.json> \
     --scenario <scenario.json> \
     --out <fresh-empty-dir>"
   ```
   - `--out` must target a non-existent or empty directory per run.
4. **Inspect evidence**:
   - Check `<out>/result.json`: `status === "passed"`, `exitCode === 0`, `firstError === null`, `assertions.failed === 0`, and `teardown.clean === true`.
   - Inspect `<out>/captures/<captureId>.frame.json` (geometry, `alphaMode`, readiness, resource counts) and `<out>/captures/<captureId>.png` (straight-alpha 8-bit RGBA PNG; decode and sample pixels programmatically or view the PNG directly).

## 3. Transparent & shell surface verification

- **Transparent capture (`capture.transparent`)**:
  - Set `"surface": { ..., "opaque": false, "background": "#00000000" }` (any `#RRGGBBAA` 9-char hex is valid when `opaque: false`; `opaque: true` requires `#RRGGBBff`).
  - Declare `"requiredCapabilities": ["pixels", "capture.transparent"]` in `scenario.json`.
  - Expect `graphics.alphaMode === "PreMultiplied"`, `graphics.compositorTransparencyValidated === false`, and straight-alpha RGBA PNG pixels (`[0, 0, 0, 0]` on uncovered clear background, `0 < a < 255` on shadows/translucent fills, `a === 255` on opaque foregrounds).
- **Pure-shell surfaces (`<LayerSurface>`, `<PopupSurface>`, `<SessionLockSurface>`)**:
  - Set `"pureShell": true` in `profile.json` and `"monitors.set"` early in `scenario.json` to populate `useMonitors()`.
  - Target surfaces by logical name (`"surface": "bar-1"`) in `pointer.*`, `key.press`, `wheel`, `capture`, and `popupDone` (infers `surface.multiple`).
  - Profile opacity propagates to `<LayerSurface>` and `<PopupSurface>`, while `<SessionLockSurface>` always retains an opaque black backdrop beneath author CSS.

## 4. Visual baseline comparison (`preview compare`)

```sh
nix-shell --run "target/debug/native-host preview compare \
  --baseline <baseline-dir> \
  --actual <run-dir> \
  --policy preview/comparison/strict.json \
  --out <diff-dir>"
```

- Strict comparison validates compatibility before pixel diffing: physical extent, scale, font config, `renderer.adapter`/`driver`, `renderer.alphaMode`, `Cargo.lock` digest, and profile/scenario/fixture digests must match.
- Comparing `llvmpipe` against a hardware RADV baseline, or an opaque run against a transparent run, intentionally exits `2` (`E_BASELINE_INCOMPATIBLE`) instead of silently scoring mismatched pixels.

## 5. Troubleshooting

When a run fails or exits non-zero, check `result.json` (`firstError.code`) and consult [`references/troubleshooting.md`](references/troubleshooting.md).

## Completion criteria

A preview verification pass is done when:
- `result.json` reports `status: "passed"`, `exitCode: 0`, `assertions.failed: 0`, and `teardown.clean: true`.
- Every target PNG capture has been checked (pixel RGBA values or visual inspection) for layout, glyphs, and expected alpha channels.
- Temporary output directories are cleaned up after test runs.
