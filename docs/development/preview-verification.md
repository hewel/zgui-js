# Preview implementation and verification

The [preview specification](preview-mode.md) describes the current generic scope.
The main host's [baseline](baseline.md) remains the production path; preview does
not replace its renderer or JavaScript bridge.

The phase results below are historical observations, not a current platform
support matrix. The niri service, panel example, niri fixture provider and
compositor-specific occlusion/layer gates have since been removed. Their earlier
results and evidence remain recorded here, but the removed commands and fixture
contracts are not shipped or runnable as current acceptance gates. Native UI,
resource services, scheduling, previews and the upstream layer-shell wrapper
remain in scope; platform and window-manager support is determined by zgui.

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

| Gate                                        | Result and evidence                                                                                                                                                                                                                              |
| ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| PV01: real renderer and fonts               | Passed. Actual Vulkan/wgpu pixels and native shaped/rasterized text; `target/preview-p0-calibration/calibration.png` is 1400 × 1050 at scale 1.75. Its bytes match the earlier calibration image that was opened and visually reviewed.          |
| PV02: unavailable adapter                   | Passed. `ZGUI_BACKENDS=none` returned exit 7 / `E_GRAPHICS_UNAVAILABLE`; no PNG was published.                                                                                                                                                   |
| PV03: extent, orientation, channels, stride | Passed. Real 1400-wide and odd 1399-wide readbacks; corner swatches and tightly packed lengths checked by the calibration.                                                                                                                       |
| PV04: color and alpha                       | Passed for the opaque capture scope. Gray and half-alpha-over-white swatches match gamma-encoded compositing; every final alpha is 255. Separate conversion tests cover zero/low/opaque alpha and BGRA ordering without an extra transfer curve. |
| Hard watchdog                               | Passed. Public 1 ms deadline returned exit 5 / `E_WATCHDOG`. An isolated blocked subprocess regression also verified termination without a teardown claim.                                                                                       |
| Artifact protocol                           | Passed. All three public CLI results matched their saved `result.json`; every inventoried file's length and SHA-256 matched. Successful stdout was 2766 bytes.                                                                                   |

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

P0 originally advertised only `pixels`. The calibration uses an explicitly driven native
harness clock but does not evaluate an application or validate shared JS virtual
time. The virtual default profile is a declared target environment, not proof of
that capability. `wall-static.json` is provided for P1/P2. TSX capture, selectors,
semantic inspection, deterministic services, native asset readiness, watch and
lifecycle capabilities remain unadvertised until their gates pass.

With a transparent profile, `preview doctor` checks premultiplied readback and
decoded straight-alpha PNG samples. The current `capture.transparent` flag
follows `pixels`; both are false on `E_GRAPHICS_UNAVAILABLE` in the doctor
result and the run's `environment.json`. Neither establishes compositor
transparency. Lock surfaces paint an opaque black backdrop beneath author
content, even when the profile and the document root are transparent.

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

| Gate                                | Result and evidence                                                                                                                                                                                         |
| ----------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| PV05: production TSX path           | Passed. Counter displays `Count: 0`, `Doubled: 0` and `Increment`; real 1400 × 1050 PNG visually checked. Final evidence: `target/preview-p1-v2-counter/`.                                                  |
| PV06: package integrity/publication | Passed. Existing loader tests verify declared-resource integrity and source maps. A mixed entry/manifest bundle returned `E_BUNDLE` without mounting.                                                       |
| PV07: actual source mapping         | Passed. A compiled TSX mount exception retained generated `app.mjs` coordinates and mapped `src/main.tsx:7`, with the producing build/map digest.                                                           |
| PV08: failed mount                  | Passed. No capture claimed; all eight resource counters/engine ownership returned to zero.                                                                                                                  |
| Fixture-only construction           | Passed. Caught and uncaught mount attempts and a caught disposer attempt returned `E_FIXTURE_MISSING_SERVICE`, never success.                                                                               |
| Teardown failure evidence           | Passed. A throwing disposer returned `E_TEARDOWN`; historical tree/frame/PNG remained available, with no claim of clean teardown.                                                                           |
| Bounded control/artifacts           | Passed. A 70 KiB package ID stayed in the detailed identity artifact while stdout remained bounded. An output-budget failure retained the original mapped JS error through controller publication failures. |
| Graphics/watchdog                   | Passed. No adapter returned exit 7; a 1 ms watchdog returned exit 5 with teardown explicitly unverified.                                                                                                    |

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

## P2 — native interaction and immutable inspection

`testId` is application-owned diagnostic metadata, not a DOM/CSS attribute.
It accepts at most 512 UTF-8 bytes; null/undefined clears it. Removal retires the
identity with the native handle. Metadata-only changes have their own revision
and request a real native validation frame without manufacturing a DOM write.

```sh
pnpm build --app counter
target/debug/native-host preview run \
  --bundle apps/examples/counter/dist \
  --profile preview/profiles/wall-static.json \
  --fixture preview/fixtures/empty.json \
  --scenario preview/scenarios/counter-static.json \
  --out artifacts/preview/counter-interaction-new-run
target/debug/native-host preview inspect \
  --run artifacts/preview/counter-interaction-new-run \
  --capture after-increment --test-id count-label \
  --fields text,state,boundsLogical --max-nodes 1
```

Targets select one exact `testId` or native accessibility `role` plus `name`,
always on `main`. Duplicate matches fail rather than choosing the first.
Truncated topology, identity metadata or semantic names cannot certify a unique
target or a negative existence assertion. Unknown/truncated assertion values
fail explicitly rather than comparing a displayed prefix.

Pointer actions use current native fragments, placements, clip bounds and hit
testing; device coordinates are divided by surface scale once for native input.
A bounded 5 × 5 search accepts only a point whose actual hit chain includes the
target. Rejection is conservative, not proof that every possible point is
obscured. Raw logical positions are explicitly untargeted. Each click delivers
move/press/release as one native burst without an intermediate frame.
The complete burst is pumped before the step returns, so even a final input
executes its handler and owns any resulting failure. Frame budgets cover the
whole step, including post-input dispatch.

Keyboard actions deliver modifiers/down/up/modifier-clear as one native burst.
`text.commit` uses the native IME commit path, not a value setter. The main
headless surface is focused; modifiers are key-local. Native `preventDefault`
and editing behavior remain authoritative.

The AccessKit consumer applies the provider's ordered incremental updates,
including focus, root replacement and unreachable-node retirement. Lost log
continuity is fatal. `state.enabled` includes native disabled ancestors;
`state.visible` means a positive CSS-visible native box, not an occlusion or
pointer-reachability claim. Native editor `value` is separate from text/name;
authored semantic-value overrides cannot establish the editor's actual value.

Saved inspection verifies completed inventory entries, byte counts, SHA-256,
and matching capture frame/tree identities. It never mounts an application or
creates a renderer. Results select the requested capture, report the source
run status separately, and contain the selected node before limited parent/child
context. Fields are allowlisted; strings are bounded UTF-8 prefixes with explicit
`truncatedFields`. `--max-nodes` is 1–100, `--max-depth` is 0–64, and cursors bind
both snapshot digest and query. Responses remain within 8 KiB.

### Observed gates

| Gate                            | Result and evidence                                                                                                                                                                                                                              |
| ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| PV09: real native activation    | Passed. Counter changed from `Count: 0` to `Count: 1` through native pointer events, with coherent before/after captures.                                                                                                                        |
| PV10: selector errors           | Passed. Missing and duplicate IDs returned exit 4 with bounded candidates and frame identity; no fallback target.                                                                                                                                |
| PV11: clipped/obscured targets  | Passed. Fully clipped and covered controls failed without activation; partially clipped and transformed controls each activated through actual hit testing. Disabled ancestors and hidden controls also failed explicitly.                       |
| PV12: scale conversion          | Passed at 1, 1.25, 1.5, 1.75 and 2. Logical/device input coordinates and actual PNG extents matched; every run incremented once.                                                                                                                 |
| PV13: keyboard defaults         | Passed. Tab focused controls; Enter cancellation suppressed activation; Space activated. Native Home/Shift+ArrowRight selection followed by IME commit replaced `a` with `界`, producing `界bc` in both the editor and its input-event consumer. |
| PV14: saved inspection          | Passed with graphics disabled and an invalid Wayland endpoint. The selected initial/after captures returned different correct counter values; ten-node pagination was complete and a cursor from the other capture was rejected.                 |
| PV15: incremental semantic tree | Passed. Ordered native-provider tests cover unchanged nodes, focus, root replacement and retirement. Public branch removal removed `Retired child` from the retained full semantic tree; metadata rename retired the old test ID.                |

The fifteen public runs are retained under `target/preview-p2-v1-*`. Every
inventoried artifact's length/hash and terminal stdout/result agreement was
checked. Counter and native editing PNGs were opened and visually inspected;
they preserve the production rendering, including the counter's existing
low-contrast styling.

Focused checks passed: 12 scenario tests, 11 semantic-provider tests, three saved
inspection pagination/cursor/identity tests, and one bridge metadata ownership test.
The first semantic run exposed test fixtures reusing `Document::new()`'s
`DocumentId::FIRST`; explicit distinct document IDs corrected the fixtures
without weakening foreign-handle or retirement assertions.

Independent review found that queued final input could return before native
dispatch and that saved capture/run associations were unchecked. The fixes
drain the native burst within its action, bind capture/build/generation and
numeric metadata revision to the saved evidence, and reject incomplete identity.
Three real-GPU CLI regressions exercise throwing final pointer, key and text
handlers; all report the failure against that final action. The empty editor
fixture needed explicit positive dimensions before it could receive input.
The affected positive matrix was repeated under `target/preview-p2-v2-*`;
the earlier negative-target cases remain valid.

```sh
cargo test -p native-host preview::scenario::tests
cargo test -p native-host preview::semantics::tests
cargo test -p native-host preview::saved::tests
cargo test -p ui-bridge --test properties test_ids_follow_native_ownership_without_changing_dom_attributes
cargo build -p native-host
node --test tests/preview/final-input.test.mjs
```

## P3 — shared time, fixture providers and native resources

```sh
pnpm build --app counter
cargo build -p native-host
target/debug/native-host preview run \
  --bundle apps/examples/counter/dist \
  --profile preview/profiles/default.json \
  --fixture preview/fixtures/empty.json \
  --scenario preview/scenarios/counter-virtual.json \
  --out target/preview-counter-virtual
```

The virtual profile's exact native clock is installed before module evaluation.
`clock.advance` visits actual native/fixture deadlines and its exact endpoint;
fractional native timer deadlines are not rounded to guessed frame intervals.
`settle`, input, resource waits and resize/scale never advance virtual time.
Future pacing is reported rather than hidden. Real execution deadlines remain
independent, including while JS spins or continually queues jobs.

Virtual `Date.now`, zero-argument construction and callable `Date()` share the
configured epoch; explicit arguments, static methods and subclass construction
retain Date semantics. Worker timezone is UTC. `performance.now()` reads native
elapsed milliseconds; seeded `Math.random` uses `splitmix64-v1`. These policies
and the verified shipped fonts are saved in `environment.json`.

At the original P3 verification, fixture providers replaced `readTextAsset` and
the former `watchNiri` boundary, not application handlers. The niri provider and
its specific verification cases below are now retired; the current fixture
boundary is resource access. Missing providers cannot fall back to live services.
Zero-latency requests return before delivery. Eligible fixture work is ordered
by release time, declared events before request completions, then source order.
`service.emit` changes typed fixture state only. An `assert` step with
`serviceCall: {service, method, count, errorCode?}` checks exact calls and
explicitly acknowledges matching injected errors.
Unacknowledged injected errors fail finalization even when application code
catches them. `services.json` retains bounded request/response/state/cancellation
and disposal evidence. Wire objects reject positional arrays and duplicate fields.

Native `<Image>`/`<image>` uses immutable verified package bytes, real decoding,
native demand variants and the real paint atlas. Preview stages only complete
worker batches in launch order; waiting neither executes JS nor advances time.
Required visible images need current-generation attachment, successful texture
upload and current atlas residency. Allocation refusal is an asset failure;
upload/device refusal is a graphics failure. CSS URL/import/image-set references
are explicitly rejected because the native painter does not implement them.

Required visibility intersects native placed fragments with the physical viewport,
ancestor clips and current opacity. An offscreen decoded image need not be
uploaded to permit capture; making it visible restores the upload requirement.
Live package sessions bind the same immutable bytes but retain ordinary asynchronous
native publication. Replacing the image policy between owner shutdown and new mount
retires old completion domains, attachments, caches and replay leases without
reusing request or atlas identities.

An explicit `allowedPendingAssets` list names current pending request IDs, never
sources or stale generations. It can capture an already coherent placeholder
before publishing worker arrivals, but cannot permit failures. Captures bind
resource revision and requested surface configuration to their tree and genuine
validation frame. Pixel presentation retains its own timestamp. Saved inspection
checks these identities; a resize/scale returning to the old pixel extent does
not make the old logical configuration current.

### Observed gates

| Case | Observed result                                                                                                                                                                                                                               |
| ---- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| PV16 | Infinite JS under a frozen clock exits 5 with `E_JOB_BUDGET`; no virtual advance is required.                                                                                                                                                 |
| PV17 | Infinite queued work is bounded; a throwing job exits 3 and preserves `original-p3-job-error`.                                                                                                                                                |
| PV18 | A 0.25 ms native timer fires at its actual deadline; equal-deadline cancellation preserves A→Promise ordering and suppresses B.                                                                                                               |
| PV19 | Resize/scale retain the held timestamp. A resize then scale returning to the original pixel extent cannot validate the old logical configuration or dispatch against it.                                                                      |
| PV20 | Native validation ordinals include genuine additional frames at 0, 40 and 80 ms; unchanged pixels retain their earlier presentation identity.                                                                                                 |
| PV21 | Loading captures succeed while 25/50 ms fixture responses remain pending; subsequent explicit advances render their results.                                                                                                                  |
| PV22 | Historical: `service.emit` changed fixture state; missing assets/niri providers failed with exit 6. The retired absent-niri test's owned Unix sentinel observed zero live connections. The niri provider and its tests are no longer shipped. |
| PV23 | Historical: same-deadline asset completions retained request order; two declared niri events retained source order. Zero-latency completion was deferred until after the caller returned. The niri event case is no longer shipped.           |
| PV24 | Actual GPU captures cover placeholder→red→blue→clear, corrupt/remote failure, offscreen→visible, and real atlas exhaustion. Controlled delayed-worker tests reject stale replacement/clear completions.                                       |
| PV25 | Timeline reruns produce byte-identical PNGs and service traces, matching shipped font identities, UTC Date values, native performance time and seeded random values despite a Honolulu parent timezone.                                       |
| PV26 | Saved inspection enforces tree/frame/resource/configuration identities without GPU access. Complete artifact inventories verify 760 byte lengths and SHA-256 digests across the final v3/v4 matrices.                                         |
| PV27 | Held pacing/configuration and exact-time mismatch reject stale pixels with exit 5; the existing graphics boundary remains distinct from asset allocation failure.                                                                             |

Focused gates passed:

- `cargo test -p native-host preview::`: 54 passed, one opt-in test ignored.
- `cargo test -p native-host work::fixture::tests`: 16 passed.
- Native runtime `image_policy` and `images_end_to_end`: nine passed;
  `delayed_`: two passed; `live_package_images_`: one passed.
- UI bridge property tests: 13 passed; image build test: one passed;
  `pnpm typecheck` passed.
- Real-GPU `tests/preview/surface-configuration.test.mjs` and
  `tests/preview/final-input.test.mjs`: six passed.
- The documented `counter-virtual.json` command exits 0 with two captures and
  its count assertion passing. Successful final matrices include timeline,
  deferred zero-latency delivery, explicitly acknowledged errors, declared events
  and images. Expected failures retain their original category and stage;
  an unacknowledged injected error fails at `fixture-finalize`.

The actual Wayland host was also launched from `/tmp` with a package image and
`--watch`. PID/app-ID/title-verified compositor screenshots each contain 3,136
source-colored pixels: red in generation 1, blue after replacing the bytes at the
same asset path and publishing generation 2. Both generations dispose with every
resource count zero and the process exits cleanly. The compositor lacks grim's
capture protocol, so this check used explicitly authorized niri window screenshots
(which replace the clipboard), then restored the previous focused window.

Independent review identified and closed stale surface-configuration acceptance,
positional service assertion decoding, atlas readiness without upload, and
offscreen visibility accounting. A focused follow-up found no remaining concrete
stale-completion hole in the live package generation boundary. Test expectations
do not require an invisible image to remain unuploaded: native eager upload is
valid, while visibility and subsequent visible readiness are the contract.

## P4 — reviewed native visual comparisons

`native-host preview compare --baseline preview/baselines/counter --actual RUN_DIR
--policy preview/comparison/strict.json --out NEW_DIR` compares saved captures
without opening a graphics device or desktop connection. Exit codes are 0 for a
policy pass, 2 for invalid policy or incompatible inputs, 4 for visual differences,
and 8 for artifact integrity or output failures. `report.json` contains detailed
compatibility and pixel evidence; stdout and `result.json` contain the same bounded
summary. Incompatible pairs retain their original PNGs and receive no pixel score.

The baseline envelope binds the exact consumed artifact inventory, reviewed policy
bytes, source build ID and complete reviewed capture set. Every listed source
artifact is hashed, including unrelated and explicitly excluded captures. Unlisted
files, symlinks, FIFOs, malformed excluded PNGs and inconsistent frame/tree/image
identities fail rather than producing a visual pass. Explicit capture subsets report
their exclusions; the default requires identical, nonempty capture sets.

Comparison requires matching deterministic inputs, shipped fonts, native root
stylesheet, compiler/dependency identities, renderer/driver/color environment,
physical dimensions, scale and capture/validation/presentation times. Application
build IDs and whole host source-tree hashes remain reported rather than being
required equal: a changed application is the intended comparison subject, and the
host tree includes tooling/tests. This does not certify arbitrary host changes as
rendering-equivalent; review renderer/root-style revisions and the reported source
identity when changing native rendering behavior.

Exact and explicitly reviewed tolerant policies score all RGBA channels, including
RGB under zero alpha. Count and ratio budgets must both pass. Physical masks require
reasons, count overlapping pixels once, reject whole-image coverage and never hide
raw differences. No resizing, blurring, alpha exclusion or automatic threshold
selection is performed. Tolerant passes still report `exactEqual: false`, raw
differences and a diff PNG.

### Baseline review and required CI entrypoint

`preview/baselines/counter` contains the unchanged successful native run, policy and
review envelope. Both PNGs were visually inspected: Count/Doubled change from 0/0 to
1/2 after a native click, with the shared live-host 22px foreground typography and
24px padding. Review caught a preview-only missing root stylesheet before baseline
creation; preview now receives the same root defaults as the live host, and records
the actual stylesheet digest. The committed policy is exact with no masks.

To replace a baseline, first inspect a fresh successful run and its original PNGs,
then deliberately replace the saved run and policy and record their exact SHA-256
digests, build ID, capture set, reviewer, time and rationale in `baseline.json`.
There is no automatic blessing or policy relaxation command.

After building the counter and native host, run
`bun run preview:check --out NEW_ROOT`. It performs the native scenario and only then
compares against the reviewed baseline, retaining both child summaries. A graphics
failure is required failure, not a skip or comparison against historical output.
This baseline is intentionally bound to its recorded AMD/RADV/Mesa environment;
another driver or adapter must fail compatibility rather than silently normalize
pixels. No self-hosted CI runner was provisioned or exposed to untrusted PR code.

### Observed gates

- `cargo test -p native-host preview::`: 67 passed, one opt-in test ignored;
  `cargo build -p native-host` passed.
- `node --test tests/preview/*.test.mjs`: 20 passed, zero skipped. This includes
  genuine GPU baseline comparison, invalid Vulkan exit 7 without comparison,
  output failure exit 8, offline comparisons, integrity and review-policy failures,
  an explicitly allowed pending native image, and existing input/configuration gates.
- A throwaway actual counter build changed the button accent from `#314665` to
  `#b44665`. Both captures failed exact comparison with exit 4: 110,904 raw changed
  pixels each, maximum channel delta 131, bounds `(42,200,1316,85)`. Original and
  difference PNGs were inspected. A separately declared one-level tolerance also
  rejected this change.
- A subsequent actual build used `#324665`. The predeclared one-level tolerance
  passed with zero over-threshold pixels while reporting 110,232 raw different
  pixels per capture, maximum delta 1 and `exactEqual: false`. This isolated policy
  did not alter the committed exact baseline.
- A real run at logical width 801 produced 1402px-wide captures versus baseline
  1400px. Comparison exited 2, retained original images, and scored neither pair.
- Independent read-only review identified excluded-capture validation and allowed
  pending-resource semantics as false-green risks. Both were corrected; focused
  follow-up confirmed the boundaries and exact consumed-inventory binding.

Local proof artifacts remain under `target/preview-p4-*`; throwaway application and
policy inputs were removed after verification. These are native offscreen pixel
checks, not compositor transparency, IME or desktop-shell certification.

## P5 — atomic publication watch and generation-safe lifecycle

Run the managed builder and preview observer together:

```sh
bun run preview --app counter --watch \
  --profile preview/profiles/default.json \
  --fixture preview/fixtures/empty.json \
  --scenario preview/scenarios/counter-virtual.json \
  --out target/counter-watch
```

Watch requires the builder's atomic generation symlink, not mutable JS/CSS files.
It resolves and validates a complete package, rechecks the publication after
validation, and pins that directory for an ordinary one-shot run controller.
Pending publications coalesce; a superseded active worker is canceled and reaped
before the next worker starts. Forced cancellation explicitly leaves clean JS
teardown unverified. Linux workers have their own process group and a parent-death
signal, including a startup parent-identity check.

The builder atomically publishes `.zgui-build-status.json` beside `dist`. A failed
compilation has an attempt UUID and diagnostic, not a fabricated package build ID.
Its previous publication identity prevents stale status from blocking a newer
manual publication. Status files and their temporary names do not trigger rebuilds.
Each run revalidates the configured profile/fixture/scenario paths and saves the
inputs it actually consumed; edits to those files alone are not build notifications.

Watch stdout is bounded JSONL. `events.jsonl` retains the complete published events,
and one atomic `index.json` contains `latestAttempt` and `lastSuccess`, avoiding torn
multi-file pointer updates. Control `runId` and native worker `nativeRunId` are
distinct and both retained. Failed builds have no current capture; previous success
is explicitly historical. A superseded completion cannot promote either index.
Run directories are immutable after their owner completes.

`--max-runs` defaults to 64 and accepts 1–1024. The observer stops rather than deleting
evidence when the run limit is reached; events are limited to `16 * maxRuns + 64`,
8 KiB each, and each native run retains its profile output limit. Start a new output
directory to continue. A run-limit exit reflects the latest attempt, not an older
successful worker. SIGINT/SIGTERM stop the controller and join its owned run.
An unread stdout pipe fails after a 250 ms delivery deadline; stderr backpressure
also becomes an output failure. Descriptor flags are restored in reverse order,
including shared `2>&1` descriptors.

### Real native surface recreation

Scenario operation `{ "id": "resume", "op": "surface.recreate", "surface": "main" }`
requires `surface.recreate`. It performs actual headless Harness suspend/resume,
not a document reset presented as compositor output testing. The old session
disposes before new native IDs are bound. Probe, capture and accessibility ownership
are retired; only current `Runtime.windows` selects main, never retained headless
offscreen entries. Scale and logical geometry are reapplied to the replacement.

Session generation increments while the run clock, RNG and fixture history continue.
Retirement cancels only that generation's fixture registrations; run-unique request
IDs prevent collisions with earlier ledger entries. Whole-run fixture disposal
belongs to `Driver::finish`, not every session shutdown. Evidence filenames and
recovery matching include generation, and final run metadata reports the current
generation. Earlier complete captures remain inspectable without certifying stale
pixels for the replacement surface.

### Named Shell surfaces and scripted outputs

Use `"pureShell": true` in a preview profile to exercise a package's `mount()`
without opening `main`. Declare `capabilities.shellSurfaces` or `capabilities.sessionLock`.
Omit `pureShell` to keep the normal `mount(root)` startup and optionally open
named Shell surfaces in the same Session.

The preview monitor list starts empty and is independent of the workstation
display setup. Replace it with `monitors.set` (inferred capability
`surface.multiple`). This example supplies a 1200×800 physical output at scale 2,
so a `LayerSurface` stretching horizontally without margins uses 600 logical pixels:

```json
{
  "id": "plug",
  "op": "monitors.set",
  "monitors": [
    {
      "id": "left",
      "name": "Left",
      "x": 0,
      "y": 0,
      "width": 1200,
      "height": 800,
      "scale": 2,
      "refreshRateMilliHz": 60000
    }
  ]
}
```

Geometry (`x`, `y`, `width`, `height`) is in physical pixels; refresh rate is
optional. Use a new list to change geometry or remove specific outputs, and
`"monitors": []` to remove them all. The SDK's `useMonitors()` observes the
ordered snapshot; neither hotplug nor surface creation advances virtual time.

Select a mounted Shell surface by its `LayerSurface`, `PopupSurface`, or
`SessionLockSurface` name on `capture`, input, assertion, and surface
configuration steps. A missing surface fails with
`E_SURFACE_NOT_FOUND`. Input positions are in the selected surface's local logical
pixels. Named pointer and wheel steps respect that surface's input region;
an explicit coordinate outside it records no dispatch. Preview does not
composite or route through to another desktop surface.

Use `{"id":"dismiss-menu","op":"popupDone","surface":"menu"}` to simulate
compositor dismissal of a mounted `PopupSurface`. This infers `surface.multiple`
and does not advance virtual time. The named popup's `onDismiss` runs; any
descendant popup regions close without receiving their own callbacks. A missing
or non-popup target fails with `E_SURFACE_NOT_FOUND`.

For session-lock scenarios, require `session-lock` and use `monitors.set` to
exercise coverage on output arrival and removal. The offscreen compositor
accepts by default; `"services": {"sessionLock": {"outcome": "finished"}}`
refuses the next request. Readiness records lock state, covered outputs,
fallback outputs, and the explicit unlock count. The `sessionLock` record in
`services.json` is sampled after JS retirement, before the fixture platform
is discarded. A retained compositor lock is not a leaked session resource.

Named layer `surface.recreate` replaces its mount region and any descendant
popup regions, preserving the Session generation and unrelated surfaces.
Named captures store evidence in
`captures/surfaces/<surface>/<captureId>.*`; `main` keeps
`captures/<captureId>.*`. To inspect these artifacts use `preview inspect --surface
<name> --capture <captureId>`. Comparison and baseline review carry both fields.

### Observed gates

- Native preview Rust tests: 68 passed, one opt-in watchdog fixture ignored.
  Fixture tests: 17 passed; live work tests: eight passed. Session tests: all 14
  passed across the initial run and the focused corrected contract check.
  `cargo build -p native-host` passed without warnings.
- Build compiler/watcher tests: ten passed, including real compilation failure,
  recovery and failed status publication.
- Four public watch tests passed: real compile failure and recovery; frozen native
  input canceled/reaped before the newest coalesced build starts; a late successful
  worker excluded after a newer failed attempt; and an unread stdout pipe failing
  promptly. Process coordination uses only owned processes.
- The 20 existing preview integration tests passed, including exact baseline CI,
  graphics/output failure categories, offline integrity and native input/configuration.
- The new actual-GPU lifecycle scenario passed: replacement native IDs, preserved
  clock/RNG, retired timer/request/subscription cancellation, four unique fixture
  registrations, continued fixture events, coherent old/new captures, native input
  and zero resources at both retirement and final teardown.
- `target/preview-p5-surface-proof` contains three 800×600 native captures at scale 2.
  Visual inspection confirms Count/Doubled reset from 1/2 to 0/0 on recreation and
  return to 1/2 through input on the replacement. Surface IDs change 1→2, document
  IDs 0→1, generation 1→2, and time remains 40,000 µs across recreation.
- The actual unlocked Wayland `reload_tests::native_full_reload` gate passed:
  failed evaluation preserves the active UI, 16 replacements retire to zero,
  failed mount exposes the native Rust fallback, Retry recovers, and stale deliveries
  have no effect. The prior focused window was restored.
- Independent review closed startup-error masking, superseded-success promotion,
  blocked-output cancellation and shared-descriptor restoration issues.

The counter baseline was deliberately refreshed for the P5 capability/evidence
revision only after both PNG files proved byte-identical to P4; the native image
was visually inspected again. Exact policy bytes, application build ID, typography
and expected Count/Doubled behavior remain unchanged. Desktop output removal, real
IME, compositor blending and live service parity remain separate P6 evidence.

## P6 — real Wayland/niri desktop verification

These are historical unlocked desktop observations, not headless surface-role
declarations or a current compositor support requirement. The niri panel, service
and compositor-specific gates described here are no longer shipped; the original
results are retained rather than erased. The native/compiler baseline for these
runs is documented above and in [baseline.md](baseline.md). The compositor
reported `26.04 (v26.04-298-gad20c318-modified)`; the installed niri CLI reported
`26.04 (8ed0da4)`. DP-1 normally used scale 1.75. The then-live service used direct
Unix-socket IPC rather than that CLI's incompatible event parser.

Explicit authorization covered PID-verified owned-window keyboard/IME input,
clipboard replacement without reading its old contents, temporary DP-1 scale
changes and off/on, and full-output captures in the controlled test workspace.
Output geometry and prior focus were restored after the scenarios; the editing
scenario also restored the prior IME engine/activation state. The original
clipboard was never read or restored. Screenshots themselves replace the
clipboard under the same authorization.

### Native editing and occlusion

The rebuilt native test binary passed the editing gate below. The associated
occlusion run used the now-retired `async_tests::native_async_delivery` gate;
that niri-specific command is no longer available.

```sh
ZGUI_PLATFORM=wayland ZGUI_EDITING_REPLACE_CLIPBOARD=1 \
  ZGUI_EDITING_WAYLAND_IME=1 cargo test -p native-host --bin native-host \
  editing_tests::native_text_editing -- --ignored --exact --nocapture
```

The editing run used the compiled test executable directly to retain its exact
PID. A temporary Wayland virtual-keyboard client sent F24 only after verifying
the owned window and focus, supplying the real serial required for clipboard
ownership. Controlled caret, native copy/cut/paste, undo, multiline movement,
read-only/disabled behavior, focus and semantic IDs passed. Actual fcitx/Rime
preedit (`n`, then `ni ha o`) committed `你虾哦`; the native controlled value and
UTF-8 selection `9:9` matched the visible field. Both retired generations had zero
resources.

The occlusion gate moved only its owned window. While genuinely occluded, the
timer and Rust service completed but the frame callback remained pending.
Restoring visibility produced the frame callback, Promise-driven layout and
visible updated row. Final teardown reported four frames and zero resources.

### Placement, scaling and output lifecycle

The following is the historical invocation of the removed niri layer gate, not
a command available in the current test suite:

```text
ZGUI_PLATFORM=wayland ZGUI_SHELL_LIFECYCLE=1 \
  cargo test -p native-host --bin native-host \
  shell::proof::real_panel_layer -- --ignored --exact --nocapture
```

- Real niri layer inventory reported `dev.zgui.js.panel`, DP-1, `Top`, keyboard
  `None`. Protocol requests used top/left/right anchors, zero margins and a 360px
  exclusive zone. The existing compositor side bar also reserves horizontal space.
  A normal owned window's settled height changed from 1220 to 860 logical pixels.
  The panel did not acquire keyboard focus.
- DP-1 changed 1.75→2→1.75. Moving the owned normal window DP-1→DP-2→DP-1 while
  DP-1 used scale 2 produced fractional-scale preferences 240→210→240
  (protocol units of 1/120), with IPC confirming its destination output.
- Turning DP-1 off produced `logical: null` and retired generation 1. Turning it
  on created document 1 instead of document 0, mounted generation 2 and delivered
  fresh real workspace/window rows with exactly one subscription. The gate passed
  in 9.27 seconds; both generations ended with all resource counts zero. P5's
  separate stale-handle/callback tests cover the retired API ownership boundary.

Protocol traces must go to regular files or continuously drained sinks. An
earlier pipe-backed instrumentation attempt timed out during output removal;
its trace is retained, not counted as a passing lifecycle run. Stopping the owned
clients restored responsiveness. The regular-file run above passed. Likewise,
`panel-placement.png` was taken before presentation and is rejected as visual
evidence; `panel-presented.png` waits for compositor presentation feedback.
An initial native frame log is not a compositor mapping/presentation barrier.

### Actual compositor transparency

Two temporary TSX packages used ordinary production `native-host --bundle`,
`--layer-config` and, for the background only, `--watch`. An opaque top-layer
background covered the test area; a transparent overlay-layer foreground held a
300×200 logical-pixel `#ff000080` rectangle and an opaque red reference. Both
surfaces used four anchors, zero margins/exclusive zone and keyboard `none`.
An atomic package publication changed only the background from green to blue.
The foreground retained its PID and generation throughout.

Full-output captures after presentation showed:

| Sample           | Green background RGB | Blue background RGB |
| ---------------- | -------------------- | ------------------- |
| Transparent area | 0, 255, 0            | 0, 0, 255           |
| Half-alpha red   | 128, 127, 0          | 128, 0, 127         |
| Opaque reference | 255, 0, 0            | 255, 0, 0           |

These are compositor-composited pixels, not an offscreen alpha-channel assertion.
They describe this desktop's observed blending path, not a cross-compositor color
management guarantee.

### Live services, wakeups and idle limits

The niri service and panel behavior in this subsection is historical; neither
integration is shipped by the current general-purpose UI library.

The ordinary panel process, without watch mode, displayed real niri rows.
Opening an owned counter window caused a new presentation and visible
`Windows (13)` without injected UI input; closing it caused another presentation.
A separate process with an absent `NIRI_SOCKET` displayed `Disconnected`, the
actual `Io: No such file or directory` error and `No snapshot received`, rather
than a successful empty desktop. It exited normally with zero resources.

The five-second observation measured six CPU ticks at 100 Hz and 23 additional
Wayland frame requests. It is **not a pure idle benchmark**: the protocol trace
records real pointer enter/motion starting at 17:14:34.198581. There were no frame
requests between 17:14:29.644859 and 17:14:34.199868, approximately 4.55 seconds;
the next request follows that input. The complete mixed-activity panel run ended
with 118 native frame passes and zero resources. No continuous artificial redraw
loop was observed during the quiet interval; these numbers do not establish a
general CPU or power budget.

### Coverage and retained evidence

PV37's real editing/IME path and PV39's compositor blending, fractional scale and
output transitions passed. PV38's placement/focus/exclusive-zone portion passed.
That historical run did not exercise layer-parent xdg-popups. The separate
offline `popup_protocol` test covers layer and nested popup protocol requests;
`tests/preview/popup-surface.test.mjs` covers named captures and dismissal in
the headless runtime. Neither certifies desktop-specific input or placement.
PV40 historically passed for the then-implemented niri and async asset-service
paths and observed occlusion/wake behavior. The niri service and its gate have
since been retired. Live audio and notification providers were not implemented
and were **not passing cases**; preview fixtures do not certify those integrations.

Local, untracked evidence is under `target/preview-p6-desktop`: `verification.json`
records accepted PNG hashes, observed samples and limitations; `packages/`
retains the three immutable compiled blending inputs, source maps and build-info;
the named `.log` files retain real protocol/lifecycle observations. The desktop
images and logs may contain window titles and are deliberately not committed.
Temporary source packages and native input helper sources/binaries were removed.

## StyleX native integration gate

The StyleX gate builds the real example package and uses the production schema-v2 loader, QuickJS session, native bridge, stylesheets, fonts, images, layout, and GPU paint:

```sh
bun run build --app stylex
cargo build -j 4 -p native-host --locked --offline
bun test --isolate --timeout=30000 tests/preview/stylex.test.mjs
```

The scenarios exercise extracted static CSS without an injector; imported tokens and nested/default/removed themes; conditional classes and dynamic custom properties; SDK root forwarding and base/StyleX sheet ordering; native focus, hover, and viewport-media conditions; invalid dynamic values and alias conflicts; keyframes with the explicit reduced-motion host preference; package image/font readiness; and consumed build/style revision evidence. Every target is a stable `testId`, never a generated StyleX hash. Nested Disclosure uses an explicit 300,000-byte stack bound with entry duration checked below 25 ms; Gallery fuzz uses a 240 KiB stack cap and a 30 ms entry bound. Both limits are asserted through `metrics.json`.

Compiler/watch, native props, schema/digest, preflight, selector/cascade, reload, provenance, and diagnostic suites cover the remaining SX01–SX19 contracts. Browser-root RTL output is an SX11 rejection case rather than an SX10 success claim. SX17 intentionally uses native package images and manifest-registered fonts, not CSS URLs or `@font-face`. SX20 remains deferred with CSS-only activation outside v1.

Observed 2026-11-18: the complete Preview suite passed **84/84 scenarios**, including all six StyleX scenarios. `pnpm check` passed 230 build tests plus Rust tests/Clippy, and `pnpm test:vendor` passed the required vendored suites. Both independent reviews ended with no remaining findings. See [StyleX authoring](stylex.md), the [decision record](stylex-integration/decision-record.md), and the [complete verification report](stylex-integration/verification.md). No historical screenshot is accepted as a current run.
