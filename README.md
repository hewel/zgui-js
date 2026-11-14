# zgui-js

**Native TSX applications powered by zgui, QuickJS, and Solid.**

zgui-js is an experimental native UI host. Applications are written in TypeScript and TSX, use Solid's reactive model through `@zgui/ui`, and run in an embedded QuickJS runtime via rquickjs. A Rust host owns the native UI tree, styling, layout, rendering, input, asynchronous services, and application lifecycle.

There is **no WebView and no browser DOM**. The repository also includes an offscreen preview system that uses the production application package, JavaScript runtime, UI bridge, and native renderer for scenario-based screenshots, assertions, and image comparisons.

## Requirements

| Component | Requirement |
| --- | --- |
| Rust | `1.100.0`, pinned by `rust-toolchain.toml` |
| Node.js | `>=26.13.0` |
| pnpm | `12.18.0` |
| Native build tools | A C/C++ toolchain capable of building QuickJS and the native dependencies |
| Graphics | A graphics adapter and driver supported by the native wgpu renderer |

Interactive examples have been verified on Linux with Wayland. The niri panel additionally requires a niri session and a valid `NIRI_SOCKET`.

Offscreen previews do not open a desktop window, but they still require a working graphics adapter. They are not software-only JavaScript screenshots.

Dependencies and upstream revisions are pinned. Keep `Cargo.lock` and `pnpm-lock.yaml` when installing or building. See the [implementation baseline](docs/development/baseline.md) for the audited dependency set and [vendor/README.md](vendor/README.md) for the local native patches.

## Quick start

Run all commands from the repository root:

```sh
pnpm install --frozen-lockfile
pnpm ui:build --app counter
cargo build -p native-host --locked
```

Launch the counter in a Wayland session:

```sh
env ZGUI_PLATFORM=wayland target/debug/native-host \
  --bundle apps/examples/counter/dist
```

Click **Increment** to update the native **Count** and **Doubled** labels. For a bounded launch, append `--exit-after-ms 5000` to request an orderly shutdown after five seconds.

### Development and reload

Start the application builder in one terminal:

```sh
pnpm ui:dev --app counter
```

Start the native host in another:

```sh
env ZGUI_PLATFORM=wayland target/debug/native-host \
  --bundle apps/examples/counter/dist --watch
```

The builder validates and publishes each application package atomically. The host then preflights the new package, retires the previous session, and mounts a fresh JavaScript generation.

This is **full-session reload, not state-preserving HMR**. The `dist` path points to immutable generation directories; rebuild after source changes rather than editing an already published package.

## Examples

Use the same build and launch commands with another application name:

| Application | Demonstrates |
| --- | --- |
| `counter` | Signals, derived values, native buttons, and click events |
| `lists` | Dynamic lists and identity-based native node reuse |
| `async` | Timers, frame callbacks, promises, and asynchronous host services |
| `editing` | Native single-line and multiline editing, selection, IME, read-only, and disabled states |
| `panel` | Live niri workspace/window subscriptions and explicit connection states |

### niri panel

List the available outputs and set `monitor` in [apps/examples/panel/layer.json](apps/examples/panel/layer.json) to an output on your system. The checked-in configuration targets `DP-1`:

```sh
niri msg --json outputs
```

Build and launch the panel:

```sh
pnpm ui:build --app panel
env ZGUI_PLATFORM=wayland target/debug/native-host \
  --bundle apps/examples/panel/dist \
  --layer-config apps/examples/panel/layer.json
```

The profile creates a top-layer surface, reserves a 360-logical-pixel exclusive zone, and requests no keyboard focus. The panel reads real niri IPC state. Startup failures and disconnects are visible in the UI; they are never replaced with a simulated desktop.

See the [panel guide](apps/examples/panel/README.md) for output lifecycle behavior and shell-specific limitations.

## Offscreen previews

Build the counter and native host as shown in [Quick start](#quick-start), then check the graphics, font, and native rendering environment:

```sh
target/debug/native-host preview doctor \
  --profile preview/profiles/default.json \
  --out target/preview-doctor-001
```

Run the counter scenario. It captures the initial frame, performs a native pointer click, advances virtual time, asserts `Count: 1`, and captures the updated frame:

```sh
target/debug/native-host preview run \
  --bundle apps/examples/counter/dist \
  --profile preview/profiles/default.json \
  --fixture preview/fixtures/empty.json \
  --scenario preview/scenarios/counter-virtual.json \
  --out target/preview-counter-001
```

The default profile uses an 800×600 logical surface at 1.75× scale, producing 1400×1050 PNG images. Each run records a structured `result.json`, screenshots, native tree snapshots, frame evidence, and an artifact manifest. Stdout contains a bounded JSON summary.

Use a **new `--out` directory for every run**. Existing historical screenshots are not proof that a later run succeeded.

Inspect the native node associated with the final capture:

```sh
target/debug/native-host preview inspect \
  --run target/preview-counter-001 \
  --capture after-increment --test-id count-label \
  --fields text,state,boundsLogical --max-nodes 1
```

Additional workflows:

- `preview watch` follows atomic package publications, cancels superseded work, and reruns scenarios in fresh workers.
- `preview compare` compares stored captures against reviewed baselines without opening a graphics device. Incompatible dimensions or environments are rejected rather than silently rescaled or compared with relaxed thresholds.
- `pnpm preview:check --out target/preview-ci-001` runs the native scenario and compares it with the repository baseline. Build the counter and native host first.

The checked-in visual baseline is tied to its recorded AMD/RADV/Mesa environment and is not expected to be portable across every GPU and driver. A required graphics check fails when no adapter is available; it is not skipped and reported as successful.

See [Preview implementation and verification](docs/development/preview-verification.md) for all commands, failure categories, artifact contracts, and recorded evidence.

## Application model

- Import native components and reactive primitives from `@zgui/ui`. Native nodes, events, and styles are not HTML or DOM APIs.
- The application entry point is `src/main.tsx`. It exports `hostApiVersion = 1` and `mount(root)`; `mount` returns the disposer supplied by `renderApp`. See the [counter source](apps/examples/counter/src/main.tsx).
- Optional `src/app.css` is packaged separately. Files under `assets/` are copied into the package and declared in its manifest; the host verifies and loads only declared resources.
- `@zgui/services` currently exposes `readTextAsset` and `watchNiri`. Start work from mount, lifecycle, or event callbacks, and cancel subscriptions, requests, timers, and frame callbacks during cleanup.
- The JavaScript environment does not provide `window`, `document`, `fetch`, `process`, or `setInterval`. Supported native styles are a deliberately bounded subset, not full browser CSS.

For API contracts, see the [UI SDK guide](packages/ui/README.md) and [host services guide](packages/services/README.md).

## Checks and tests

```sh
pnpm typecheck
pnpm smoke
pnpm test:build
cargo test --workspace --locked
pnpm test:preview
```

`pnpm test:preview` builds the counter and native host before running the preview integration tests. Pixel-producing checks require a working graphics environment.

Real-desktop integration tests are separate opt-in, ignored tests. They may open windows, use the clipboard, require coordinated keyboard/IME input, or—in output lifecycle tests—change display configuration. Read the [native test notes](docs/development/baseline.md) and [P6 desktop verification record](docs/development/preview-verification.md#p6--real-waylandniri-desktop-verification) before enabling them; do not run every ignored test indiscriminately.

## Repository layout

| Path | Purpose |
| --- | --- |
| `crates/native-host` | Package loading, native hosting, session reload, desktop shell integration, and preview CLI |
| `crates/js-runtime` | Embedded JavaScript execution and runtime boundaries |
| `crates/ui-bridge` | Native node handles, properties, events, and structural operations |
| `crates/host-services` | Rust-backed resource services and niri IPC |
| `packages/ui` | Native TSX SDK, components, renderer adapter, and types |
| `packages/services` | Typed application-facing host services |
| `packages/build` | Type checking, TSX compilation, package publication, and build watching |
| `apps/examples` | Runnable example applications |
| `preview` | Profiles, fixtures, scenarios, pinned fonts, comparison policies, and visual baselines |
| `schemas/preview` | JSON Schemas for preview protocols |
| `tests` | Build, type, native-control, and preview tests |
| `vendor` | Required local patches at pinned upstream revisions |
| `docs/development` | Specifications, implementation baseline, and verification records |

## Status and limitations

Development phases 0–8 and preview phases P0–P6 have recorded completion evidence. These milestones do not imply complete browser or desktop feature parity.

Verified on a real Wayland/niri desktop: native text editing with Chinese IME input, panel placement, compositor transparency, fractional scaling, and monitor removal/restoration.

Current limitations:

- No layer-parent xdg-popup support or native menus extending beyond panel bounds.
- No live audio or notification service backends. Preview fixtures do not certify those integrations.
- Offscreen scenarios do not replace real-desktop testing of focus, clipboard, IME, compositor behavior, or output lifecycle.
- Performance notes are scoped observations from specific environments, not general cross-hardware guarantees.

## Documentation

- [Development specification](docs/development/tsx-native-ui.md) — original native host and TSX requirements.
- [Preview specification](docs/development/preview-mode.md) — original preview-mode requirements.
- [Implementation baseline](docs/development/baseline.md) — pinned dependencies, runtime contracts, and native test guidance.
- [Preview verification](docs/development/preview-verification.md) — preview workflows and recorded acceptance evidence.
