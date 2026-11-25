# niri blur shapes: client integration

`WindowSurface`, `LayerSurface`, and `PopupSurface` use the existing `blurRegion` API. `getBackgroundBlurSupported()` is true for either blur protocol; it does not promise that the compositor's rules permit blur. `SessionLockSurface` never allocates blur objects.

## Registry and wire contract

Bind `niri_blur_shape_manager_v1` at version 1 when available. Prefer it over `ext_background_effect_manager_v1` with `Capability::Blur`. Match the compositor fork against the [pinned XML](../../vendor/zgui-platform-wayland/src/driver/niri-blur-shape-v1.xml): a registry name and version cannot distinguish incompatible v1 forks. Rust clients can generate the interfaces and client requests with `wayland_scanner::generate_interfaces!` and `wayland_scanner::generate_client_code!` as in [`background_effect.rs`](../../vendor/zgui-platform-wayland/src/driver/background_effect.rs).

| Object  | Opcode | Request                       | Effect                                                                                         |
| ------- | -----: | ----------------------------- | ---------------------------------------------------------------------------------------------- |
| Manager |      0 | `destroy`                     | Retire the manager; existing surface objects remain valid.                                     |
| Manager |      1 | `get_surface(id, wl_surface)` | Create the one active shape object allowed per surface.                                        |
| Surface |      0 | `destroy`                     | Retire the object and remove the override on commit. Destroy before the role and `wl_surface`. |
| Surface |      1 | `clear`                       | Set an explicit empty list. No blur and no fallback until replaced or unset.                   |
| Surface |      2 | `add_rounded_rect`            | Append a shape to the pending list.                                                            |
| Surface |      3 | `unset`                       | Remove the override on commit and allow fallback to ext blur or compositor rules.              |

A surface request takes effect on `wl_surface.commit`. On first nonempty blur, bind lazily, send `clear`, then one `add_rounded_rect` per mask, then commit with the buffer. Thereafter an empty mask sends `clear`, not `unset`. An untouched surface never binds, leaving compositor rules in charge. For protocol changes, destroy the old surface object and stage its replacement in the same commit batch. Do not send either operation during registry dispatch or an unrelated shell input-region commit.

## Coordinates and opacity

`add_rounded_rect` takes ten signed 24.8 values: `x, y, width, height, radius_tl, radius_tr, radius_br, radius_bl, corner_exponent, opacity`. Use surface-local logical pixels, not physical pixels or rounded integers. Round shared edges to nearest wire ticks, then subtract those edges to obtain widths and heights. Verify all values and positive extents fit `i32` ticks; divide the rounded ticks by 256 before passing them to the scanner's truncating encoder. For example, `2.83` becomes `724 / 256 = 2.828125`.

Use exponent `2` for circular corners or `2.83` before quantization for zgui squircles. Radii are nonnegative, opacity is clamped to `[0, 1]`, and shapes with zero wire opacity are omitted. Cap the list at 256. Never submit overlapping masks: niri blends them in order. zgui preserves exact SDF shapes where possible and partitions incompatible clip/corner or overlap cases into disjoint strips with maximum effective opacity. In shape mode, `minOpacity` is ignored; binary fallback re-resolves it rather than projecting shape bounds onto `wl_region`.

## Verification and redistribution

Run `bun run test:vendor zgui-platform-wayland` and `bun test --isolate --timeout=30000 tests/preview/blur-region.test.mjs` in the devShell after building the host. Socket tests verify client requests and commits; preview verifies mask evidence, not compositor pixels. On a matching live niri fork, check popup paint and retirement, SDF edge alignment while animating, fractional scaling, HDR, xray/non-xray, and liquid-glass coexistence. Global and per-surface compositor blur settings can override a client's request.

The operator-supplied XML is vendored verbatim under its original GPL-3.0-or-later header; the offline test pins SHA-256 `b1118671ff88677d9acefb63c5894dbe241e003b435d28f7832eba84909dd352`. The approved integration does not establish redistribution rights for Apache-2.0 host packages containing generated bindings. Review licensing and notices before distributing affected source or binaries; no release or license change is part of this integration.
