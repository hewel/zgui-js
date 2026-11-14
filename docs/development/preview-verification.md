# Preview implementation and verification

The [preview specification](preview-mode.md) is preserved unchanged. The main
host's [baseline](baseline.md) remains the production path; preview does not
replace its renderer or JavaScript bridge.

## P0 — environment and real calibration

Implemented `native-host preview doctor --profile PATH [--out NEW_OR_EMPTY_DIR]`.
It validates the closed profile and shipped-font manifest, records exact embedded
lockfile inventories and native source identity, then runs native calibration in
a supervised child process. `--width`, `--height`, `--scale`, `--theme`, and
`--timeout-ms` override the resolved profile. `--format json` is the only format.
Stdout is one JSON object, bounded to 8 KiB. Detailed inventory and PNG evidence
are saved when `--out` is supplied.

```sh
cargo build -p native-host --locked
target/debug/native-host preview doctor \
  --profile preview/profiles/default.json --out target/preview-p0-calibration
ZGUI_BACKENDS=none target/debug/native-host preview doctor \
  --profile preview/profiles/default.json --out target/preview-p0-no-adapter
target/debug/native-host preview doctor \
  --profile preview/profiles/default.json --timeout-ms 1 \
  --out target/preview-p0-watchdog
```

Use new output names when repeating these commands. An exclusive ownership record
and atomic no-replace publication reject competing controllers and stale output.
The controller publishes `result.json` last. After worker failure, retained files
are accounted against the output budget; completed files are historical and
orphan temporary files are partial. A killed worker never claims clean teardown.
The local filesystem must support same-directory hard links for atomic
no-replace publication.

### Observed gates

| Gate | Result and evidence |
| --- | --- |
| PV01: real renderer and fonts | Passed. Actual Vulkan/wgpu pixels and native shaped/rasterized text; `target/preview-p0-calibration/calibration.png` is 1400 × 1050 at scale 1.75. Its bytes match the earlier calibration image that was opened and visually reviewed. |
| PV02: unavailable adapter | Passed. `ZGUI_BACKENDS=none` returned exit 7 / `E_GRAPHICS_UNAVAILABLE`; no PNG was published. |
| PV03: extent, orientation, channels, stride | Passed. Real 1400-wide and odd 1399-wide readbacks; corner swatches and tightly packed lengths checked by the calibration. |
| PV04: color and alpha | Passed for the opaque capture scope. Gray and half-alpha-over-white swatches match gamma-encoded compositing; every final alpha is 255. Separate conversion tests cover zero/low/opaque alpha and BGRA ordering without an extra transfer curve. |
| Hard watchdog | Passed. Public 1 ms deadline returned exit 5 / `E_WATCHDOG`. An isolated blocked subprocess regression also verified termination without a teardown claim. |
| Artifact protocol | Passed. All three public CLI results matched their saved `result.json`; every inventoried file's length and SHA-256 matched. Successful stdout was 2766 bytes. |

The native renderer reported AMD Radeon RX 7900 XTX (RADV NAVI31), Vulkan,
Mesa `26.2.6-arch3.1`, `Bgra8Unorm` target/scene, `Rgba8Unorm` scratch,
opaque alpha mode and native sRGB handling tier. Stored values are
premultiplied gamma-encoded colors; PNG conversion does not apply another
linear-to-sRGB transform. These results are not cross-GPU identity claims.

Pinned zgui/rquickjs revisions and registry versions are unchanged. Cargo gained
the matching headless package plus direct uses of already-resolved PNG/TOML and
native renderer/layout dependencies. The default fonts are licensed, hashed
Latin/CJK files with [pinned provenance](../../preview/fonts/PROVENANCE.md), not
system enumeration.

### Focused verification and review

Sixteen distinct P0 tests passed across the initial integrated run and targeted
fix runs; one ignored test is the blocked-child fixture invoked by its supervising
test. Coverage includes closed schemas, physical allocation bounds, font hashes
and containment, odd-width conversion, real empty-backend rejection, watchdogs,
output ownership and lost-worker accounting.

The initial schema test caught serde accepting fields on a unit wall-clock enum
variant. An empty struct variant now rejects them, and that exact test passed.
Independent read-only review found a missing post-GPU step-deadline check,
lost artifact accounting on worker termination, and overwrite-prone publication.
All three were fixed; four focused output tests passed, the public CLI scenarios
above passed, and the bounded static recheck reported no remaining findings.

### Capability boundary

P0 advertises `pixels` only. The calibration uses an explicitly driven native
harness clock but does not evaluate an application or validate shared JS virtual
time. The virtual default profile is a declared target environment, not proof of
that capability. `wall-static.json` is provided for P1/P2. TSX capture, selectors,
semantic inspection, deterministic services, native asset readiness, transparent
capture, watch and lifecycle capabilities remain unadvertised until their gates
pass. Transparent byte conversion tests do not establish compositor transparency.
