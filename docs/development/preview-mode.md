# Preview Mode Development Specification for zgui, rquickjs, and TSX

**Revision:** 1.0  
**Date:** 2026-11-14  
**Audience:** The implementing developer, coding agents, and UI reviewers  
**Suggested repository location:** `docs/development/preview-mode.md`  
**Companion:** `ZGUI_RQUICKJS_TSX_DEVELOPMENT_SPEC.md`

Build a repeatable native preview runner that allows an agent to compile a UI change, run a short interaction scenario, inspect a screenshot and structured state, and fix the next concrete failure. The preview must execute the real application bundle through rquickjs and the real zgui layout and rendering pipeline.

This is an implementation specification. The `native-host preview` commands, JSON schemas, readiness APIs, selectors, and artifact formats below are project functionality to implement. They are not existing commands supplied by zgui, rquickjs, Solid, or Preact. Upstream capabilities and integration gaps are identified in Section 2.

The selected stack remains **zgui + rquickjs + ahead-of-time compiled TSX**. Solid Universal remains the initial frontend from the companion specification. Preview must depend on the native host contract, so a future Preact adapter can use the same runner without a second preview implementation.

**Current scope:** Preview supports the general-purpose UI library and its
resource services. Runtime platform and compositor support is determined by
upstream zgui. The former niri service, panel example, fixture provider and
compositor-specific gates are no longer shipped; historical verification records
do not establish a current compositor baseline.

## Contents

- [1. Goals and first usable milestone](#1-goals-and-first-usable-milestone)
- [2. Upstream baseline and integration gaps](#2-upstream-baseline-and-integration-gaps)
- [3. Architecture and ownership](#3-architecture-and-ownership)
- [4. CLI and agent workflow](#4-cli-and-agent-workflow)
- [5. Profiles and reproducibility](#5-profiles-and-reproducibility)
- [6. Fixture services and assets](#6-fixture-services-and-assets)
- [7. Virtual time and bounded readiness](#7-virtual-time-and-bounded-readiness)
- [8. Scenarios, selectors, and native input](#8-scenarios-selectors-and-native-input)
- [9. Screenshot and frame evidence](#9-screenshot-and-frame-evidence)
- [10. Tree inspection and source attribution](#10-tree-inspection-and-source-attribution)
- [11. Results, diagnostics, and exit codes](#11-results-diagnostics-and-exit-codes)
- [12. Visual comparisons and baseline review](#12-visual-comparisons-and-baseline-review)
- [13. Reload and watch mode](#13-reload-and-watch-mode)
- [14. Performance and feedback cost](#14-performance-and-feedback-cost)
- [15. Real desktop validation](#15-real-desktop-validation)
- [16. Implementation phases](#16-implementation-phases)
- [17. Acceptance matrix](#17-acceptance-matrix)
- [18. Agent execution rules and handoff prompt](#18-agent-execution-rules-and-handoff-prompt)
- [19. Future extensions](#19-future-extensions)
- [20. Source index](#20-source-index)

## 1. Goals and first usable milestone

### 1.1 Required outcome

An agent must be able to answer five questions from one run:

1. Did this exact application build mount successfully?
2. Did native input reach the intended control?
3. Did the expected state and geometry appear?
4. What did the actual native renderer present?
5. If the run failed, what is the smallest useful next investigation?

The runner must produce evidence that is small enough for routine use and complete enough for debugging. A screenshot alone cannot explain a missed click or a stale bundle. A tree dump alone cannot reveal incorrect typography or clipping.

### 1.2 First usable vertical slice

Implement the smallest complete loop before adding a large protocol surface:

1. Build the existing TSX counter example.
2. Load its actual package in a fresh rquickjs session.
3. Mount it through the production UI bridge.
4. Render with real zgui fonts, shaping, layout, and offscreen wgpu.
5. Save an initial PNG.
6. Resolve the increment button by `testId`.
7. Deliver native pointer input.
8. Assert that the displayed count changed.
9. Save a second PNG and a compact JSON result.
10. Dispose the application and release the runtime correctly.

This slice is useful before images, multiple surfaces, animation timelines, or persistent agent sessions exist. Unsupported later capabilities must be reported explicitly.

### 1.3 Design decisions

| Decision | Requirement |
| --- | --- |
| Same application | Load the same manifest, JS, CSS, assets, SDK, and bridge used by the interactive host. |
| Same renderer | Use zgui's actual native pipeline for pixel evidence. |
| Fixture services | Instantiate mock providers by default; never fall through to live services. |
| Scripted time | Use one virtual application clock for supported deterministic runs. |
| Real watchdog | Use a separate real monotonic clock to bound execution and GPU/worker waits. |
| Native input | Exercise hit testing, focus, default behavior, and event cancellation. |
| Stable targeting | Prefer unique test IDs, then verified semantic selectors. |
| Coherent evidence | Associate pixels, tree, layout, build, session, and presentation identities. |
| Bounded work | Every run, step, job pump, resource wait, and teardown has a limit. |
| Small responses | Print a bounded machine result; keep detailed evidence in files. |
| Framework independence | Put preview below the Solid/Preact adapter boundary. |
| Early usefulness | Ship one-shot execution before watch, RPC, MCP, or an inspector UI. |

### 1.4 Scope boundaries

Preview defaults to one logical surface named `main`. A shell-capable package can add named Shell surfaces or start a pure-shell Session without `main`. All surfaces still use one locally built application bundle, packaged resources, and one Session generation.

It does not establish compositor protocol correctness, operating-system service integration, arbitrary browser compatibility, or hostile-code isolation. These boundaries follow the existing host architecture. The one-shot worker process is useful for reliable cancellation and failure reporting; it is not a claim of OS sandboxing.

## 2. Upstream baseline and integration gaps

### 2.1 Dependency baseline

Use the same application dependencies as the companion specification:

| Component | Audited baseline |
| --- | --- |
| zgui | `80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a` |
| rquickjs | `73fcd6062ee615aeb2a4675cd32be67a56e93973` |
| QuickJS-NG engine submodule | `6d46d07d04041b40f4f49eaa7fdebe44c314c699` |
| Solid/compiler source baseline | `99714d4e2875d2d7ef8023e82f4ef6aa63ff0752` |

Preserve the companion document's package-resolution gates. Record actual lockfile versions and source revisions in every run. Do not silently upgrade dependencies while implementing preview.

### 2.2 Existing building blocks

| Existing upstream capability | Use in this project | Boundary |
| --- | --- | --- |
| Headless `Harness` | Deliver events, pump work, advance virtual time, inspect deadlines | It is a test driver, not the proposed CLI. |
| `VirtualClock` | Native timers, frame scheduling, and animations | JS and service adapters must use the same clock. |
| Offscreen wgpu builder and readback | Actual pixel capture | A usable graphics adapter is still required. |
| `scale_capture` example | Reference integration for runtime, fonts, renderer, scale, and capture | It is a fixed Rust gallery, not a TSX runner. |
| `FrameProbe` and public window queries | Collect frame-aligned diagnostics | An agent-facing schema and inspector must be implemented. |
| Headless accessibility logs | Observe native semantic updates | Updates are incremental, not complete snapshots. |
| Shipped-only font registration | Reproducible font selection | The project must supply and verify its own font files. |
| Scene/view testkits | Fast structural tests | Their recording/fixed-metric paths do not constitute native screenshots. |

The headless and capture integration is demonstrated by zgui's own source. Keep structural test evidence and real rendered-pixel evidence separately identified. [Z1] [Z2] [Z3] [Z4] [Z5] [Z6] [Z7]

### 2.3 Explicit implementation gaps

The following are project work and must have acceptance gates:

- Loading TSX application packages into the preview host.
- Test IDs, semantic selectors, and a JSON tree projection.
- PNG encoding with correct channel and alpha handling.
- A shared virtual clock policy for JS, native UI, and mock services.
- Native asset readiness and failure reporting.
- Structured scenario execution, result files, and comparison reports.
- Source attribution from generated JS back to TSX.
- Generation-safe reload and surface rebinding.

In particular, the audited native image loader performs asynchronous probe/decode work and exposes no public all-assets-ready contract in the examined interfaces. Add a narrow public status/probe seam or a documented integration patch before claiming image-complete captures. Reading image bytes in advance or waiting a fixed number of frames does not prove that native decoded content has been attached. [Z8]

### 2.4 Lessons from AntiWebUI and Shoji Bar

AntiWebUI demonstrates headless rendering and driven input snapshots. Shoji Bar turns those capabilities into a convenient preview command. These are useful references for making UI iteration accessible from a terminal. [A1] [A2] [A3]

Shoji's documented preview can still operate real network, Bluetooth, audio, and clipboard services. Its frame stepping also uses real time. This project instead defines fixture services and an explicit virtual-time contract, because an agent needs reproducible actions and predictable side effects. [A3] [A4]

Do not copy ambiguous frame-count semantics: this specification's `capture.sequence.count` is the number of output images, including its initial image.

## 3. Architecture and ownership

### 3.1 Shared production path

The implemented deterministic preview path uses a supervised process, not a second UI runtime:

~~~mermaid
flowchart TB
    Controller["preview CLI controller: deadline and process supervision"]
    Inputs["Application package, profile, asset fixtures, scenario"]
    subgraph Worker["One-shot worker process"]
        Runner["Scenario runner"]
        Input["Native input adapter"]
        Session["Production Session: QuickJS, Solid and UI bridge"]
        Fixtures["Asset-only FixtureRuntime: state, requests, call trace"]
        Clock["Shared virtual application clock"]
        Native["Upstream zgui: headless platform and real renderer"]
        Capture["Frame-aligned GPU pixels and native tree"]
        Runner --> Session
        Runner --> Input
        Runner -->|Advance virtual time| Clock
        Runner -->|Emit fixture state and assert calls| Fixtures
        Input --> Native
        Session <--> Native
        Fixtures <--> Session
        Clock --> Session
        Clock --> Native
        Native --> Capture
    end
    Controller -->|Start and supervise| Runner
    Inputs --> Runner
    Capture --> Artifacts["Run artifacts: PNG, trees, traces and results"]
    Fixtures --> Artifacts
    Controller -->|Publish final result and inventory| Artifacts
    Artifacts --> Inspect["preview inspect: saved native-tree queries"]
    Artifacts --> Compare["preview compare: offline pixel comparison"]
    Baseline["Reviewed baseline and comparison policy"] --> Compare
~~~

The diagram shows the default virtual-clock profile. Wall-clock profiles are explicitly nondeterministic. The controller enforces a real elapsed-time deadline even when application time is virtual.

Preview changes the platform selection, asset-service provider, clock, and evidence sinks. It reuses the production package, `Session`, UI bridge, and upstream renderer. GPU capture still requires a supported graphics adapter; saved-artifact inspection and comparison do not open a graphics device.

Fixture services cover packaged text assets only, not compositor state or live WM integration. Native images continue through the real resource/decoder/renderer path. Platform and compositor support remains upstream zgui's responsibility; offscreen evidence does not certify real-desktop focus, IME, or compositor behavior.

Implementation entry points: [process supervisor](../../crates/native-host/src/preview/supervisor.rs), [run orchestration](../../crates/native-host/src/preview/run.rs), [native driver](../../crates/native-host/src/preview/driver.rs), [asset fixture runtime](../../crates/native-host/src/work/fixture.rs), and [saved-run comparison](../../crates/native-host/src/preview/compare.rs).

A preview fixture may deliberately choose a component or route through a documented application launch parameter. Record those parameters. Do not add hidden branches that change layout or event behavior merely because preview is running.

### 3.2 Proposed implementation modules

Start with modules where possible; separate crates only when dependency ownership warrants it.

| Location | Responsibility |
| --- | --- |
| `crates/native-host/src/preview/` | CLI dispatch, one-shot process supervision, run orchestration |
| `crates/native-host/src/preview/driver.rs` | Headless harness and virtual-time stepping |
| `crates/native-host/src/preview/capture.rs` | Renderer wrapper, readback, PNG conversion |
| `crates/native-host/src/preview/inspect.rs` | Frame-aligned native/semantic tree projection |
| `crates/native-host/src/preview/scenario.rs` | Validated actions, selectors, assertions |
| `crates/host-services/src/mock/` | Typed fixture providers and call recording |
| `crates/js-runtime/` | Existing runtime ownership, jobs, exceptions, deadlines |
| `crates/ui-bridge/` | Existing input, nodes, callbacks, test metadata, generation checks |
| `packages/ui/` | Native `testId` declarations and optional ready-marker API |
| `packages/build/` | Existing compilation, source maps, atomic bundle publication |
| `preview/profiles/` | Viewport, fonts, clock, graphics, and limits |
| `preview/fixtures/` | Typed service state and declared resources |
| `preview/scenarios/` | Ordered reproducible interactions and assertions |
| `preview/baselines/` | Reviewed image/semantic baselines and comparison metadata |
| `schemas/preview/` | Versioned closed JSON schemas and example fixtures |

### 3.3 Ownership rules

All rquickjs entry, native mutation, event dispatch, scenario actions, and tree inspection run on the owner UI thread.

Worker tasks return owned data with run/session/request identifiers. They never carry JS values, persistent callbacks, zgui node references, or borrowed contexts. Release registry and native borrows before invoking JavaScript.

Use one fresh application session per one-shot run. A logical surface name is mapped to the current native surface and document. Never expose raw 64-bit native node IDs as JS numbers or stable external selectors.

The capture probe is passive. It must not update application signals, request perpetual redraws, or keep otherwise idle UI active.

### 3.4 Supervised one-shot worker

The CLI controller should start a worker for each run and enforce a real deadline. The worker owns the UI runtime and GPU. The controller owns the run directory and final result publication.

This contains a native panic or blocked GPU operation within one run. If the worker exits unexpectedly, the controller writes a structured failure using the last complete evidence record. Killing a worker ends that run; it does not produce a successful teardown or a valid fresh screenshot.

Use a small internal control channel for progress and completion. Application stdout must not contaminate the public JSON result.

## 4. CLI and agent workflow

### 4.1 Canonical command surface

All commands below are proposed project commands.

| Command | Purpose | Phase |
| --- | --- | --- |
| `native-host preview doctor` | Report build, graphics, fonts, and supported capabilities | P0 |
| `native-host preview run` | Execute one package/profile/fixture/scenario | P1 onward |
| `native-host preview inspect` | Query saved tree/evidence without loading the runtime | P2 |
| `native-host preview compare` | Compare complete run artifacts with reviewed baselines | P4 |
| `native-host preview watch` | Rebuild and rerun using the same one-shot engine | P5 |
| `native-host preview serve` | Persistent protocol adapter | Deferred |

The full deterministic loop, available after P3, uses the companion specification's proposed workspace scripts:

~~~sh
cargo build -p native-host
bun run typecheck
bun run build --app counter

./target/debug/native-host preview doctor --profile preview/profiles/default.json

./target/debug/native-host preview run --bundle apps/examples/counter/dist --profile preview/profiles/default.json --fixture preview/fixtures/empty.json --scenario preview/scenarios/counter.json --out artifacts/preview/counter-001
~~~

For the earlier P1/P2 loop, use the explicit wall-clock profile and static scenario defined in Sections 5.5 and 8.9. Fixture-only provider construction and a real watchdog are required from P1; P3 adds the richer timing/resource capabilities.

Do not rebuild Rust for an ordinary TSX or CSS edit when the host and bridge have not changed. The application build must remain atomic and must include compatible JS, CSS, assets, manifest, and source maps.

### 4.2 Run options

| Option | Contract |
| --- | --- |
| `--bundle PATH` | Required application package directory containing `manifest.json`. |
| `--profile PATH` | Required validated preview environment profile. |
| `--fixture PATH` | Required service/resource fixture; an explicit empty fixture is allowed. |
| `--scenario PATH` | Required ordered scenario document. |
| `--out PATH` | Required new or empty run directory. |
| `--width N`, `--height N` | Optional logical-pixel overrides, reflected in the resolved profile. |
| `--scale N` | Optional finite positive scale override. |
| `--theme NAME` | Optional supported theme override. |
| `--timeout-ms N` | Optional real run deadline, subject to the host's maximum. |
| `--format json` | Machine result; the default for preview commands. |

CLI overrides take precedence over profile values. Store the complete resolved profile and its digest. Reject unknown options, schema versions, actions, and unsupported capabilities before mounting.

Do not introduce an implicit live-services option into this command. A later live integration mode must be explicitly named and identified in its artifacts; it must never produce a deterministic fixture baseline.

### 4.3 Output rules

For one-shot commands, stdout contains exactly one JSON result object followed by a newline. Send bounded progress and human-readable logs to stderr. Store detailed application logs in `diagnostics.jsonl`.

Never put PNG bytes, base64 images, full source files, or an unbounded tree on stdout. The result identifies relative artifact paths, failure codes, and a small summary.

Use relative artifact paths within the run directory. The runner may also report the absolute run directory for the local caller. Output is written through temporary files and atomic rename where supported; publish `result.json` last.

Refuse to reuse a nonempty run directory in version 1. This prevents stale images from an earlier build being mistaken for new evidence.

### 4.4 The agent's short loop

1. Run the affected type check and application build.
2. Execute the smallest scenario that covers the change.
3. Read the compact result.
4. On a runtime failure, inspect the source-mapped diagnostic first.
5. On a selector or assertion failure, query the relevant saved subtree.
6. Open the relevant PNG for visual review.
7. Make one focused correction and rerun the same scenario.
8. Broaden the test set only when the change affects shared behavior.

A successful exit verifies the configured assertions. It does not mean an agent has visually reviewed the image, that every supported profile passed, or that the UI works under a real compositor.

## 5. Profiles and reproducibility

### 5.1 Profile example

~~~json
{
  "schemaVersion": 1,
  "id": "desktop-800x600-175",
  "surface": {
    "name": "main",
    "width": 800,
    "height": 600,
    "scale": 1.75,
    "refreshRateMilliHz": 60000,
    "opaque": true,
    "background": "#17191fff"
  },
  "theme": "dark",
  "locale": "en-US",
  "direction": "ltr",
  "timeZone": "UTC",
  "clock": {
    "mode": "virtual",
    "epochMs": 1794614400000,
    "startUs": 0
  },
  "random": {
    "seed": 12345
  },
  "fonts": {
    "mode": "shipped-only",
    "manifest": "fonts.json"
  },
  "assets": {
    "mode": "package-only",
    "readiness": "required-visible"
  },
  "limits": {
    "maxRunWallMs": 15000,
    "maxStepWallMs": 3000,
    "maxJsEntryWallMs": 25,
    "maxJobsPerPump": 1000,
    "maxPumpWallMs": 50,
    "maxSettleTurns": 64,
    "maxFramesPerStep": 120,
    "maxVirtualDurationMs": 60000,
    "maxOutputBytes": 33554432
  }
}
~~~

The numbers are initial development defaults, not measured performance guarantees. Make them configurable within implementation limits and record any override. The fixed epoch is 2026-11-14 00:00:00 UTC.

Relative paths in a profile resolve against its own directory. Bundle assets resolve against the bundle root. Fixture data refers to declared bundle asset IDs, not arbitrary filesystem paths. Profile-declared fonts are an explicit additional resource root. Canonicalize and validate each permitted root independently; the output directory is not a resource search path.

### 5.2 Profile schema rules

Version 1 accepts one `main` surface. Width and height are positive integer logical pixels; scale is a finite positive number. Impose a documented physical-pixel allocation limit before creating textures. Refresh rate is a positive integer in millihertz.

Set `surface.background` to an eight-digit `#RRGGBBAA` color; `surface.opaque: true` requires `ff` alpha. Use `opaque: false` and `#00000000` for a clear background. Preview layer and popup targets follow the profile opacity; session-lock surfaces stay opaque.

The runner must validate locale/time-zone support. Do not silently claim a requested locale when the runtime lacks the required implementation. The initial deterministic profile supports UTC and explicit locale-aware SDK formatters; arbitrary system locale discovery is outside that profile.

All numerical time inputs must be finite, nonnegative, and exactly representable within the documented range. Internally use integer durations; do not accumulate repeated floating-point frame increments.

### 5.3 Coordinates and physical dimensions

Compute physical extents as:

~~~text
physicalWidth  = floor(logicalWidth  * scale + 0.5)
physicalHeight = floor(logicalHeight * scale + 0.5)
~~~

Reject overflow and zero physical dimensions. Report requested logical size, actual physical size, and effective logical size `physical / scale`; fractional rounding can make the last value slightly different.

The default profile above produces 1400 by 1050 physical pixels. The supported profile set should include scales 1.0, 1.25, 1.5, 1.75, and 2.0, plus at least one odd physical width to expose row-stride mistakes.

### 5.4 Fonts, text, and random values

Create a shared `Fonts::shipped_only()` collection, register all declared font bytes before mounting, and use it for metrics, shaping, and rasterization. An empty shipped-only collection is not a useful default: upstream documents that it draws no text. [Z6]

The font manifest is a closed object with `schemaVersion: 1`, a nonempty `faces` array, and `fallbackOrder`. Each face contains `family` (nonempty alias), `path` (relative to the font-manifest directory), `sha256` (64 lowercase hexadecimal digits for the actual bytes), and `licensePath` (a local license/provenance file). `fallbackOrder` is an ordered array of declared family aliases. Apply the declared order through the shared text/font configuration; if it cannot be enforced at the selected baseline, fail that profile rather than recording an order that was ignored.

Include the CJK and emoji coverage the application actually needs. Missing declared files or hash mismatches are fatal; missing glyphs must be visible in diagnostics where the engine can report them. This specification does not supply font binaries; the implementation must add appropriately licensed fixture fonts.

A developer-only system-font profile may be convenient, but mark it non-hermetic and exclude it from strict visual baselines.

The seeded random contract must identify an algorithm and revision. It applies only to the application's declared preview random API and any explicitly overridden `Math.random`. Never pretend that unspecified cryptographic randomness, external services, or native libraries have become deterministic.

### 5.5 Transitional wall-clock profile

To make P1/P2 useful before full time virtualization, create `preview/profiles/wall-static.json` by copying the default profile and replacing its entire clock object with:

~~~json
{ "mode": "wall" }
~~~

Omit `epochMs` and `startUs` in wall mode. Keep fixture-only services, packaged fonts/assets, and real watchdog limits. Record `timingDeterministic: false`; reject `clock.advance`, exact-frame-time requirements, and a requested `virtual-time` capability. Strict visual baselines reject this profile.

Wall mode uses real elapsed time to drive the native platform and waits for the specifically required frame under its real step deadline. It does not wait for all future timers. It is suitable for early static-component iteration and manual visual review, and is not presented as a deterministic animation test environment.

## 6. Fixture services and assets

### 6.1 Construct mock providers directly

Choose the fixture service registry before evaluating application code. Do not create live D-Bus, compositor IPC, clipboard, filesystem settings, network, audio, process-launch, or power providers and then disable their writes afterward.

Application-facing service interfaces remain the same as in production. Fixture providers return owned data through the same completion/inbox mechanism. Missing providers, unsupported methods, and unexpected requests produce typed failures.

This default directly improves agent iteration: a click in a preview can exercise the UI's behavior without changing the developer's desktop.

### 6.2 Minimal fixture

~~~json
{
  "schemaVersion": 1,
  "id": "empty",
  "services": {},
  "events": []
}
~~~

The counter example needs no system services. An application that requests one with this fixture must fail with `E_FIXTURE_MISSING_SERVICE`.

### 6.3 Stateful service fixture

~~~json
{
  "schemaVersion": 1,
  "id": "desktop-demo",
  "services": {
    "audio": {
      "provider": "mock-audio-v1",
      "initialState": {
        "volume": 0.4,
        "muted": false
      },
      "latencyMs": 0
    },
    "workspaces": {
      "provider": "mock-workspaces-v1",
      "initialState": {
        "activeId": "1",
        "items": [
          { "id": "1", "name": "1" },
          { "id": "2", "name": "2" }
        ]
      },
      "latencyMs": 0
    }
  },
  "events": [
    {
      "atMs": 1000,
      "channel": "audio.state",
      "value": { "volume": 0.65, "muted": false }
    }
  ]
}
~~~

These provider names and payloads are proposed examples. Implement and version their typed contracts with the corresponding service API.

A provider defines its methods, validation, state transitions, event channel, response latency, and failure injection. For example, a mock audio command changes the fixture audio state and records a call; it never changes real system audio. Avoid embedding arbitrary executable JavaScript in fixture JSON.

Record each request, response, state transition, subscription, and disposal with ordered sequence numbers. Assign request IDs on the owner thread. Sort eligible fixture work by `(releaseTimeUs, sourcePriority, sourceSequence)`: declared fixture events have priority 0 and their array index; generated request completions have priority 1 and their request sequence. This ordering applies within the fixture delivery phase, not across native input/timer phases.

Even `latencyMs: 0` completes at the next permitted owner-thread delivery boundary after the host method has returned its Promise. Never recursively resolve during request construction. Take/mark the request entry and release its registry borrow before invoking its resolver. Deliver it once, then release both success and failure roots. Stale or duplicate completions are ignored and recorded.

### 6.4 Fixture errors and application error states

A fixture may intentionally return a typed service error to test an error message. The scenario must identify that expected condition.

Distinguish:

- An expected application-level service failure.
- A missing fixture definition.
- An unexpected method or malformed argument.
- A scenario assertion failure.
- A runtime exception.

An error-state scenario can pass while containing an expected service rejection. An unhandled Promise rejection or fixture contract violation must not be silently treated as expected.

### 6.5 Native resource readiness

Resources must be declared, locally available, and hashed. Reject remote URLs in the deterministic package-only profile, including CSS-triggered loads and image paths that bypass a higher-level JS service.

For dynamic images, `services.images.files` maps exact `file://` URIs to
manifest-declared asset paths. The package must also grant the URI's directory via
`imagePrefixes`; the fixture never opens the URI on disk. `services.images.blobs`
maps declared `registerImageAsset` input paths to declared assets containing the
fixture bytes. Both maps are optional and have no live fallback.

A resource has at least these states: declared, requested, loading, ready-for-current-demand, failed, canceled. Native image demand can change after layout or DPR changes; a ready low-resolution variant is not automatically the required current variant.

Add a native readiness probe that reports request generation, source identity, requested decode variant, attachment status, and failure. Reject stale completions after source replacement or reload. The source's asynchronous image pipeline makes this a concrete integration requirement. [Z8]

The `required-visible` policy blocks capture on currently visible required resources. A scenario deliberately showing a loading placeholder must explicitly allow the pending resource IDs and record that allowance in capture metadata.

Wait for resources under a real deadline while application virtual time remains fixed. Before strict deterministic resource tests are enabled, establish a defined publication barrier/order for worker completions that can affect observable application behavior. A real thread's arrival order must not become an unrecorded source of application ordering.

## 7. Virtual time and bounded readiness

### 7.1 Two separate clocks

| Clock | Controls | Must not control |
| --- | --- | --- |
| Virtual application clock | Native timers/animation, JS timers/rAF timestamps, mock service deadlines, supported UI time APIs | Real watchdogs and I/O deadlines |
| Real monotonic clock | JS execution limits, worker/GPU waits, process timeout, performance measurements | Scenario state or animation position |

The following exact-time rules apply to virtual profiles. Wall profiles follow Section 5.5 and report real elapsed time rather than a fictitious frozen application timestamp.

Use the headless platform's virtual clock throughout the native host. Set `Harness::hold_clock(true)` before delivering scripted configure/resize events, because the harness can otherwise advance time to service resize pacing. [Z2] [Z9]

The existing harness operations have different meanings:

- `advance(duration)` moves virtual time; it does not itself pump all work.
- `pump()` processes queued work at the current time.
- `settle(turns)` repeats pumping and can panic when the budget is exhausted.
- Future deadlines are not the same as immediately pending work.

Implement a project driver that returns structured results and uses exact remaining durations. Do not call `run_for(total, step)` blindly when a final partial step is required. [Z2]

### 7.2 JavaScript clock contract

Install supported clock APIs before evaluating any application module:

- `performance.now()`, if exposed, reads virtual elapsed milliseconds.
- The SDK's wall-clock API reads `epochMs + virtualElapsedMs`.
- If `Date` is virtualized, cover `Date.now()`, zero-argument construction, and the callable `Date()` behavior consistently. Preserve argument-based construction and static parsing behavior.
- Otherwise, reject or explicitly exclude ambient wall-clock use from deterministic applications and provide a typed SDK time API.

Overriding `Date.now` alone is insufficient. Verify the chosen Date surface with actual rquickjs execution before advertising the capability. UTC formatting must not depend on the developer's system time zone.

Preserve the engine-provided `queueMicrotask` in the audited runtime baseline. Pump pending jobs using the runtime API that preserves job exceptions, as required by the companion specification. [Q1] [Q2] [Q3]

### 7.3 Exact advancement

`clock.advance` advances to a target elapsed time, processing relevant intermediate deadlines in order rather than jumping over a timer-driven interaction.

At each visited deadline, enqueue due fixture work using Section 6.3, then run normal native phases and bounded owner-thread checkpoints. The project command includes processing at its endpoint before returning. Stop at that exact endpoint; never overshoot by a fixed frame step.

Scenario commands are sequential. For `clock.advance` to 20 ms followed by a click, timers and fixture work processed at the advance endpoint precede that click, even though both commands report 20 ms. Do not defer endpoint work or reorder it around the next command. A standalone input step first releases already-due fixture work, then delivers its declared native event burst and runs the usual checkpoints. Within that native processing, preserve the pinned engine's input/timer order rather than globally sorting unrelated phases.

If an implementation cannot yet expose or coordinate all required deadlines, it must advertise a narrower timing capability. It may not label frame counting as full virtual-time determinism.

Native repeating timers coalesce missed periods according to the engine contract. The project's deadline-processing advance therefore differs from a raw harness clock jump followed by one pump. Keep a raw-jump operation out of the initial public scenario API. Recheck JS timer cancellation and generation immediately before invocation: an earlier due callback can cancel another callback already extracted into the native due batch. [Z25]

### 7.4 Current-time settle

A `settle` step never advances virtual time implicitly.

It drains runnable owner-thread completions, bounded QuickJS jobs, queued native work, and currently eligible frames. It succeeds when the current state has no immediately runnable work covered by the readiness policy.

It does not wait for every future timer, a forever-running clock, or all future animation frames.

Separate these conditions in diagnostics:

| Condition | Meaning |
| --- | --- |
| App ready | Mount succeeded and any explicitly required ready marker was observed. |
| Current work settled | No covered runnable work remains at the current virtual time. |
| Required assets ready | Visible required native resources have been attached or explicitly allowed pending. |
| Frame current | A successful frame covers the required state/configuration revision. |
| Readback complete | The corresponding GPU pixels are available. |

A quiet event queue alone does not establish all five conditions.

Plain `settle` may succeed while reporting `frameCurrent: false` and a future native deadline. It promises current-time quiescence, not capture readiness. A subsequent `capture` or frame-dependent assertion returns `E_CAPTURE_NOT_READY` if its frame barrier is unmet, including the current time, pending reason, and next native deadline. The agent can add an explicit advance. A later optional bounded auto-advance must be requested explicitly and report the actual time reached.

### 7.5 Frame timestamps

Keep application virtual time, native frame time, and presentation sequence separate.

At virtual time 20 ms, the currently displayed image may legitimately be the frame produced at 16.667 ms. A capture may reuse that image only if it still represents the required state and configuration. Its metadata must retain the actual frame time.

If the scenario supplies `requiredFrameTimeUs`, require an exact match with the `validationFrame` timestamp; otherwise return `E_CAPTURE_NOT_READY`. A validating `Undamaged` frame at that time can reuse an older pixel presentation, whose original timestamp remains separately visible. Never relabel old pixels as newly presented.

Native frame callbacks can run on every actual frame, including multiple frames with the same virtual timestamp. Use frame ordinals and bounded work; do not promise one rAF callback batch per distinct timestamp unless a separate tested adapter enforces that policy. [Z10] [Z11]

### 7.6 Job and execution budgets

Count attempted pending jobs, including jobs that throw. Clamp every JS entry/job deadline to the minimum of its own entry limit and the remaining pump, step, and run deadlines. Clamp each new pump to its enclosing step/run budget before entering the engine.

Preserve thrown values, stack traces, rejection diagnostics, and execution stage. Do not use an exception-swallowing job helper. Recover a job exception through its returned context without nested runtime/context entry. [Q1] [Q2]

When a budget is exceeded:

- Stop the step and prevent further scenario actions.
- Record runnable counts, deadlines, last successful action, and current generation.
- Save only complete frame evidence.
- Attempt bounded disposal.
- Let the controller terminate an unresponsive worker after the real deadline.

An engine interrupt is not a resumable timeslice. A blocked native host call may require process termination; it cannot be made safe by advancing virtual time.

## 8. Scenarios, selectors, and native input

### 8.1 Scenario schema

A version 1 scenario is a closed JSON object with `schemaVersion`, `id`, `requiredCapabilities`, and an ordered `steps` array. Each step has a unique `id` and an `op`; other fields are determined by that operation.

Reject unknown properties. Before mounting, derive capability requirements from the resolved profile, all operations, selector forms, and requested artifact types, then union them with the scenario's explicit `requiredCapabilities`. Omitting a capability from that array never bypasses validation. For example, a virtual profile requires virtual time, a role/name target requires semantic selectors, and `text.commit` requires native text input.

The required-visible asset policy needs `assets.native-readiness` whenever a run uses native image loading. An early no-image scenario may satisfy that policy vacuously, but the host must then reject any image creation/load that would require an unavailable readiness probe. Unknown dynamically requested capabilities fail at their first use; preflight inference does not excuse runtime checks.

Step IDs and capture IDs use `[a-z0-9][a-z0-9._-]*`, must not contain path traversal, and are unique within a run.

### 8.2 Counter example

Add `testId` as a project-owned, typed SDK property. It is diagnostic metadata, not an upstream zgui attribute requirement.

~~~tsx
function Counter() {
  const [count, setCount] = createSignal(0);

  return (
    <column>
      <text testId="count-label">Count: {count()}</text>
      <Button testId="increment" onClick={() => setCount(v => v + 1)}>
        Increment
      </Button>
    </column>
  );
}
~~~

Use the existing bundle entry and `renderApp` mount/disposal contract from the companion specification.

~~~json
{
  "schemaVersion": 1,
  "id": "counter-increments",
  "requiredCapabilities": [
    "pixels",
    "virtual-time",
    "selectors.test-id",
    "input.pointer"
  ],
  "steps": [
    { "id": "mount-settle", "op": "settle" },
    { "id": "initial", "op": "capture", "captureId": "initial" },
    {
      "id": "increment",
      "op": "pointer.click",
      "target": { "surface": "main", "testId": "increment" },
      "button": "primary"
    },
    { "id": "next-frame", "op": "clock.advance", "byMs": 20 },
    { "id": "update-settle", "op": "settle" },
    {
      "id": "count-is-one",
      "op": "assert",
      "target": { "surface": "main", "testId": "count-label" },
      "check": { "property": "text", "equals": "Count: 1" }
    },
    { "id": "updated", "op": "capture", "captureId": "after-increment" }
  ]
}
~~~

The 20 ms step is an explicit scenario choice. If the selected backend/profile requires another deadline, report that fact; do not insert an undocumented sleep or time advance.

### 8.3 Initial operation set

| Operation | Required payload | Behavior |
| --- | --- | --- |
| `settle` | None; optional lower per-step budget | Drain current-time work under Section 7. |
| `clock.advance` | `byMs` | Advance by a nonnegative integer number of milliseconds. |
| `pointer.move` | `target` or `position` plus `surface` | Deliver native pointer movement. |
| `pointer.click` | `target` or explicit position; `button` | Resolve, move, press, release using native dispatch. |
| `pointer.down` | `target` or explicit position; `button` | Dispatch one native press and settle. |
| `pointer.up` | `target` or explicit position; `button` | Dispatch one native release and settle. |
| `pointer.cancel` | `target` or explicit position | Dispatch one native cancel and settle. |
| `wheel` | `target` or `position` plus `surface`; `delta`; optional `phase` | Deliver one native wheel event through the real scroll pipeline. |
| `key.press` | `surface`, `key`, optional `modifiers` | Deliver key down/up, preserving native defaults. |
| `text.commit` | `surface`, `text` | Commit text through the supported native text-input path. |
| `service.emit` | `channel`, `value` | Apply a typed fixture update and enqueue its notification. |
| `assert` | Target/check or service-call check | Evaluate a defined predicate; never arbitrary JS. |
| `capture` | `captureId`; optional `surface` (defaults to `main`) and `requiredFrameTimeUs` | Save coherent pixels, tree, and metadata for the selected surface. |
| `monitors.set` | `monitors` | Replace the synthetic output inventory without advancing virtual time. |
| `popupDone` | `surface` | Deliver a native destruction event to a named popup, as for compositor dismissal. |

For explicit position, use `{"x": 120, "y": 40}` in logical surface pixels. The target form and position form are mutually exclusive. Supported pointer buttons are `primary`, `secondary`, and `middle`; reject other values until explicitly implemented. `pointer.down` and `pointer.up` require a button; `pointer.move` and `pointer.cancel` reject one.

Pointer operations accept an optional pointer object: `{"id":"7","kind":"pen","primary":true}`. The default is primary mouse id "0". IDs are canonical decimal u64 strings; kinds are mouse, touch, pen, or unknown. Use the same ID across a gesture. Each down/up/cancel step dispatches one native action and settles; move and click retain their existing behavior. Explicit positions allow captured gestures to continue outside the control. No operation advances virtual time implicitly.

Wheel accepts the same mutually exclusive location forms as pointer operations, including the resolved-target visibility and enabled checks. `delta` is `{"x": 0, "y": 3, "unit": "lines"}`: `x` and `y` are signed CSS pixels when `unit` is `pixels` and line counts when `unit` is `lines`, with positive `y` scrolling down. `phase` is `discrete`, `started`, `moved`, `ended`, or `momentum` and defaults to `discrete`; the named phases carry native gesture semantics, so a wheel-gesture scenario sends a `started`/`moved`/`ended` sequence explicitly. Wheel uses the primary mouse identity and does not advance virtual time implicitly.

~~~json
{ "id": "scroll", "op": "wheel", "target": { "surface": "main", "testId": "list" }, "delta": { "x": 0, "y": 3, "unit": "lines" } }
~~~

Version 1 key names are an explicit tested enum: `Tab`, `Enter`, `Escape`, `Space`, arrow keys, `Home`, `End`, `PageUp`, `PageDown`, `Backspace`, and `Delete`. Modifiers are an array from `Shift`, `Control`, `Alt`, and `Meta`. Reject unsupported key mappings. `key.press` delivers the native key events exactly; delivering a navigation key such as `PageDown` does not invent window scrolling defaults the runtime lacks, so scroll scenarios use `wheel`. Use `"key": {"character": "é"}` for a character-producing key. The text must be nonempty and may contain multiple codepoints; it is delivered as native `Key::Character` without inventing a physical key or an IME commit. Named `"Space"` and character `{"character":" "}` retain their distinct native categories.

Text commit and physical key input are different operations. Do not simulate Unicode typing by invoking a change handler or assigning a field value. The native editing adapter must use the actual editing model and its supported `ViewHost::set_value` integration for controlled updates, while input scenarios exercise the text-input path. [Z26]

### 8.4 Additional operations after the first slice

Add these only with capability tests:

- `pointer.drag`: start/end target or position, duration, and sample count; use deterministic substep timestamps and native capture behavior.
- `ime.preedit`, `ime.commit`, `ime.cancel`: composition routing and selection tests.
- `surface.resize`, `surface.scale`: update both headless surface and runtime consistently.
- `wait.until`: typed predicate plus explicit maximum virtual advance and real timeout.
- `capture.sequence`: initial capture plus `count - 1` captures at positive `intervalMs`; `count` is the exact number of images.
- `lifecycle.reload` and surface-loss scenarios: generation-safe reinitialization.

A sequence cannot rely on repeated real-time sleeps. If an advanced operation is unavailable, validation fails with `E_UNSUPPORTED_CAPABILITY`.

### 8.5 Selector contract

Version 1 requires exactly one matching node for a target:

~~~json
{ "surface": "main", "testId": "increment" }
~~~

The semantic extension supports exact role and accessible name:

~~~json
{ "surface": "main", "role": "button", "name": "Increment" }
~~~

Do not combine selector forms. Test IDs are unique within a logical surface; duplicates fail validation or target resolution. Accessible names need not be unique; ambiguous matches return candidate summaries and fail.

Return separate failure codes for missing, ambiguous, hidden, disabled, clipped, obscured, and stale targets. Do not silently select the first node.

A successful lookup is not sufficient for a pointer action. Resolve geometry from the latest coherent native layout, choose a visible point, and verify that native hit testing reaches the intended control or its owned descendant. Include clipping, scroll, transforms, and overlays. Recheck after any operation that changes layout.

### 8.6 Coordinate conversion

zgui pointer positions are CSS/logical pixels. Native window boxes, hit-test queries, and screenshot pixels use device pixels in the examined interfaces. [Z12] [Z13]

For a geometry-selected point:

1. Obtain the candidate point in device space from current native geometry.
2. Verify it through the device-space hit-test path.
3. Divide by scale exactly once.
4. Deliver the resulting logical position in the native pointer event.

Do not multiply scenario logical coordinates by scale and then pass them to a logical-coordinate pointer API. Record both logical and device positions in the action trace.

### 8.7 Event dispatch semantics

Pointer click expands to real native movement, press, and release. Keyboard activation must use native key handling. Preserve modifier state, focus, capture, propagation, and synchronous `preventDefault`.

Never call `onClick` directly, manually set hover/active states, or mutate a signal to imitate interaction. Those shortcuts would fail to test the bridge and input system.

Deliver an action's low-level events in a documented burst and record that burst. Repeated individual delivery and one combined burst can produce different scheduling histories.

### 8.8 Assertions

Initial node predicates are `exists`, `text`, `role`, `name`, `enabled`, `focused`, and `visible`. Support exact equality first. `exists: false` is evaluated against zero matches; other predicates require one match.

Define `text` as the native text projection of the selected subtree, excluding hidden inspector chrome. Native editor value is a separate property; it must come from the editing model.

Layout predicates use explicit coordinate space and tolerance. Service assertions match typed method/argument records and expected call counts.

Node assertions use a coherent completed validation frame covering the required state revision. If that frame is unavailable, report readiness failure rather than comparing against a stale tree. Service-call assertions can read the completed fixture trace without a pixel frame.

Assertions are read-only. A failed assertion cannot run a recovery mutation, broaden a selector, or update a baseline automatically.

### 8.9 Early static scenario

The P1 path can execute just the first two steps below with the `pixels` capability. P2 adds the remaining native interaction steps and their capabilities. Use the wall-clock profile from Section 5.5.

~~~json
{
  "schemaVersion": 1,
  "id": "counter-static",
  "requiredCapabilities": ["pixels", "selectors.test-id", "input.pointer"],
  "steps": [
    { "id": "ready", "op": "settle" },
    { "id": "before", "op": "capture", "captureId": "initial" },
    {
      "id": "click",
      "op": "pointer.click",
      "target": { "surface": "main", "testId": "increment" },
      "button": "primary"
    },
    { "id": "updated", "op": "settle" },
    {
      "id": "verify",
      "op": "assert",
      "target": { "surface": "main", "testId": "count-label" },
      "check": { "property": "text", "equals": "Count: 1" }
    },
    { "id": "after", "op": "capture", "captureId": "after-increment" }
  ]
}
~~~

In wall mode, frame-dependent operations wait for their required validation frame using native pacing and the real step budget. They must still reject stale evidence. For P1, create a separate two-step scenario with only `pixels` required; do not edit capability validation to make the P2 scenario run prematurely.

## 9. Screenshot and frame evidence

### 9.1 Real capture integration

Use a real offscreen renderer constructed through the native renderer factory. Preserve the supplied target size, scale, and opacity policy. Wrap drawing to observe its outcome and collect presented pixels.

Version 1 captures through `Bgra8Unorm`; other BGRA8/RGBA8 variants require explicit tests. The current readback implementation assumes four bytes per pixel, so arbitrary float, packed HDR, or other wgpu texture formats are unsupported by this capture adapter. [Z15]

The upstream `scale_capture` example is the integration reference. `read_presented()` is for offscreen/supplied textures; it does not turn an ordinary compositor window into a general screenshot source. `read_composed()` may describe an intermediate image that was not presented and must be labeled separately. [Z1] [Z3] [Z14]

A missing graphics device produces `E_GRAPHICS_UNAVAILABLE`. A structural scene transcript is a different artifact type and must never be emitted as a substitute PNG.

### 9.2 Readback format

Use `Pixels::size()` as the authoritative extent. Its bytes are tightly packed after native row-padding removal. Inspect channel order; BGRA requires swizzling before RGBA PNG encoding. `Pixels::rgba()` changes channel order but does not perform color-transfer or alpha conversion. [Z15]

The pinned renderer explicitly composites premultiplied, gamma-encoded values. A plain `Bgra8Unorm` texture does not imply that these stored values are linear RGB. Do not apply an additional linear-to-sRGB transform merely because the format lacks an `Srgb` suffix. [Z16]

Opaque captures require alpha 255 throughout. Transparent capture converts stored premultiplied RGB to straight-alpha PNG, including zero-alpha handling and low-alpha quantization. `preview doctor` checks both native readback and decoded PNG samples for transparent profiles.

Record target/scene formats, sRGB handling tier, alpha policy, and conversion revision. Validate color swatches and alpha independently from an application screenshot.

### 9.3 Frame freshness

The evidence collector must track:

- Application build ID.
- Session generation.
- Logical surface and current native document identity.
- Input/action sequence.
- Required state and configuration revision.
- Native frame ordinal and frame timestamp.
- Draw outcome and presentation sequence.
- Resource readiness generation.
- Layout/tree snapshot version.

The collector assigns a per-surface frame ordinal and correlates the renderer wrapper's outcome/readback with the passive probe's owned snapshot. Record the virtual frame-start clock value through the driver/renderer integration. `FrameProbe::frame_ended` supplies only a window reference; it does not supply an outcome or timestamp. This correlation is project work. [Z7] [Z11]

Maintain a required validation barrier for accepted input, JS mutations, configuration changes, and observable resource completion. The latest completed native frame must cover that barrier. Its owned snapshot must not be reconstructed later from a mutated live tree.

Keep `validationFrame` separate from `pixelPresentation`. A completed frame reporting `Undamaged` may validate newer semantic/state information while proving that the prior presented pixels remain correct. The collector can pair that new validated tree with the old pixel buffer, retaining both identities and the original pixel timestamp. A timeout, held frame, device failure, resize, or recovery cannot make the same claim. Invalidate captures on device recovery and wait for a new successful presentation. [Z17]

### 9.4 Capture outputs

Each named capture writes:

| File | Contents |
| --- | --- |
| `captures/<id>.png` | Displayable pixel evidence |
| `captures/<id>.frame.json` | Build, generation, timing, dimensions, renderer, readiness, and content digests |
| `captures/<id>.tree.json` | Coherent structured native/semantic projection |
| `captures/<id>.rgba` | Optional raw diagnostic bytes; format declared in frame metadata |

These are the unchanged paths for `main`. Named Shell surfaces write the same
files under `captures/surfaces/<surface>/`; a `captureId` may be reused on another
surface. Baseline review and comparison keys identify the `(captureId, surface)` pair.

Do not bake target rectangles, cursor hints, or diff highlights into the baseline PNG. Optional annotated images use a distinct suffix and refer to the unmodified source capture.

A transparent checkerboard view is also a derived artifact. It cannot replace the raw transparent output or establish real compositor transparency.

## 10. Tree inspection and source attribution

### 10.1 Native and semantic projections

Build the diagnostic tree from the bridge's JS-owned topology, concrete native queries, native editing state, and verified semantic information. Include in-surface overlay content; exclude preview inspector chrome.

The native DOM, accessibility tree, TSX component tree, and layout boxes are related but distinct structures. Preserve their identity domains. Do not assume an accessibility node ID equals a zgui node ID or a bridge token.

Use public window/DOM inspection and a passive frame probe. The existing devtools tree sampler is internal; it is a reference for an implementation, not a public JSON-export API. [Z7] [Z18] [Z19]

### 10.2 Tree record

An illustrative node record:

~~~json
{
  "token": "node-17",
  "parent": "node-9",
  "surface": "main",
  "generation": 1,
  "kind": "control",
  "ownership": "application",
  "testId": "increment",
  "role": "button",
  "name": "Increment",
  "text": "Increment",
  "state": {
    "enabled": true,
    "focused": false,
    "visible": true
  },
  "boundsLogical": { "x": 16, "y": 48, "width": 120, "height": 36 },
  "boundsDevice": { "x": 28, "y": 84, "width": 210, "height": 63 },
  "source": null
}
~~~

This record is a schema example, not a captured measurement. Actual records must carry their frame/layout version through the enclosing snapshot.

A snapshot contains ordered nodes, root tokens, generation, build ID, logical surface, layout version, frame ordinal, presentation sequence, and truncation metadata. Unknown fields are null or omitted according to the schema; do not invent role, name, geometry, or source locations.

Report logical bounds and device bounds explicitly. Geometry should identify whether it is a border box, content box, or visible clipped region. A bounding rectangle alone is not proof of pointer reachability.

### 10.3 Accessibility reconstruction

Headless accessibility logs contain incremental AccessKit updates. Consume them in order and apply them to a retained semantic model, including tree/root changes, focus, and unreachable-node removal. Do not serialize only the final delta as the complete tree. Drain the log regularly so its bounded retention does not lose the initial baseline. [Z20]

Role/name selectors are advertised only after this full-tree behavior is verified, or after an equivalent complete native semantic projection is implemented. An application-authored diagnostic label is not automatically an accessible name.

Keep accessibility inspection read-only. The first pointer/keyboard scenarios still use native input routing. Any future accessibility-action command must be explicitly named, since it exercises a different path.

### 10.4 Source mapping

A JavaScript exception can be source-mapped using the bundle's map. Mapping a native node to a TSX element additionally requires compiler/SDK metadata.

An optional build transform can assign stable source-site IDs to native element creation and component boundaries. It must preserve the normal render behavior and record the metadata version. Derive file/line/column from the actual build's mapping.

Store source locations only when verified. A missing location is better than a guessed component or stale line number. Repeated component instances share a source site but have distinct instance tokens and test IDs.

Do not call application getters or arbitrary `toString` methods while inspecting state. Inspection must not trigger effects or mutate the UI.

### 10.5 Saved inspection command

~~~sh
./target/debug/native-host preview inspect --run artifacts/preview/counter-001 --capture after-increment --test-id count-label --fields text,state,boundsLogical
~~~

This command reads saved artifacts and does not start QuickJS or a GPU. Add
`--surface <name>` to inspect a named capture; the default is `main`. Support a
field allowlist, maximum node count, maximum depth, and a cursor for a stable saved snapshot.

Default responses should contain the selected node and a small amount of parent/child context. Return truncation counts and the full artifact path. Do not print a large tree merely to locate one button.

## 11. Results, diagnostics, and exit codes

### 11.1 Artifact layout

| Path inside a run | Required contents |
| --- | --- |
| `result.json` | Final bounded machine result, written last |
| `artifacts.json` | Complete artifact inventory, roles, digests, frame associations |
| `environment.json` | Actual runtime, graphics, font, and build environment |
| `inputs/profile.json` | Resolved profile including CLI overrides |
| `inputs/fixture.json` | Exact fixture input |
| `inputs/scenario.json` | Exact scenario input |
| `steps.jsonl` | Ordered action outcomes, timing, and selected target information |
| `diagnostics.jsonl` | Bounded structured errors, logs, fixture calls, and pending-work reports |
| `metrics.json` | Work counts and real-time measurements, with availability flags |
| `captures/` | Named frame evidence defined in Section 9 |

An artifact inventory distinguishes `complete`, `historical`, and `partial` entries. Partial files are not accepted as baseline inputs.

The input digest covers the canonical resolved inputs and application package identity. The environment digest covers the declared comparison environment. Keep these separate: a changed application build is expected when comparing a UI edit.

### 11.2 Successful result example

The following is an illustrative result shape, not evidence that this specification has already been implemented:

~~~json
{
  "schemaVersion": 1,
  "command": "run",
  "status": "passed",
  "exitCode": 0,
  "runId": "counter-001",
  "scenarioId": "counter-increments",
  "generation": 1,
  "assertions": { "passed": 1, "failed": 0 },
  "captureCount": 2,
  "firstError": null,
  "artifacts": {
    "manifest": "artifacts.json",
    "latestCapture": "captures/after-increment.png",
    "latestTree": "captures/after-increment.tree.json",
    "diagnostics": "diagnostics.jsonl",
    "environment": "environment.json"
  }
}
~~~

The artifact inventory and environment file contain the actual full build/input digests. The summary must remain small even when a sequence produces many images.

Aim for approximately 2 KiB on stdout; impose a documented upper bound such as 8 KiB. Truncate verbose text, not required identifiers or error codes. `result.json` and stdout must agree on final status and exit code.

### 11.3 Failure information

The first causal failure includes:

- Stable code and category.
- Stage, scenario step, and action sequence.
- Short message and bounded expected/actual values.
- Producing build ID, session generation, and source-map identity.
- Current virtual time and relevant frame timestamp.
- Requested/available state revisions when freshness matters.
- Relevant artifact paths.
- Pending-work details and next deadline for readiness failures.

For JS errors, preserve generated positions alongside mapped TSX locations. Use the map from the build that produced the error, including a rejected candidate build. Never remap an old stack with the newest source map.

For target failures, include a small candidate/blocker summary. For assets, include the exact source, generation, demand, and failed stage. For graphics failures, include the actual backend/device rejection or draw/readback outcome.

On failure, automatically publish bounded evidence from the latest complete owned validation snapshot to `failure/tree.json` and `failure/frame.json`, even if no explicit capture step wrote it. Include the failure step's required revision and whether that saved snapshot satisfies it. If complete pixels are available, include `failure/last-valid.png` with their original presentation identity. These paths go in the result and artifact inventory.

Do not run application code or advance time merely to manufacture a fresh failure screenshot. If no complete frame/tree exists, report those artifacts as unavailable. A stale snapshot can help debugging but must be labeled historical, never current or successful.

Capture diagnostics without executing arbitrary application formatting code. If formatting fails, retain a bounded fallback representation and the original error category.

### 11.4 Exit codes

| Exit | Meaning | Representative codes |
| --- | --- | --- |
| 0 | Requested operation passed | No outstanding failure |
| 2 | Invalid command, schema, incompatible input, or unsupported capability | `E_SCHEMA`, `E_UNSUPPORTED_CAPABILITY`, `E_BASELINE_INCOMPATIBLE` |
| 3 | Bundle evaluation, JS execution, or unhandled rejection failed | `E_BUNDLE`, `E_JS_EXCEPTION`, `E_UNHANDLED_REJECTION` |
| 4 | Target, assertion, or visual comparison failed | `E_TARGET_MISSING`, `E_TARGET_AMBIGUOUS`, `E_ASSERTION`, `E_VISUAL_DIFF` |
| 5 | Execution/readiness budget exhausted or required frame not ready | `E_JOB_BUDGET`, `E_WATCHDOG`, `E_CAPTURE_NOT_READY`, `E_ASSET_TIMEOUT` |
| 6 | Fixture or resource contract failed | `E_FIXTURE_MISSING_SERVICE`, `E_FIXTURE_CALL`, `E_ASSET_FAILED` |
| 7 | Graphics setup, drawing, or readback failed | `E_GRAPHICS_UNAVAILABLE`, `E_DEVICE_LOST`, `E_READBACK` |
| 8 | Artifacts could not be safely published | `E_OUTPUT_IO`, `E_OUTPUT_LIMIT` |
| 9 | Host lifecycle/internal failure or worker crash | `E_TEARDOWN`, `E_HOST_INVARIANT`, `E_WORKER_CRASH` |
| 130 | Explicit cancellation | `E_CANCELED` |

The first causal failure determines the run category. If final artifact publication itself fails, exit 8 and report both the publication failure and original cause when a result channel remains available.

A failed teardown changes an otherwise passing run to failure. Later cleanup diagnostics must not erase an earlier causal error. Fail fast by default; do not continue mutating scenario actions after failure.

### 11.5 Capability negotiation

`doctor` reports a versioned capability set, actual graphics availability, profile validation, and missing resources. A capability is true only after its integration gate is implemented and checked for the selected environment.

Initial names:

~~~text
pixels
virtual-time
selectors.test-id
selectors.semantic
input.pointer
input.keyboard
input.text
input.wheel
input.ime
assets.native-readiness
capture.transparent
capture.sequence
surface.resize
surface.scale
surface.multiple
lifecycle.reload
source.node-location
~~~

Do not imply that a flag alone establishes correctness. Store the capability implementation revision and the selected dependency baseline.

`capture.transparent` is available whenever `pixels` is available. A scenario can require it with either profile opacity; only a transparent target preserves translucent pixels.

A GPU-less machine can still run pure runtime/bridge tests or saved inspection/comparison. It cannot pass a scenario requiring `pixels`. Required CI graphics jobs must fail when the adapter is unavailable. `preview run` uses exit 7; any optional local skip belongs to an external test wrapper and must not be recorded as a passing preview run.

### 11.6 Logging and artifact limits

Apply byte limits to logs, node dumps, screenshots, traces, and the run total. Preserve the first causal error, recent relevant context, and dropped-record counts.

Keep service payloads fixture-scoped. Do not copy the developer's process environment, live clipboard, credentials, or unrelated home-directory data into diagnostics. The runner needs reproducible inputs and concrete errors, not a full machine dump.

## 12. Visual comparisons and baseline review

### 12.1 Compare complete evidence

~~~sh
./target/debug/native-host preview compare --baseline preview/baselines/counter --actual artifacts/preview/counter-001 --policy preview/comparison/strict.json --out artifacts/preview/counter-diff-001
~~~

Comparison reads complete artifacts; it does not rerun the application. Match captures by stable capture ID and logical surface, not directory ordering. Strict comparison requires the expected capture set on both sides, rejects missing/unexpected captures, and never scores only the intersection. An empty capture set cannot pass. A deliberately selected subset must be explicit in the comparison policy and reported in the result.

Check compatibility before pixel scoring: physical extent, scale, font configuration, relevant renderer/color policy, scenario timing, fixture identity, and comparison-policy version. Application build identity is expected to change and is reported rather than required to match.

If an environment or profile change invalidates strict comparison, report `E_BASELINE_INCOMPATIBLE`. Still allow side-by-side evidence generation for review, but do not report a strict pass.

### 12.2 Policies

Start with exact RGBA equality in a pinned environment. Add a tolerant policy only with explicit, reviewed thresholds.

A tolerant policy must define:

- Per-channel delta threshold.
- Allowed count or ratio of changed pixels.
- Whether alpha is scored separately.
- Any region-specific rules.
- Mask geometry and its reason.
- Policy/schema revision.

Return changed-pixel count/ratio, maximum channel difference, and the changed region's bounds. Generate a diff image and an optional side-by-side image. Do not silently resize images, ignore alpha, blur both images, or choose a different threshold until the comparison passes.

Freeze time and data before adding masks. A mask is part of the reviewed test input; report masked pixel count and reject a policy that masks the entire image.

Perceptual metrics can be added later, but must not hide structural assertions such as a missing button or incorrect text.

### 12.3 Baseline changes

Keep the baseline image, matching tree assertions, input identity, and environment policy together. A baseline update is an intentional artifact change that should be reviewable alongside the source change.

An agent must not bless a failing image merely to clear a test. It should explain whether the difference is intended, inspect the relevant capture, and update only the affected baseline when the task authorizes that new behavior.

Store baseline images in an appropriate repository artifact store or versioned location. Routine run outputs are disposable and should have a bounded retention policy.

### 12.4 Transparency and platform limits

A transparent offscreen image can test alpha pixels and text/render-target policy. Its `PreMultiplied` metadata describes stored pixels, not the Wayland compositor's blend mode; `compositorTransparencyValidated` remains `false`. [Z21]

Do not claim cross-GPU byte identity. Use the recorded environment to distinguish a UI regression from an environment change, then choose the appropriate comparison policy deliberately.

## 13. Reload and watch mode

### 13.1 Reuse one-shot execution first

The first watch implementation watches atomically published application packages and starts a fresh one-shot worker for each accepted build. It reuses the same profile, fixture, scenario, and validation code as `preview run`.

Use `bun run preview --app counter --watch` to prepare and manage the builder and preview watcher in one terminal. The preview watcher observes complete manifest publication, not individual partially written JS/CSS files. Integrations managing their own host can use `bun run build --app counter --watch` separately.

Do not run simultaneous mutating scenarios in one session. Coalesce rapid rebuild notifications to the newest complete build. Cancel a superseded run through its controller, with a real termination deadline.

### 13.2 Watch outputs

Watch mode is the exception to the one-JSON-object stdout rule: it emits bounded JSON Lines with event types such as `build.accepted`, `run.started`, `run.completed`, and `run.canceled`.

Each run receives its own immutable output directory. Publish pointers or small index files for `latest-attempt` and `last-success`, each containing run and build identity.

If the latest build fails, a previous screenshot may remain useful as historical evidence. Label it as last-success; never present it as a capture of the failed build.

### 13.3 Persistent session reload later

If a persistent session is added, preserve the companion specification's full-generation reload contract:

1. Validate a complete candidate package.
2. Preflight module evaluation in an isolated candidate context with mount-time side effects rejected or explicitly staged.
3. Keep the old active session if candidate evaluation fails.
4. At a safe owner-thread boundary, stop new work and dispose the old session.
5. Enable mount-time APIs and mount the candidate into a clean root.
6. Deliver queued fixture completions only after successful mounting.
7. If mounting fails after old disposal, report that failure; do not promise automatic rollback of the old live UI.

Reset creates a new clock, fixture model, RNG state, generation, registries, and application root. It does not rewind an existing clock or preserve unknown JS state.

### 13.4 Teardown and stale work

Teardown is bounded and idempotent:

- Stop accepting scenario actions and new resource creation.
- Permit cleanup removals, cancellation, and unsubscription.
- Run the framework disposer and native subtree cleanup.
- Cancel timers, frame callbacks, subscriptions, and requests.
- Ignore late completions from retired generations.
- Release persistent JS roots before dropping the context/runtime.
- Release capture buffers and native document mappings.

Persistent rquickjs values must not outlive their runtime. The existing runtime/lifetime rules remain mandatory in preview. [Q4] [Q5]

If a future Preact adapter is added, test its own effect and unmount scheduling before enabling reload. Preview must not assume that every TSX framework disposes resources on Solid's schedule.

### 13.5 Recreated native surfaces

Logical name `main` is not a permanently valid native SurfaceId. On surface loss/recreation, find current runtime windows, rebind the JS host and capture/probe state to the new document, and retire stale accessibility/capture mappings.

The audited headless suspend/resume behavior can leave old platform surface entries while the runtime creates new ones. Do not use `deliver_to_first()` or `offscreens()[0]` after recreation. [Z2] [Z22] [Z23]

## 14. Performance and feedback cost

### 14.1 Measure the feedback loop

Record real elapsed stages separately:

- Application type check and build, when invoked by the surrounding workflow.
- Worker startup and GPU initialization.
- Bundle evaluation and mount.
- Scenario actions and JS job execution.
- Native style/layout/paint.
- Resource readiness wait.
- GPU readback and PNG encoding.
- Comparison and artifact publication.
- Teardown.

Also record available counters: JS entries/jobs, native frames, redraw requests, timer callbacks, node creations/removals, host-service calls, bridge calls, and pending registrations at teardown.

Do not present virtual elapsed time as a performance measurement. Do not claim exact JS heap/GPU memory accounting when the underlying API cannot provide it; mark the metric unavailable.

### 14.2 Cost controls

Use targeted scenarios and named captures. Avoid encoding every frame unless the scenario requests a sequence. Read back GPU pixels only when needed for evidence or a capture gate.

Reuse the compiled host for UI-only work. Keep immutable asset/build caches separate from session state. Evaluate warm-process optimization only after one-shot correctness, and report cold versus warm measurements separately.

Saved inspection should be much cheaper than restarting the runtime. Its bounded output is the default next step for a selector or text assertion failure.

Never keep the application awake for the inspector. After the scenario reaches a legitimate idle state, the probe must not create additional work. Use native counters to detect unintended frame feedback. [Z2]

### 14.3 Suggested evaluation workloads

Use these workloads to assess improvements, without assuming a target result in advance:

| Workload | What it measures |
| --- | --- |
| Static counter mount | Minimum end-to-end overhead |
| One click and one text update | Typical short agent loop |
| Keyed list insertion/reorder | Identity and native mutation cost |
| Native field editing | Input, selection, controlled value, and IME path |
| Delayed mock service | Loading/error/success transitions |
| Repeated image source/DPR changes | Native resource readiness and capture correctness |
| Scripted animation sequence | Timing and frame evidence |
| Repeated reloads | Cleanup stability and accumulated overhead |
| Idle native application | Unnecessary wakeups and redraws |

Record results in a separate measured report. The default limits in this specification are execution safeguards and feedback controls, not benchmark claims.

## 15. Real desktop validation

Headless preview validates application behavior and native rendering under a controlled host. It does not replace the desktop integration suite.

Before claiming a native desktop capability works on a backend supported by upstream zgui, verify the applicable behavior on that backend:

- Layer placement, anchors, margins, and exclusive zones.
- Keyboard interactivity and focus transitions.
- Popup strategy and interaction outside the layer-surface bounds, where supported upstream.
- Transparency over an actual compositor background.
- Fractional scale, including 1.75, and moving between outputs.
- Output removal/recreation and stale-handle rejection.
- Actual IME composition and clipboard integration.
- Occlusion, idle behavior, and service-triggered wakeups.
- Asynchronous resource-service delivery and failures.

The offline `popup_protocol` gate verifies Wayland layer-parent popup requests.
Headless popup captures verify the mount region and dismissal lifecycle, not
compositor placement, edge flipping, or input grabs. [Z24]

A headless platform can declare monitor/capability fixtures without implementing the corresponding compositor operations. Record requested surface roles as useful diagnostics; do not treat those declarations as protocol validation. [Z23]

For multiple surfaces, capture each surface separately with parent/role/placement metadata. A combined desktop image requires a separately defined compositor or composition fixture and must be labeled accordingly.

No particular window manager is a baseline requirement, and this library does
not provide compositor-specific IPC services or a window-manager abstraction.

## 16. Implementation phases

| Phase | Deliverable | Completion gate |
| --- | --- | --- |
| P0: Environment and source seams | Doctor command, exact dependency inventory, real offscreen calibration capture | Real fonts and GPU pixels; explicit no-adapter failure |
| P1: Real TSX capture | Existing bundle loader/session/bridge in one-shot preview; PNG and result | Counter mounts through actual rquickjs; clean teardown |
| P2: Agent interaction loop | Test IDs, coherent tree, native click, text assertion, saved inspection | Counter changes through real input; target errors are actionable |
| P3: Deterministic environment | Shared virtual time, typed fixture providers, budgets, native asset readiness | Timing/fixture/resource tests pass without hidden sleeps or live fallbacks |
| P4: Visual regression and CI | Artifact identity, compare policy, strict/tolerant evidence, CI graphics gate | Compatible comparisons are reproducible; incompatible environments are explicit |
| P5: Watch and lifecycle | Atomic-build watcher, fresh worker reruns, cancellation; persistent reload only if needed | Old builds and generations cannot contaminate new results |
| P6: Desktop parity | Real supported-backend smoke checks and documented upstream platform limits | Actual compositor/input/resource-service behavior is verified; no specific window manager is required |

P0-P2 should produce a useful static-fixture loop using the explicit transitional wall-clock profile. Fixture-only services and the real watchdog already apply in P1. Do not advertise full virtual time, images-ready, transparent capture, semantic selectors, or reload before their respective gates pass.

The fully deterministic example in Section 8 becomes a release example after its required capabilities are available. During P1/P2, use an explicitly narrower static scenario/profile rather than ignoring required capability flags.

Implementation should reuse the main development specification's completed phases. If the main host does not exist yet, implement its minimal real-engine/native-counter path first; do not build a preview-only substitute.

## 17. Acceptance matrix

These cases test cross-layer behavior. They are implementation requirements, not claims that tests have already run.

| ID | Scenario | Required result |
| --- | --- | --- |
| PV01 | Real renderer calibration | PNG comes from actual offscreen wgpu with real font shaping and rasterization. |
| PV02 | No usable graphics adapter | Required-pixels run fails explicitly; no transcript fallback passes. |
| PV03 | Odd physical width and corner colors | Correct extent, orientation, channel order, and row-padding removal. |
| PV04 | Color and alpha calibration | Gray/color swatches and premultiplication conversion match the renderer contract. |
| PV05 | Real TSX counter bundle | Actual rquickjs evaluation and production bridge create the visible UI. |
| PV06 | Atomic bundle publication | Mixed JS/CSS/manifest generations are never loaded. |
| PV07 | Source-mapped application exception | Original error, producing build, generated position, and valid source mapping are retained. |
| PV08 | Initial mount failure | Bounded failure and native/JS cleanup; no successful capture is claimed. |
| PV09 | Native counter click | Hit-tested movement/press/release changes Count: 0 to Count: 1. |
| PV10 | Duplicate/missing test ID | Unique-match failure with bounded useful candidates; no first-match fallback. |
| PV11 | Clipped or obscured button | Pointer action reports the visibility/blocker failure instead of invoking a callback. |
| PV12 | Scales 1.0/1.25/1.5/1.75/2.0 | Geometry-selected input hits the intended control; pixel dimensions are correct. |
| PV13 | Keyboard activation and focus | Native Tab/Enter/Space behavior and cancellation semantics are preserved. |
| PV14 | Saved tree inspection | Queries use the chosen capture version and do not start a runtime. |
| PV15 | Semantic tree updates | Ordered incremental updates reconstruct a full current tree and remove retired nodes. |
| PV16 | Frozen time plus infinite JS loop | Real watchdog interrupts/terminates; virtual time does not need to advance. |
| PV17 | Infinite or repeatedly throwing jobs | Every attempt consumes budget; original job failures remain actionable. |
| PV18 | Due timers and cancellation | Equal deadlines follow native ordering; a canceled due JS timer is suppressed by bridge guards. |
| PV19 | Resize under held clock | No implicit advance; pending pacing deadline is reported. |
| PV20 | Native RAF and extra same-time frames | Native ordinals are recorded and bounded; no false once-per-timestamp promise. |
| PV21 | Delayed mock response | Loading UI can settle/capture while a future response remains pending. |
| PV22 | Fixture writes and missing providers | Only mock state changes; unexpected calls fail with zero live-provider calls. |
| PV23 | Simultaneous service completions | Stable documented ordering independent of real thread arrival. |
| PV24 | Delayed/corrupt/replaced image | Readiness detects pending, failure, and stale completion; placeholders cannot pass as ready. |
| PV25 | Font and time reproducibility | Declared fonts, locale, epoch, clock APIs, and random policy agree across reruns. |
| PV26 | Frame/tree coherence | Pixels and tree cover the same generation, revision, scale, and presentation state. |
| PV27 | Pacing/readback failure | Old pixels are not relabeled current; readiness/graphics code and last valid evidence are retained. |
| PV28 | Exact visual baseline | Same controlled inputs compare correctly; app changes produce a reviewable diff. |
| PV29 | Incompatible baseline or mask | Dimension/environment mismatch is explicit; all-image masking is rejected. |
| PV30 | CI without GPU or with output failure | Required jobs fail with the correct category; no false green/skipped pass. |
| PV31 | Watch build failure | Last-success remains labeled historical; latest-attempt identifies the failed build. |
| PV32 | Superseded run | Cancellation is bounded; only the newest complete build becomes latest-attempt. |
| PV33 | Late completion after reload | Retired generation cannot mutate the new session; roots are released. |
| PV34 | Persistent candidate/mount failure | Preflight and post-disposal failure behavior follow the declared transaction boundary. |
| PV35 | Repeated reload/disposal | Node, callback, request, and timer counts return to the established baseline. |
| PV36 | Surface loss/resume | New native IDs are rebound once; obsolete surfaces/captures are never targeted. |
| PV37 | Native text and IME scenario | Synthetic routing preserves selection/composition; real IME remains a separate desktop test. |
| PV38 | Upstream layer-shell placement/popups | Actual supported roles, focus, exclusive zones, and popup limitations are verified on the selected upstream backend. |
| PV39 | Real transparency and output scaling | Actual compositor blending and fractional/output transitions are verified. |
| PV40 | Idle/occlusion/live service behavior | Native wakeups and real service integration work without continuous artificial redraw. |

For each implemented capability, record the command, dependency baseline, environment, outcome, and evidence paths. Do not mark unimplemented cases as passing. Use targeted subsets during ordinary UI edits and the relevant broader gate when changing shared lifecycle, timing, input, or rendering behavior.

## 18. Agent execution rules and handoff prompt

### 18.1 Rules for the implementation agent

- Read the repository instructions and the companion host specification before editing.
- Preserve zgui, rquickjs, and TSX as the selected stack.
- Keep Solid as the current frontend; preview does not authorize a framework migration.
- Verify source signatures before using them; project API names in this specification are not upstream exports.
- Make the first real TSX capture work before adding an RPC server, MCP integration, or inspector UI.
- Keep application input on the native dispatch path.
- Resolve unique selectors and current geometry; do not fake state changes to pass a scenario.
- Use fixture providers by construction and fail on missing contracts.
- Keep virtual application time separate from real execution deadlines.
- Preserve pending-job errors and runtime ownership rules.
- Track native resource readiness and actual presentation, not an arbitrary sleep count.
- Preserve frame/build/generation identity across all artifacts.
- Never silently downgrade a requested capability or reuse stale pixels as current evidence.
- Keep output bounded and source diagnostics tied to the producing artifact.
- Validate the smallest meaningful scenario after a UI change; broaden checks for shared infrastructure changes.
- Treat baseline updates as intentional reviewed changes.
- Report actual build/runtime/visual verification levels separately.

### 18.2 Ready-to-use English handoff prompt

~~~text
Implement Preview Mode for the existing zgui + rquickjs + TSX native UI host.

Read the repository instructions, the main native UI development
specification, and this Preview Mode specification first. Preserve the
selected stack and the current Solid Universal frontend. Keep preview below
the framework adapter boundary.

Start with the smallest end-to-end path:
1. Load the real compiled counter application package.
2. Execute it in the existing rquickjs session and production UI bridge.
3. Render through real zgui fonts, layout, text rasterization, and offscreen
   wgpu.
4. Write an actual PNG and a compact structured result.
5. Add one unique testId target, native pointer activation, a count-text
   assertion, a second capture, and correct teardown.

Do not substitute a browser preview, a hand-authored mock, a scene transcript,
or direct onClick invocation for the native path.

Use fixture service providers by construction. Missing providers and
unexpected calls must fail; they must never fall through to live desktop
services. Keep application virtual time separate from the real watchdog.

Add capabilities in the phases defined by the specification. Do not claim
deterministic time, native asset readiness, semantic selectors, transparent
capture, or reload until the corresponding gate passes.

In particular:
- use the shared native virtual clock and prevent hidden resize advancement;
- preserve QuickJS job exceptions and bound individual JS entries;
- resolve input against current native geometry and correct coordinate units;
- verify native image readiness instead of sleeping for a guessed duration;
- convert readback channels and alpha according to the actual renderer;
- associate PNGs and trees with one build/generation/frame identity;
- reject stale targets and late completions after reset or reload;
- retain historical screenshots only with their original build identity.

Implement one-shot run and saved inspection before persistent protocols.
Reuse the same runner for later watch/CI/MCP integration.

Keep changes reviewable. Run the smallest meaningful acceptance cases for
each phase and record commands, environment, results, and evidence paths.
Do not mark a source audit as a successful build, a transcript as a native
screenshot, or an unimplemented/skipped test as passed.

Deliver the working implementation, versioned schemas and example inputs,
updated local commands, and a phase-by-phase verification report. Stop
optional expansion once the requested phase and its acceptance gates are
complete.
~~~

### 18.3 Rules for an agent using preview

When changing a component, preserve the scenario that reproduces its behavior. Read the machine failure before opening unrelated files.

Use a source-mapped stack for code errors, saved tree queries for targeting/state errors, and the PNG for visual errors. Compare the same capture IDs under the same profile.

If the runner reports a future deadline, make the time advance explicit. If it reports an unsupported capability or absent graphics device, address that environment/integration issue; do not edit the application to conceal it.

A final change report should identify the affected scenario, actual assertions, visual evidence reviewed, and remaining platform limitations. A screenshot being generated does not prove it was inspected.

## 19. Future extensions

### 19.1 Persistent stdio or MCP interface

After one-shot behavior is reliable, expose the same typed commands through a persistent adapter:

~~~text
initialize
load
reset
perform
inspect
capture
compare
dispose
~~~

A persistent request includes request ID, expected session generation, and command sequence. Serialize mutations. Read-only responses identify the snapshot version they describe.

Use the existing runner/schema implementation. Do not maintain a second event loop or a second interpretation of selectors, settle, capture, or fixture semantics for MCP.

Begin with local stdio. A network server adds transport, authentication, and lifecycle requirements and is not needed for the first agent workflow.

### 19.2 Component catalog and focused previews

A later component catalog can register named preview entries that import production components with typed fixture props. It should share the same SDK, renderer, services, fonts, and scenario runner.

A component preview is useful for isolating a menu or notification. It must not become a separately styled duplicate of the production component.

### 19.3 Additional frontend adapters

A future Preact adapter can reuse profiles, fixture services, native input, capture, saved inspection, comparisons, and most scenarios.

Its render-completion and disposal behavior must satisfy the same native host contract. Add adapter-specific tests for scheduling and lifecycle; do not change the preview protocol merely to expose framework internals.

### 19.4 Animation and multiple surfaces

Add exact animation timelines only after native/application clock parity and frame semantics are verified. Add multiple surfaces only after logical-to-native identity rebinding and capture ownership are tested.

Each extension should reduce a concrete development bottleneck. A complete browser automation API, dual JS runtime, or custom compositor is not a prerequisite for an effective preview loop.

## 20. Source index

The following sources establish the upstream contracts used here. Proposed CLI/schema behavior is defined by this specification. Recheck source-dependent behavior when changing a pinned revision.

### zgui

- [Z1] Real native pipeline and offscreen capture example.
- [Z2] Headless harness, time advancement, settle, and lifecycle.
- [Z3] Offscreen renderer builder and graphics failure.
- [Z4] Scene testkit and fixed text metrics.
- [Z5] View testkit boundaries.
- [Z6] Shipped-only font collection and registration.
- [Z7] Public frame probe.
- [Z8] Native asynchronous image loading and demand.
- [Z9] Virtual clock.
- [Z10] Native frame callback batching.
- [Z11] Native frame order and presentation pipeline.
- [Z12] Logical pointer coordinates.
- [Z13] Window geometry and hit testing.
- [Z14] Presented/composed readback interfaces.
- [Z15] Pixel format access and readback row padding.
- [Z16] Gamma-encoded premultiplied renderer contract.
- [Z17] Draw outcomes and recovery.
- [Z18] Public window inspection and binding.
- [Z19] Concrete DOM queries.
- [Z20] Headless semantic/input logs and refresh rate.
- [Z21] Offscreen target and composite-alpha handling.
- [Z22] Runtime surface lifecycle.
- [Z23] Headless platform capabilities, monitors, and surfaces.
- [Z24] Wayland layer/popup constraints.
- [Z25] Native timer ordering and extraction.
- [Z26] Native host editing and scheduling interface.

### rquickjs

- [Q1] Runtime job execution, limits, and interruption.
- [Q2] Context-level job and Promise behavior.
- [Q3] Runtime/context entry ownership.
- [Q4] Persistent value lifetime.
- [Q5] Runtime rejection/interrupt callback definitions.

### AntiWebUI and Shoji Bar references

- [A1] AntiWebUI headless rendering helpers.
- [A2] Driven native input snapshot example.
- [A3] Shoji preview CLI and real-service behavior.
- [A4] Shoji preview execution and real-time frame stepping.

[Z1]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui/examples/scale_capture.rs
[Z2]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-platform-headless/src/harness.rs
[Z3]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-render-wgpu/src/renderer/builder.rs
[Z4]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-testkit-scene/src/lib.rs
[Z5]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-testkit-view/src/lib.rs
[Z6]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui/src/app/fonts/mod.rs
[Z7]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-runtime/src/probe.rs
[Z8]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-runtime/src/images.rs
[Z9]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-platform/src/clock/manual.rs
[Z10]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-runtime/src/frame_callbacks.rs
[Z11]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-runtime/src/window/frame.rs
[Z12]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-vocab/src/event/payload/pointer.rs
[Z13]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-runtime/src/window/probe.rs
[Z14]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-render-wgpu/src/renderer/mod.rs
[Z15]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-render-wgpu/src/renderer/readback.rs
[Z16]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-render-wgpu/src/gpu/formats.rs
[Z17]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-render/src/outcome.rs
[Z18]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-runtime/src/window/mod.rs
[Z19]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-view-dom/src/dom/query.rs
[Z20]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-platform-headless/src/surface.rs
[Z21]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-render-wgpu/src/target/swapchain.rs
[Z22]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-runtime/src/app.rs
[Z23]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-platform-headless/src/platform.rs
[Z24]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-platform-wayland/src/driver/surfaces.rs
[Z25]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-runtime/src/timer.rs
[Z26]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-view/src/host/mod.rs
[Q1]: https://github.com/DelSkayn/rquickjs/blob/73fcd6062ee615aeb2a4675cd32be67a56e93973/core/src/runtime/base.rs
[Q2]: https://github.com/DelSkayn/rquickjs/blob/73fcd6062ee615aeb2a4675cd32be67a56e93973/core/src/context/ctx.rs
[Q3]: https://github.com/DelSkayn/rquickjs/blob/73fcd6062ee615aeb2a4675cd32be67a56e93973/core/src/context/base.rs
[Q4]: https://github.com/DelSkayn/rquickjs/blob/73fcd6062ee615aeb2a4675cd32be67a56e93973/core/src/persistent.rs
[Q5]: https://github.com/DelSkayn/rquickjs/blob/73fcd6062ee615aeb2a4675cd32be67a56e93973/core/src/runtime.rs
[A1]: https://github.com/bea4dev/AntiWebUI/blob/43f0b88b3e5cf882c157563a547b05273fed7ebb/src/anti-webui-app/src/headless.rs
[A2]: https://github.com/bea4dev/AntiWebUI/blob/43f0b88b3e5cf882c157563a547b05273fed7ebb/src/anti-webui-app/examples/interaction_snapshot.rs
[A3]: https://github.com/bea4dev/shoji-bar-4/blob/a30d90cb80754f45785a6c444e31c59e9ea1fac0/docs/development.md
[A4]: https://github.com/bea4dev/shoji-bar-4/blob/a30d90cb80754f45785a6c444e31c59e9ea1fac0/src/preview.rs


