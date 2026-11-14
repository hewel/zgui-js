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

## P1 — real TSX capture

`preview run` now mounts the integrity-verified package through the production
`Package`, `Session`, rquickjs runtime, UI bridge, native root and binding.
The headless platform supplies real elapsed monotonic time in the explicitly
nondeterministic wall profile. It does not invoke example `advance` functions.

```sh
target/debug/native-host preview run \
  --bundle apps/examples/counter/dist \
  --profile preview/profiles/wall-static.json \
  --fixture preview/fixtures/empty.json \
  --scenario preview/scenarios/counter-capture.json \
  --out artifacts/preview/counter-new-run
```

Profile/fixture/scenario validation precedes application execution. P1 admits
only explicit empty fixture services/events and `settle`/`capture`; virtual
profiles and exact frame times fail capability negotiation. Step/capture IDs
are bounded to 128 ASCII characters. A step timeout may lower, not raise, its
profile limit. Native image readiness remains unavailable; the production
bridge does not admit an image element.

### Evidence and failure ownership

The passive bridge snapshot contains only attached live nodes. Text traversal,
node count and retained text bytes are bounded. Unknown semantics, enabled and
visibility information remain null rather than inferred.

The vendored runtime adds `Window::completed_frame()` without changing the
probe callback. It records the actual native timestamp/outcome and the revision
entering the last completed layout/fragment pass. Late writes or outstanding
layout obligations cannot certify old geometry. An `Undamaged` validation
retains the older pixel presentation identity; held, occluded and recovered
frames cannot validate it.

Complete owned tree/frame checkpoints are atomically saved before subsequent
callbacks can fail. Captures include separate validation/presentation identities
and hashes of the actual PNG and tree files. The controller verifies the sealed
worker inventory and publishes the final result last; the control message stays
small independently of the number of artifacts or verbose package IDs.

Fixture violations stay fatal even when caught in JavaScript, including in
disposers. Original generated and source-mapped stacks retain their producing
build/map identity. The first cause is saved before failure readback or cleanup;
later publication errors retain it as `originalError`. Recovery never executes
application code or labels historical evidence as a fresh successful capture.

### Observed gates

| Gate | Result and evidence |
| --- | --- |
| PV05: production TSX path | Passed. Counter displays `Count: 0`, `Doubled: 0` and `Increment`; real 1400 × 1050 PNG visually checked. Final evidence: `target/preview-p1-v2-counter/`. |
| PV06: package integrity/publication | Passed. Existing loader tests verify declared-resource integrity and source maps. A mixed entry/manifest bundle returned `E_BUNDLE` without mounting. |
| PV07: actual source mapping | Passed. A compiled TSX mount exception retained generated `app.mjs` coordinates and mapped `src/main.tsx:7`, with the producing build/map digest. |
| PV08: failed mount | Passed. No capture claimed; all eight resource counters/engine ownership returned to zero. |
| Fixture-only construction | Passed. Caught and uncaught mount attempts and a caught disposer attempt returned `E_FIXTURE_MISSING_SERVICE`, never success. |
| Teardown failure evidence | Passed. A throwing disposer returned `E_TEARDOWN`; historical tree/frame/PNG remained available, with no claim of clean teardown. |
| Bounded control/artifacts | Passed. A 70 KiB package ID stayed in the detailed identity artifact while stdout remained bounded. An output-budget failure retained the original mapped JS error through controller publication failures. |
| Graphics/watchdog | Passed. No adapter returned exit 7; a 1 ms watchdog returned exit 5 with teardown explicitly unverified. |

Focused checks covered scenario/fixture preflight, session/provider policy,
package integrity/mapping, bridge topology, enclosing JS deadlines, publication
recovery and native frame coherence. The standalone native frame gate passed
eight cases, including semantic-only undamaged frames and late no-op writes.
Its first geometry test exposed a redundant observer write: that write really
increments the view revision without requiring relayout. The corrected tests
retain conservative rejection and verify the next explicit validation frame;
the production coherence invariant was not weakened.

```sh
cargo test -p native-host preview::scenario
cargo test -p native-host preview::fixture
cargo test -p native-host session::tests
cargo test -p native-host package::tests
cargo test -p native-host preview::run::tests
cargo test -p ui-bridge --test topology
cargo test -p js-runtime owner_deadline
cargo test --manifest-path vendor/zgui-runtime/Cargo.toml --offline --test held_frame
```

The native snapshot is a standalone workspace, so root `cargo test -p
zgui-runtime` cannot run its dev-dependencies. Independent review identified
causal-error loss, fixture classification/teardown gaps and oversized control
messages. Those paths were corrected and exercised by focused regressions and
the public CLI.
