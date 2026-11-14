# zgui, rquickjs, and TSX Development Specification

**Revision:** 1.0  
**Date:** 2026-11-13  
**Primary target:** Linux desktop applications, with a later Wayland shell profile  
**Audience:** The implementing developer and coding agent  
**Suggested repository location:** `docs/development/tsx-native-ui.md`

Build a native UI host in Rust using zgui for the retained UI tree, styling, layout, input, accessibility, and rendering; rquickjs for JavaScript execution; and a build-time TSX compiler for application code. The initial frontend uses Solid Universal through a project-owned SDK. The native bridge remains independent of Solid.

This specification defines the implementation to build. Package names such as `@zgui/ui`, Rust types such as `UiSession`, and host functions described as proposed are project APIs to implement, not existing zgui or rquickjs APIs. Source-backed upstream APIs are identified separately. The dependency revisions below are an audited source baseline; integration acceptance requires the builds and tests in this document.

**Current scope:** This is a general-purpose UI library. Platform and
window-manager support is determined by upstream zgui. The earlier niri service,
panel example, and compositor-specific acceptance gates have been removed; no
optional compositor package or window-manager abstraction replaces them.
Historical verification is retained in [baseline.md](baseline.md), while the
requirements below describe the remaining generic UI and upstream integrations.

## Contents

- [1. Decisions and scope](#1-decisions-and-scope)
- [2. Dependency baseline and reproducibility](#2-dependency-baseline-and-reproducibility)
- [3. Architecture](#3-architecture)
- [4. TSX frontend and build pipeline](#4-tsx-frontend-and-build-pipeline)
- [5. Application and SDK contracts](#5-application-and-sdk-contracts)
- [6. Native node model](#6-native-node-model)
- [7. Properties, styles, semantics, and native controls](#7-properties-styles-semantics-and-native-controls)
- [8. Events and synchronous native behavior](#8-events-and-synchronous-native-behavior)
- [9. rquickjs ownership and bootstrap](#9-rquickjs-ownership-and-bootstrap)
- [10. Scheduling, Promise jobs, timers, and frames](#10-scheduling-promise-jobs-timers-and-frames)
- [11. Asynchronous host services](#11-asynchronous-host-services)
- [12. Lifecycle, cleanup, and reload](#12-lifecycle-cleanup-and-reload)
- [13. Errors, limits, and observability](#13-errors-limits-and-observability)
- [14. Installing the bridge in zgui](#14-installing-the-bridge-in-zgui)
- [15. Linux and Wayland shell profile](#15-linux-and-wayland-shell-profile)
- [16. Development phases and acceptance gates](#16-development-phases-and-acceptance-gates)
- [17. Test plan](#17-test-plan)
- [18. Developer workflow and command contract](#18-developer-workflow-and-command-contract)
- [19. Rules for the implementing coding agent](#19-rules-for-the-implementing-coding-agent)
- [20. Extension decisions](#20-extension-decisions)
- [21. Source index](#21-source-index)

## 1. Decisions and scope

### 1.1 Confirmed requirements and implementation defaults

| Decision | Status | Implementation consequence |
| --- | --- | --- |
| zgui is the native UI foundation | User selected | Keep native rendering and platform integration in zgui. |
| rquickjs embeds the JavaScript engine | User selected | Use its public Rust API and lifetime model. |
| Application UI is authored in TypeScript and TSX | User selected | Compile ahead of runtime; ship JavaScript, CSS, assets, and metadata. |
| Solid Universal supplies the first TSX frontend | Recommended default | Implement its custom renderer contract; keep the host interface replaceable. |
| One owner UI thread executes JavaScript and mutates UI | Recommended default | Native asynchronous tasks send owned data back to that thread. |
| One runtime and one context serve the initial application bundle | Recommended default | Establish correct ownership before adding multiple independent sessions. |
| The first executable uses a regular desktop window | Recommended default | Validate rendering and scheduling before layer-shell integration. |
| The first bundles are locally authored application code | Initial trust assumption | Define explicit host APIs; do not present in-process execution as a hostile-code sandbox. |

These defaults allow implementation to proceed without requiring a separate framework decision. Replacing Solid later should affect the frontend adapter and compiler, not the system-service layer or zgui platform integration.

### 1.2 First release capabilities

The first usable release must support:

- Function components, signals, memos, reactive effects, cleanup, and batching.
- Native TSX elements with text, classes, a documented CSS subset, and explicit property mappings.
- Conditional content and identity-preserving list reordering.
- Synchronous native events, including cancellation while dispatch is active.
- Timers, Promise jobs, and promise-returning asynchronous host services.
- Correct mounting, unmounting, window shutdown, and complete bundle reload.
- Source-mapped application errors and basic runtime/frame measurements.
- A keyboard-accessible button and, after the editing integration gate, native text fields.

### 1.3 Deferred capabilities

Defer the following until the core integration has evidence that it is needed:

- React compatibility or `react-reconciler`.
- Lynx-style dual JavaScript runtimes, first-frame hydration, and worklets.
- A custom static-template compiler beyond the Solid Universal transform.
- Worker-thread JavaScript execution.
- Runtime TSX transpilation, arbitrary npm loading, and unrestricted dynamic imports.
- State-preserving component HMR.
- A general browser DOM, Node.js compatibility layer, or complete web CSS implementation.
- Cross-frame reuse of detached native node handles, until a retention strategy is implemented.
- Solid transitions, deferred computations, and Suspense without a verified scheduler integration.
- A third-party plugin marketplace and hostile-code process isolation.

The last two items are separate extensions. Resource limits and capability checks improve host behavior, but they do not make a JavaScript context an OS security boundary.

## 2. Dependency baseline and reproducibility

### 2.1 Audited source revisions

| Component | Source revision | Observed version | Required treatment |
| --- | --- | --- | --- |
| zgui | `80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a` | Workspace 0.1.0; Rust 1.90 minimum | Pin the git revision initially. |
| rquickjs | `73fcd6062ee615aeb2a4675cd32be67a56e93973` | Manifest 0.14.0; Rust 1.87 minimum | Pin this revision for the source behavior specified here. |
| QuickJS-NG inside rquickjs | `6d46d07d04041b40f4f49eaa7fdebe44c314c699` | Engine submodule revision | Record the actual resolved engine identity in build metadata. |
| Solid and babel-preset-solid | `99714d4e2875d2d7ef8023e82f4ef6aa63ff0752` | Both source manifests declare 1.9.19 | Verify registry availability before selecting exact npm versions. |
| dom-expressions reference implementation | Tag `v0.40.14` | Universal renderer/compiler reference | Inspect the actual transitive version in the lockfile. |

The Solid source manifests establish the examined API, not a validated registry installation. During Phase 0, verify the available package versions, install a compatible exact pair, and record the resolved compiler/runtime matrix. If 1.9.19 is unavailable, choose an available compatible pair and re-run the compiler and QuickJS smoke tests. Do not silently substitute versions.

The current rquickjs README identifies its backend as **QuickJS-NG**. The project may use “QuickJS” as the family name in prose, but build metadata must identify the actual backend. [Z1] [Q1] [Q11] [S1] [S10]

### 2.2 Rust dependencies

Proposed initial declarations:

~~~toml
[dependencies]
zgui = { git = "https://github.com/zortax/zgui", rev = "80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a" }
rquickjs = { git = "https://github.com/DelSkayn/rquickjs", rev = "73fcd6062ee615aeb2a4675cd32be67a56e93973", default-features = false, features = ["std"] }
~~~

Add rquickjs `macro` only when the implementation uses its procedural macros, and `loader` only when implementing a runtime module graph. Start without `full`, `full-async`, `parallel`, `dyn-load`, or `rust-alloc`.

The zgui facade does not expose `wayland` or `winit` Cargo features. At the audited revision its optional features are `instrument`, `tokio`, and `wall-clock`; platform selection is handled separately. [Z2] [Z21] [Q11]

Pin a toolchain satisfying the complete workspace and verify the target build. Do not infer that the lower rquickjs minimum is sufficient for zgui.

### 2.3 Fontique patch choice

zgui's workspace applies a local Fontique patch. Cargo does not automatically inherit a dependency workspace's `[patch.crates-io]` configuration.

The zgui vendor documentation describes the released Fontique as functionally usable, with slower font resolution. Matching the vendored implementation is a deliberate performance/reproducibility choice, not a documented prerequisite for every downstream build.

If adopted, add an application-root patch and validate dependency resolution:

~~~toml
[patch.crates-io]
fontique = { git = "https://github.com/zortax/zgui", rev = "80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a" }
~~~

Commit Cargo.lock, the JavaScript lockfile, the selected package-manager version, and the compiler configuration. Record whether the Fontique patch is active. [Z3]

## 3. Architecture

### 3.1 System boundaries

~~~mermaid
flowchart TB
    Source["TSX, TypeScript, CSS, assets"] --> Build["Type check, Solid transform, bundle"]
    Build --> Package["Versioned application package"]
    Package --> VM["rquickjs session on UI thread"]
    VM <--> Adapter["Native node and event adapter"]
    Adapter <--> Zgui["zgui tree, input, layout, rendering"]
    VM <--> Services["Typed host services"]
    Services <--> Workers["Rust asynchronous tasks"]
    Workers --> Inbox["Owned completion messages and wakeup"]
    Inbox --> VM
~~~

The bridge owns the application-facing contract. zgui owns native UI behavior. Solid owns JavaScript reactivity. The task system owns blocking or asynchronous host work.

A single-threaded bridge does not need a serialized mutation protocol initially. Synchronous operations can mutate zgui's retained tree while layout and painting remain scheduled by zgui. Preserve a clear end-of-turn boundary so that a command transport can be added later.

### 3.2 Proposed workspace

| Path | Responsibility |
| --- | --- |
| `crates/native-host/` | Executable, platform startup, window lifecycle, application loading |
| `crates/js-runtime/` | rquickjs ownership, bootstrap, jobs, errors, interruption |
| `crates/ui-bridge/` | Node handles, topology, properties, events, native scheduling |
| `crates/host-services/` | Typed application/system services and completion messages |
| `packages/ui/` | Proposed `@zgui/ui` SDK, native JSX types, components |
| `packages/ui/src/renderer/` | Solid Universal adapter and compiler helper exports |
| `packages/build/` | Babel/esbuild integration, package validation, watch mode |
| `apps/examples/counter/` | First real TSX application |
| `apps/examples/lists/` | Conditional content, list movement, cleanup |
| `apps/examples/async/` | Timers, Promise work, service completion |
| `apps/examples/editing/` | Native field, selection, composition, controlled values |
| `tests/integration/` | Cross-layer tests for actual failure modes |
| `docs/development/` | Architecture, API contracts, dependency matrix, measured results |

Keep the initial workspace small. These boundaries may begin as modules rather than separate crates; split them when dependency ownership becomes useful.

### 3.3 Proposed runtime objects

| Object | Owns |
| --- | --- |
| `UiSession` | Runtime, context, generation, lifecycle state, application entry/disposer roots |
| `NodeRegistry` | Opaque handle validation and logical topology for JS-owned nodes |
| `CallbackRegistry` | Persistent callback references, listener associations, callback IDs |
| `RequestRegistry` | Pending host requests and persistent Promise resolvers |
| `SchedulerState` | Entry guard, deadlines, pending jobs, timers, frame callback bookkeeping |
| `HostInbox` | Owned completion messages from external Rust tasks |
| `WindowBinding` | The zgui HostBinding implementation and its session association |
| `BundleManager` | Validated package identity, loading, reload generation, last good build |

Use short-lived interior-mutability borrows. A `UiSession` must not be implemented as one large mutable borrow held while JavaScript calls back into the host.

### 3.4 Ownership invariants

1. All JavaScript entry and all zgui node mutation occur on the owner UI thread.
2. A node belongs to one session generation and one native document.
3. JavaScript components do not own the window, GPU device, or system service implementation.
4. External tasks carry owned Rust data and request identifiers, never JS values or native UI handles.
5. A persistent JS value is released before its runtime is destroyed.
6. Only the bridge structurally mutates a JS-owned subtree.
7. Native editing internals remain owned by zgui.
8. No callback is invoked while a mutable callback or topology registry borrow is held.
9. A pending job or completion has an explicit path to wake the host.
10. An idle application requests no artificial continuous redraw loop.

## 4. TSX frontend and build pipeline

### 4.1 Why Solid Universal is the initial frontend

Solid Universal provides a custom renderer interface and compiler output directed to that renderer. It fits a retained native tree without implementing a browser DOM.

The public project SDK should expose a curated native API. Application code imports reactive primitives and native components from `@zgui/ui`; it should not need to understand rquickjs or zgui internals.

This remains a replaceable frontend decision. The host API must not expose Solid owners or computations to Rust services. [S2]

### 4.2 Supported reactive APIs

Initially expose and test:

- `createSignal`, `createMemo`, `createEffect`.
- `onMount`, `onCleanup`, `batch`, `untrack`.
- `For`, `Index`, and `Show`.
- A project `renderApp`/mount wrapper.
- Native component, child, ref, event, and style types.

Add Context, stores, error boundaries, and additional control flow as independently verified features.

Do not export every Solid API automatically. In particular, `createDeferred` reaches a scheduler that uses browser facilities such as MessageChannel and navigator. `enableScheduling(customScheduler)` does not replace every such path. Transitions and Suspense require their own compatibility work. [S7]

### 4.3 Build stages

1. Type-check application code against the native SDK.
2. Transform TSX with Solid's universal generator.
3. Remove TypeScript syntax.
4. Bundle all runtime JavaScript dependencies into one ESM artifact.
5. Emit CSS and assets as separate package resources.
6. Compose source maps across transformations.
7. Validate imports, dependency resolution, runtime globals, and package metadata.
8. Publish the complete build output atomically for the host to load.

Node.js and package-manager dependencies exist on the development/build side. They are not embedded in the shipped UI runtime.

### 4.4 Babel configuration

Proposed configuration:

~~~js
const babelOptions = {
  babelrc: false,
  configFile: false,
  presets: [
    [
      "babel-preset-solid",
      {
        generate: "universal",
        moduleName: "@zgui/ui/renderer",
        hydratable: false
      }
    ],
    [
      "@babel/preset-typescript",
      { onlyRemoveTypeImports: true }
    ]
  ]
};
~~~

Pass the source filename for each transformation so the TypeScript preset distinguishes TS and TSX. Preserve source-map input/output.

Solid must see the JSX structure. Do not run React's automatic JSX transform or esbuild's ordinary JSX lowering before the Solid transform. [S2] [B1]

### 4.5 Bundling configuration

Proposed esbuild settings, with a Babel onLoad integration supplied by the build package:

~~~js
const buildOptions = {
  entryPoints: ["src/main.tsx"],
  outfile: "dist/app.mjs",
  bundle: true,
  format: "esm",
  platform: "neutral",
  conditions: ["browser", "production"],
  mainFields: ["module", "main"],
  target: "es2022",
  splitting: false,
  sourcemap: "external",
  metafile: true,
  define: {
    "process.env.NODE_ENV": '"production"'
  }
};
~~~

The configuration object is an integration recipe, not a complete build script. The Babel plugin must return transformed JavaScript with usable source-map information. For development, deliberately select the development condition and matching NODE_ENV value, then run the same runtime compatibility tests.

The `browser` export condition selects Solid's reactive implementation. It does not create DOM globals. Solid's `node`, `worker`, and `deno` branches select server code; do not add `worker` merely because a future host runs JS on a Rust worker thread.

Inspect the resolved module graph. Require one Solid runtime and exclude `solid-js/web`, React DOM, server implementations, Node built-ins, and unimplemented browser dependencies. The browser condition applies to other packages too, so it is not a general guarantee of runtime compatibility.

An ES2022 target controls syntax lowering; the actual bundle still needs execution tests in the selected rquickjs build. [S1] [B2]

### 4.6 Native TypeScript configuration

~~~json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "Bundler",
    "lib": ["ES2022"],
    "types": [],
    "strict": true,
    "noEmit": true,
    "jsx": "preserve",
    "jsxImportSource": "@zgui/ui"
  }
}
~~~

Publish a `@zgui/ui/jsx-runtime` declaration entry containing the native JSX namespace. Expose only supported tags and properties; a catch-all intrinsic-element index signature would conceal unsupported APIs and spelling errors.

Use a separate TypeScript configuration for build scripts that legitimately use Node APIs.

The SDK's public type entry must declare the exact supported host globals, either directly or through an explicitly included `@zgui/ui/runtime-globals` declaration entry. With ES2022 libraries and types: [], logging and timer APIs are not automatically declared. Declare the implemented console, timer, frame-callback, and microtask signatures without importing DOM or Node ambient libraries.

Solid's existing component and control-flow types reference its own DOM-oriented JSX definitions. Merely changing jsxImportSource is insufficient for a native public type boundary. Publish curated native declarations, including component/child/ref/control-flow types, and avoid leaking upstream DOM declarations through public reexports. The SDK implementation may use a separate typing configuration.

Acceptance requires native TSX to type-check while `document`, `HTMLElement`, browser event types, and unsupported elements fail in application code. [S3]

### 4.7 Package artifacts

| Artifact | Purpose |
| --- | --- |
| `manifest.json` | Package ID, schema/API versions, entry/resources, build identity |
| `app.mjs` | Complete runtime JavaScript bundle |
| `app.mjs.map` | Source mapping for diagnostics |
| `app.css` | Styles loaded by the native host |
| `assets/` | Packaged images, fonts, and other declared resources |
| `build-info.json` | Compiler versions, lockfile identity, engine/host baseline |

The first runtime accepts no unresolved external imports, runtime CSS imports, arbitrary dynamic imports, or top-level await. Validate these properties structurally rather than relying only on substring searches.

Use a package generation directory or temporary output followed by atomic publication. A watcher must never load half of a new bundle with half of an old stylesheet.

Proposed minimal manifest:

~~~json
{
  "schemaVersion": 1,
  "id": "dev.example.counter",
  "hostApiVersion": 1,
  "entry": "app.mjs",
  "styles": ["app.css"],
  "assets": [],
  "buildId": "sha256:<generated-content-digest>"
}
~~~

The build tool fills the real digest. Resolve resources relative to the package root, validate canonical paths, and require the manifest hostApiVersion to agree with the module export. Reject incompatible versions before mounting. The manifest describes requested resources; the host remains responsible for its service policy.

## 5. Application and SDK contracts

### 5.1 Entry module

Proposed application entry:

~~~tsx
import { createSignal, renderApp, Button } from "@zgui/ui";
import type { NativeNode } from "@zgui/ui";

function App() {
  const [count, setCount] = createSignal(0);

  return (
    <column class="counter">
      <text>Count: {count()}</text>
      <Button onClick={() => setCount(value => value + 1)}>
        Increment
      </Button>
    </column>
  );
}

export const hostApiVersion = 1;

export function mount(root: NativeNode): () => void {
  return renderApp(() => <App />, root);
}
~~~

This is the target application API to implement. Button and renderApp are project SDK exports. zgui does not provide this TypeScript package.

Require synchronous mount and an idempotent disposer. Start asynchronous work through lifecycle hooks or event handlers after the host has established the session.

### 5.2 Native child values

A native JSX child may be:

- An opaque NativeNode.
- A string or number, rendered as native text.
- A boolean, null, or undefined, representing empty content.
- An array of supported children.
- A supported accessor returning such content.

The project mount wrapper must deliberately adapt this child type to Universal's narrower declared render signature. Do not solve type mismatches by spreading `any` across application declarations.

### 5.3 Runtime globals

Provide only implemented globals and SDK services:

| API | Initial policy |
| --- | --- |
| `console` | Structured host logging with safe argument formatting |
| Promise and standard language built-ins | Engine-provided |
| `setTimeout`, `clearTimeout` | Host-managed timer registration |
| `requestAnimationFrame`, cancellation | Native frame callback integration |
| `queueMicrotask` | Preserve the engine-provided function in the audited Context::full baseline; verify its behavior |
| `performance.now` | Optional monotonic host clock, if exposed |
| `window`, `document`, `require`, `process` | Absent |
| `fetch` | Absent initially; expose a typed service or implement a defined fetch surface later |
| File/process/system APIs | Only explicitly registered host services |

rquickjs provides an engine binding, not a browser or Node runtime. A dependency must be evaluated against this actual environment. Add a queueMicrotask shim only if a later engine baseline lacks the function; a Promise.then replacement changes thrown-error reporting and must not silently replace the native implementation. [Q1] [Q9]

### 5.4 Separate UI and service APIs

Keep native UI operations separate from system services.

Examples of later service groups are workspace state, audio state, notifications, storage, and launcher requests. These are proposed host modules; zgui is not assumed to supply them.

Each service specifies its argument/result types, cancellation policy, subscription disposal, and failure behavior. Service responses contain data, not native node pointers or JavaScript functions.

## 6. Native node model

### 6.1 Opaque identity

Use an opaque **JavaScript object** for NativeNode.

Do not expose zgui's raw NodeId as a JS Number: the audited NodeId uses 64 bits. A numeric node token also conflicts with Universal's handling of numeric children as text.

A native wrapper or equivalent validated object should identify:

- Session identity.
- Session generation.
- Native document/window identity.
- Bridge-local node token.
- Node kind and lifecycle state.

The native NodeId remains an implementation detail. Validate every operation before reaching zgui. Reject stale, cross-session, cross-document, and fabricated handles with a defined host error.

Preserve wrapper identity while a node is live. Traversal and event targets referring to the same node should return the same logical NativeNode identity. [Z5]

The mount root is a host-owned boundary. JavaScript may manage its assigned children but may not destroy/reparent the host root or traverse into Rust-owned siblings. Logical parent traversal stops at that boundary.

### 6.2 Logical topology

Solid requires synchronous parent, first-child, and next-sibling lookup. The public zgui Dom trait exposes parent and liveness but not first-child or next-sibling traversal.

Maintain adapter-owned topology for JS-owned regions:

~~~text
NodeRecord:
  native_id
  session_generation
  document_id
  kind
  parent
  ordered_children or sibling links
  lifecycle_state
  listener_ids
~~~

Use efficient lookup appropriate to the expected list sizes. Updates to native structure and bridge topology must be coordinated in the same owner-thread operation.

The concrete DocumentDom has a children() reader. It can validate the shadow topology in development, but it returns a vector and should not be the default hot-path sibling traversal.

Treat native editor-managed children as opaque. They do not belong in the JS logical child list. [Z4] [Z6]

### 6.3 Solid Universal mapping

| Universal operation | Proposed adapter behavior | zgui primitive |
| --- | --- | --- |
| createElement(tag) | Validate a supported native tag, allocate an opaque node | create_element |
| createTextNode(value) | Allocate text, including empty text | create_text |
| replaceText(node, value) | Validate text kind and update content | set_text |
| isTextNode(node) | Read bridge node metadata | No native query required |
| setProperty(node, name, value, previous) | Use explicit property dispatch | Attribute/style/state/property/event APIs |
| insertNode(parent, node, anchor) | Insert or move, update topology | insert |
| removeNode(parent, node) | Detach under the lifecycle policy below | detach |
| getParentNode(node) | Return logical parent | Bridge metadata |
| getFirstChild(node) | Return first logical child | Bridge metadata |
| getNextSibling(node) | Return next logical sibling | Bridge metadata |

The renderer module must export the named helpers expected by the compiler: render, effect, memo, createComponent, createElement, createTextNode, insertNode, insert, spread, setProp, mergeProps, and use.

An insertion must move an existing node without duplicating it. Preserve empty text nodes used as insertion positions. zgui's create_marker is useful for host infrastructure, but it is not a general browser Comment implementation and must not silently replace every text node. [S2] [S4] [Z4]

### 6.4 Detachment and reclamation

zgui keeps a detached subtree available through the current frame, then reclaims it if it has not been reinserted. A node created detached and never linked has a different lifetime and can survive frame end. [Z7]

The first bridge must therefore define a narrower lifecycle contract than unrestricted cached-node reuse:

- Complete reorders and moves within one synchronous JS update turn.
- Do not detach a native subtree, await an operation, and then reuse its old handle.
- A branch that is permanently removed invalidates its native handles.
- Application refs are borrowed access to live nodes, not independent native ownership.
- Recreating a branch from application data creates fresh nodes after disposal.
- Cross-frame cached JSX/NativeNode reuse is unsupported until explicit retention is implemented.

Define a synchronous structural turn as one outermost call into JS, such as mount, an event handler, a timer callback, a completion resolver invocation, or one Promise job. A later Promise continuation is a separate turn even when it runs before the next paint.

During the turn, removeNode marks the affected root pending-detach. Reinsertion during that turn cancels pending removal. At the end of the turn, invalidate roots that remain detached and the descendants still belonging to them, unregister node-associated listeners/observers, and release their callback roots and topology records. Descendants moved to a surviving parent remain valid. Native reclamation may occur later at frame end.

Retaining an invalid wrapper as a JS object is safe; using it for a native operation produces a defined stale-handle error. Check native liveness defensively, but do not use native reclamation as a substitute for bridge cleanup.

This restriction affects ordinary cached JSX patterns too. For example, the following is unsupported after the content has been hidden and its removal finalized:

~~~tsx
const cached = <text>Retained content</text>;
return <Show when={visible()}>{cached}</Show>;
~~~

The initial supported form constructs the branch lazily:

~~~tsx
return (
  <Show when={visible()}>
    <text>Fresh content</text>
  </Show>
);
~~~

Verify that form with the actual compiler/runtime matrix and a delayed hide/show test across native frames. Describe compatibility as the documented reactive APIs and TSX patterns, rather than arbitrary Solid application compatibility. [S11]

For later keep-alive support, choose a tested retention or rematerialization design. A hidden parking subtree may preserve native resources, but it also needs explicit ownership and reclamation; retaining every removed node until application exit is not acceptable.

### 6.5 Construction cleanup

Track every allocation made during mount or a structural update. Aborted construction must reclaim nodes even if they were never attached.

Calling detach on a never-linked root does not schedule reclamation: the native removal path returns when there is no parent. Use this public-API cleanup policy:

1. Identify orphan roots owned by the failed allocation journal or retiring session.
2. Create a temporary cleanup box.
3. Insert the cleanup box under a valid host-owned/session root.
4. Insert only those owned orphan roots under the cleanup box.
5. Detach the cleanup box during the same synchronous turn.
6. Release associated bridge metadata and callback roots.
7. Let normal frame-end reclamation collect the box and descendants, requesting a frame if required.

The cleanup box must itself have a parent before detachment. Do not adopt surviving preexisting nodes just because an unfinished move temporarily detached them. Do not call DocumentDom::end_frame from a callback to force collection; frame reclamation belongs to the native loop.

When a window/document is being destroyed, its normal document destruction completes native reclamation. A failed update that already changed existing nodes is not rolled back merely by freeing new allocations. Without a real rollback journal, fault and tear down that session. [Z20] [Z23] [Z24]

## 7. Properties, styles, semantics, and native controls

### 7.1 Initial element vocabulary

Expose a deliberate subset of zgui's native vocabulary:

| Native TSX tag | Purpose |
| --- | --- |
| box | General container |
| row | Horizontal flex container |
| column | Vertical flex container |
| stack | Stacking container |
| text | Inline text element containing primitive text nodes |
| label | Label semantics/layout as supported by zgui |
| scroll | Native scrolling container |
| control | Base interactive control |
| image | Native image element after asset-loading integration |
| field / editor | Native editable controls after the editing gate |

Native names also include vector, canvas, surface, and spacer, but their specialized contracts should be added separately.

There is no built-in HTML button/input tag in the audited vocabulary. The SDK may offer Button, TextField, and TextArea components with explicit mappings. [Z8]

### 7.2 Property dispatch

Use an explicit property table rather than converting every value into a string attribute.

| Property category | Required behavior |
| --- | --- |
| class | Normalize and replace the native class list |
| style | Apply supported CSS properties; remove keys that disappear |
| CSS custom property | Use the native custom-property path |
| Attribute | Set or remove through the typed attribute API |
| Native state | Map to UiState/custom state |
| Native property | Convert to PropValue; clear with PropValue::Unset |
| Accessibility | Set/clear Semantics |
| Event | Register, replace, or remove a listener and callback root |
| ref | Handled by the frontend; expose a NativeNode |
| children | Handled by reconciliation, not generic setProperty |

Use null/undefined clearing semantics consistently. A spread property removed on the next update must remove its previous native value.

Start with one canonical class prop spelling. If className is added as an alias, define conflict behavior when both are supplied.

### 7.3 CSS contract

Use zgui's actual stylesheet and property support. Publish a supported-property table for the SDK examples rather than claiming full web CSS compatibility.

Prefer string CSS values initially, including explicit units. Avoid inventing browser-style numeric unit coercion.

For style objects, diff previous and next keys. For a switch from a style string to an object or vice versa, clear the previous representation deliberately. Otherwise old native declarations can remain active.

Load package styles through the host's verified stylesheet integration before mounting. CSS file watching should update the package generation coherently with JavaScript. CSS Modules or utility-class generation can be added after the basic cascade and asset paths are validated.

### 7.4 Button component

Implement Button over native control with:

- Button accessibility semantics.
- Disabled state through UiState::DISABLED and matching semantics.
- A documented focus and keyboard activation policy.
- Native hover/pressed/focus styling where available.
- One activation callback with no duplicate pointer/keyboard dispatch.

Start with the Rust Button's native structure and state, including sequential tabindex and Role::Button semantics. The audited Rust Button does not add its own Enter/Space click synthesizer; native control defaults provide pointer and non-repeating keyboard activation. Reuse those defaults and test exactly-once activation before adding custom handlers. [Z9] [Z22]

Keep reactive props reactive in SDK components: read through props or a getter-preserving utility rather than destructuring values into stale locals.

### 7.5 Text field and editor ownership

Use the public ViewHost::set_value path for controlled field/editor values. Do not rewrite their text children on every signal update, and do not assume a generic `value` property assignment updates the editing model.

zgui's native editing path coordinates value, caret, and composition-related state. User changes use Input events; committed changes use Change. Programmatic value changes do not generate another input event.

Field/editor descendants managed by the editor remain outside the bridge's reconciled child topology. The initial field/editor components accept value/defaultValue and reject user-authored reconciled children, including at the type level.

Acquire the UI-thread HostHandle through BuildCx::host_handle() or by cloning Window::host_handle(). It dereferences to ViewHost, so the bridge can call set_value through the public handle.

Define value, default value, selection/measurement, and composition behavior as a separate control contract. Verify Chinese IME, clipboard, selection, deletion, and cursor preservation before treating controlled input as complete. [Z10] [Z16] [Z18]

## 8. Events and synchronous native behavior

### 8.1 Use native listeners

Register JS-facing handlers through Dom listeners. HostBinding::before_dispatch only receives a target and event kind; it does not supply the complete event payload. Some synthetic native events also bypass that hook.

The actual native input path releases document/layout borrows before invoking listeners. Listener handles are cloned before invocation, allowing handlers to mutate the same native tree or replace registrations. The bridge must preserve this property by releasing its own registry borrows before entering JS. [Z11]

### 8.2 Event objects

Define native event types, not browser Event subclasses. Copy the applicable data:

- Type, timestamp, phase, target, and currentTarget.
- Pointer position/buttons and supported coalesced samples.
- Keyboard key/code and modifiers.
- Input/change value data for native editing controls.
- A dispatch token for cancellation methods.

The native EventCx is borrowed and must never be stored in JavaScript. Event-control methods operate on a short-lived bridge token. Copy ordinary event data so users can retain it, but invalidate cancellation authority as soon as the synchronous handler returns.

Define target retargeting for native control internals: an event from an editor-owned descendant must expose the public control handle rather than fabricate a JS-owned handle for an internal node. Preserve the actual currentTarget corresponding to the registered listener.

Initially support an explicit mapping such as onClick, onPointerDown, onPointerUp, onKeyDown, onKeyUp, onInput, onChange, onFocus, and onBlur only after matching each to the pinned native event vocabulary.

### 8.3 Cancellation and propagation

Implement preventDefault, stopPropagation, and stopImmediatePropagation only with the corresponding native behavior.

For an ordinary native listener:

1. Validate the node, generation, and callback ID.
2. Clone the persistent function from the registry and release the registry borrow.
3. Copy the native event payload.
4. Activate a dispatch-control token.
5. Enter the owning JS context and call the handler.
6. Capture any exception while the context is active.
7. Invalidate the token.
8. Copy the accumulated cancellation/propagation flags into EventCx.
9. Return from native dispatch after completing the permitted end-of-turn work.

Native default editing, focus, scrolling, and activation decisions occur after the listener returns. Therefore cancellation cannot be delivered later through an asynchronous queue.

An async handler's return value does not delay native dispatch. Calling preventDefault after an await must produce a defined expired-dispatch error or documented no-op; prefer a diagnostic error in development.

### 8.4 Reentrancy policy

Normal events enter JavaScript synchronously so cancellation works.

If JavaScript is already executing, do not call Context::with recursively. Prefer host commands that defer the event-generating native action to a safe point. Unexpected nested notifications may be queued, but they cannot claim the ability to cancel a default action that already occurred.

This is a semantic requirement, not merely a lock workaround. Test the actual focus, editing, and scroll behavior.

### 8.5 Handler replacement and options

Replacing onClick must release the old callback association and install the new one. Clearing an event property, including removal through a spread update, unregisters the listener.

Initially expose capture deliberately. The audited native ListenerOptions contains once/passive fields, but their presence alone does not establish that the dispatch path enforces them. Either implement these semantics in the bridge with tests or reject them until supported.

Do not inherit browser delegation or special web event syntax from examples of Solid's DOM renderer. Universal routes ordinary event props through the custom setProperty implementation. [S5] [Z11]

## 9. rquickjs ownership and bootstrap

### 9.1 Runtime and context

Use one synchronous Runtime and one Context for the initial UiSession.

Verified upstream entry points include:

~~~rust
Runtime::new()
Context::full(&runtime)
context.with(|ctx| { /* JavaScript operations */ })
~~~

Context::with acquires the runtime's internal access guard. In the default configuration the guard is based on RefCell; with parallel it uses a mutex.

Never call another Context::with or a locking Runtime method for the same runtime from inside an active context entry. Native functions invoked by JS use the Ctx already supplied to them. [Q2]

The host may run Rust async I/O elsewhere without using rquickjs AsyncRuntime. These are separate decisions.

### 9.2 Callback roots

Keep callbacks that survive a context entry in Persistent handles. The proposed native callback registry can map a bridge CallbackId to a Persistent Function and its generation/owner information.

Use this discipline:

1. Borrow the registry briefly.
2. Validate identity and clone the Persistent handle.
3. Release the registry borrow.
4. Enter the correct context, unless already in a native callback with its Ctx.
5. Restore the cloned handle.
6. Invoke the function and capture the result/exception.
7. Release temporary restored values before leaving the context.

Persistent::restore consumes the handle, so clone the registry's copy when it must remain registered. Native listeners capture callback IDs and weak session references rather than owning independent long-lived JS functions.

No Persistent may outlive its runtime. The upstream documentation explicitly warns that destroying a runtime with surviving persistent links can abort the process. [Q3]

### 9.3 Module loading

Install the native bridge and supported globals before application evaluation.

Use the module API deliberately:

- Module::declare creates a declared module.
- Evaluating that module returns an evaluated module handle and an evaluation Promise.
- Module::evaluate directly returns an evaluation Promise, not a module namespace.

Use declare/eval when the host needs exports such as hostApiVersion and mount. Observe evaluation completion before using the application. An Evaluated type state does not prove that asynchronous module evaluation has settled.

The initial package contract forbids top-level await and unresolved imports, which keeps bootstrap manageable. Preserve bootstrap state across turns if the engine still has pending jobs; never block the window waiting for host I/O. [Q4]

### 9.4 Bootstrap sequence

1. Validate package schema, host API compatibility, resource paths, and artifact identity.
2. Create the runtime and configure limits, interruption, and rejection tracking.
3. Create the context.
4. Install console, scheduling functions, and the internal native bridge.
5. Evaluate the self-contained ESM artifact.
6. Observe its evaluation Promise using the normal bounded job mechanism.
7. Read and validate required exports.
8. Create or obtain the session's dedicated native mount root.
9. Load its stylesheet/resources through the host integration.
10. Enable mount-time APIs, enter Mounting state, call mount(root), and retain its disposer.
11. Settle initial reactive/native updates.
12. Enter Mounted state, enable external completion delivery, and request the first frame through the host lifecycle.

If any step fails, clean up all allocations and JS roots already created. Do not leave a partially initialized runtime registered as the active window binding.

### 9.5 Runtime loader extension

A custom loader is unnecessary for the first bundled artifact. If added later, define allowed specifiers, canonicalization, import attributes, package-relative resolution, and failure behavior.

The audited rquickjs Resolver and Loader signatures include an import-attributes argument. Verify the pinned signatures instead of copying an older example.

Bytecode caching is also deferred. Module::load is unsafe and expects valid compatible bytecode. A future cache must be tied to the exact engine/build identity and recover from cache misses using known source. [Q4] [Q8]

## 10. Scheduling, Promise jobs, timers, and frames

### 10.1 Keep three scheduling systems distinct

| System | Responsibility |
| --- | --- |
| Solid reactivity | Track dependencies and run affected computations |
| JavaScript host loop | Deliver events/completions and execute Promise jobs fairly |
| zgui frame loop | Input routing, native scheduling, style/layout, paint, reclamation |

Solid batch groups reactive writes; it is not a native frame transaction and does not extend across an await.

A native retained-tree mutation need not trigger immediate layout. Let zgui schedule its own style/layout work.

### 10.2 Actual zgui hook timing

At the audited revision the relevant order is:

1. Input dispatch.
2. Native timers.
3. Native frame callbacks.
4. Animation and Rust reactive flushing.
5. HostBinding::checkpoint.
6. Pending host commands.
7. Restyle, layout, and observations.
8. HostBinding::before_paint.
9. Paint and frame-end recycling.

Use checkpoint for bounded JS work that should affect the upcoming style/layout pass.

Do not implement general requestAnimationFrame using before_paint: it runs after layout. Use ViewHost::request_frame_callback and cancel_frame_callback, which run before the style/layout work.

The frame callback queue drains its current batch before invoking callbacks, so re-registering schedules a later frame. Occlusion and display pacing belong to the native callback system. [Z12]

### 10.3 Per-event settling

HostBinding::checkpoint does not run between every queued native input event. The native between-event path settles Rust reactive work and host commands, not an arbitrary JS job loop.

Therefore apply synchronous Solid updates and finish the bridge's mutation bookkeeping before the JS listener returns. Where Promise jobs are pumped after that JS entry, invalidate the event's cancellation token first.

Use the frame checkpoint as an additional opportunity to drain jobs and completions, not as the only place where an event's JS-generated tree changes become coherent.

### 10.4 Bounded job pump

The preferred upstream host API is:

~~~rust
Runtime::execute_pending_job()
    -> Result<bool, JobException>
~~~

Interpret Ok(true) as a job executed, Ok(false) as no pending job, and Err(JobException(context)) as an exception associated with that context.

Run this API outside Context::with. After an error returns and the runtime guard has been released, enter the exception's context to retrieve the thrown value.

Do not base the production scheduler on Ctx::execute_pending_job: at the audited revision it returns only bool and consumes thrown job exceptions internally. [Q5] [Q9]

Proposed scheduler algorithm:

~~~text
For each eligible owner-thread turn:
  deliver a bounded number of host completions or scheduled callbacks
  exit every context entry before calling Runtime methods
  pump jobs until count or elapsed-time budget is exhausted
  capture and classify job exceptions
  finalize synchronous structural bookkeeping
  arrange native style/layout through normal dirty-state scheduling
  if work remains, request a real follow-up wake
  otherwise allow the host to sleep
~~~

A limit checked between jobs cannot stop one infinite job. The interpreter interrupt handler must also enforce the current entry deadline. Count attempted jobs that return errors against the job budget, and clamp each job's interrupt deadline to the earlier of its normal entry deadline and the pump's remaining deadline. Repeated exceptions or freshly reset per-job budgets must not evade the turn bound.

This is a bounded native-host policy, not a claim of exact browser event-loop ordering.

### 10.5 Timers

Implement setTimeout using ViewHost::schedule and cancellation using cancel_timer. Associate each timer with session generation and a callback root.

Use the native one-shot repeat mode for setTimeout. HostHandle is obtained from the custom root/window; it is distinct from the retained platform waker.

Specify that delays are minimum scheduling delays, not exact deadlines. A callback can be delayed by native work or application load.

On firing:

1. Remove or mark the timer as fired before invoking JS.
2. Validate session state and generation.
3. Invoke the callback through the standard entry guard.
4. Pump permitted Promise jobs afterward.
5. Release a one-shot callback root even if the callback throws.

Prefer a recursive timeout for periodic work initially. If setInterval is introduced, define drift, coalescing, and missed-tick behavior instead of accumulating an unbounded backlog.

### 10.6 Frame callbacks and measurements

Map requestAnimationFrame to the native pre-layout frame callback API. Use the native timestamp, with an explicit conversion to the JavaScript time unit and origin.

Keep frame callback IDs scoped to a session generation. Cancellation and unmount remove their callback roots.

A ref or Solid onMount does not imply native layout or presentation is complete. Provide a separate asynchronous measurement/observation API when needed. Results should identify the node and relevant layout/frame version.

Do not synchronously write layout-affecting properties from a layout observation callback unless the host explicitly supports and bounds that feedback path. Queue resulting JS work for a safe phase.

### 10.7 Wakeups

A queued completion does not wake a sleeping UI by itself.

Capture a platform waker through PlatformCx::waker and route completions to the affected surface. The runtime's ReactiveWork wake path can request frames for surfaces; Window::request_frame coalesces frame requests.

Native timer/frame registrations are made on the owner thread during a host turn. External tasks must wake the platform explicitly. Budget exhaustion must also arrange a follow-up turn.

Native DOM mutation records dirty/redraw state; it is not the external wake transport. Every owner-thread path that mutates the DOM outside a running frame must request a frame for the affected window. After handling a completion or job continuation, the wrapper requests that frame even if the JS job queue is now empty.

Keep these handles distinct: HostHandle is an Rc-based owner-thread ViewHost interface; PlatformCx::waker supplies the Arc-based external wake handle; Window::request_frame requests a coalesced redraw. HostBinding runs when the frame machinery reaches it and does not create an independent event loop. [Z25]

Verify behavior with an otherwise idle window and without mouse or keyboard input. Also verify the policy for occluded windows: visual updates may wait for visibility, but housekeeping and cancellation must not depend indefinitely on a paint callback. The application wrapper must have a bounded owner-thread delivery path on wake, independent of actually painting; clone needed handles and release window/runtime borrows before entering JS. [Z13]

## 11. Asynchronous host services

### 11.1 Promise capability pattern

A synchronous rquickjs Runtime can expose a Promise-returning host method without enabling its futures integration.

The current Ctx::promise API returns a Promise plus resolve and reject functions. The host may save the resolver functions and complete them later on the owner thread. [Q9]

Use this sequence:

1. Validate JS arguments and copy them into owned Rust values.
2. Create the Promise capability.
3. Store resolver roots under a request ID and generation.
4. Submit the owned request to a Rust task executor.
5. Return the Promise immediately.
6. The worker sends an owned result into HostInbox and wakes the owner thread.
7. The owner validates request state and generation, then removes the pending entry or marks it settled.
8. Take the required resolver handle, release the registry borrow, and invoke resolve/reject inside the correct context.
9. Release resolver roots on both success and failure; duplicate completions cannot settle the request again.
10. Pump resulting Promise jobs and schedule affected UI work.

~~~mermaid
sequenceDiagram
    participant JS as JavaScript
    participant Bridge as UI bridge
    participant Task as Rust task
    participant Loop as Native loop
    JS->>Bridge: Call typed async service
    Bridge->>Task: Owned request and generation
    Bridge-->>JS: Promise
    Task->>Loop: Completion and wakeup
    Loop->>Bridge: Safe owner-thread delivery
    Bridge->>JS: Resolve or reject
    Bridge->>JS: Execute pending jobs
~~~

### 11.2 Message contract

A proposed completion message contains:

~~~text
session_id
generation
request_id
result_or_error
~~~

Do not move Ctx, Value, Function, Persistent, DomHandle, or NativeNode objects into worker tasks. Keep native UI and JS resources on the owner thread even if a Cargo feature makes selected runtime types Send.

Reject or ignore stale completions without touching the new session. Define cancellation as best effort at the worker level and definitive at the delivery level.

### 11.3 AsyncRuntime is a separate design

Enabling the futures feature alone does not make async Rust functions work with a plain Runtime. The synchronous runtime has no async spawner; the audited implementation rejects attempts to use that path.

If AsyncRuntime/AsyncContext is adopted later, integrate its task driver intentionally and remove competing scheduler assumptions. Do not await an unbounded idle operation inside a UI frame. [Q6]

### 11.4 Subscriptions

A subscription must return an explicit unsubscribe handle. Register it with the application's disposal scope.

Native data updates should be normalized before updating Solid state. For lists, preserve object identity or use an explicit keyed reconciliation layer. Solid For tracks item value identity, not a React-style key prop; repeatedly parsing fresh objects can recreate rows despite identical IDs.

Use Index only when positional identity is intended. [S6]

## 12. Lifecycle, cleanup, and reload

### 12.1 Session states

~~~mermaid
stateDiagram-v2
    [*] --> Created
    Created --> Loading
    Loading --> Mounting: Module preflight succeeds
    Mounting --> Mounted: Mount succeeds
    Mounting --> Faulted: Mount fails
    Loading --> Faulted: Bootstrap fails
    Mounted --> Stopping: Reload or window close
    Mounted --> Faulted: Unrecoverable session failure
    Faulted --> Stopping: Cleanup
    Stopping --> Disposed
    Disposed --> [*]
~~~

A reload creates a new generation. It does not revive a disposed context or reassign old callback/node IDs to the new application.

### 12.2 Reactive cleanup and native cleanup

Solid Universal's render disposer releases its reactive root but does not explicitly clear the mount container.

The project renderApp disposer must therefore coordinate both:

- Dispose the Solid root and its cleanup callbacks.
- Remove the application's native subtree.
- Unregister listeners, timers, frame callbacks, observers, requests, and subscriptions.
- Release persistent JS roots and node metadata.
- Invalidate old handles.

Make the disposer idempotent. Returning a function from an onMount callback is not cleanup registration; use onCleanup. Callback refs also do not imply React-style invocation with null on unmount; retained refs follow the explicit stale-handle policy. [S4] [S8]

### 12.3 Shutdown order

Use an explicit shutdown procedure:

1. Mark the session Stopping and stop external delivery.
2. Invalidate the generation for new external messages.
3. Cancel requests and subscriptions.
4. Invoke the JS disposer under a bounded cleanup entry while its context is alive.
5. Continue cleanup even if that disposer throws.
6. Remove native listeners, timer/frame registrations, observers, and remaining native nodes.
7. Clear callback, resolver, entry, disposer, and other persistent-value registries.
8. Remove bridge-owned JS roots and release every retained context clone.
9. Drop the context.
10. Drop the runtime.

During cleanup, allow required remove/cancel operations from the retiring generation while rejecting creation of new work. Otherwise generation invalidation could prevent the disposer from cleaning its own resources.

Do not run JavaScript from native/JS GC finalizers. Finalization may release bookkeeping or enqueue a later native cleanup, but explicit disposal is the primary ownership mechanism. [Q3] [Q8]

### 12.4 Full bundle reload

The first development reload is a complete application reload:

1. Build and type-check a new package generation.
2. Validate all artifacts before changing the active application.
3. Evaluate the new module in an explicit preflight bridge mode.
4. Keep the old application active if parsing/evaluation fails.
5. At a safe owner-thread boundary, dispose the old session.
6. Enable mount-time APIs, enter Mounting, and mount the new session into a clean dedicated root.
7. If mounting fails, show a Rust-owned diagnostic/fallback view.

Preflight permits pure JS initialization, version inspection, and diagnostics, but rejects UI allocation, timer/frame registration, and host requests unless a deliberate staging implementation owns them. Prohibiting top-level await alone does not prevent top-level side effects. Dispose all candidate roots and any staged resources after a failed preflight.

Enable mount-time APIs before calling mount because Solid onMount may synchronously register timers or start services inside that call. Defer external completion delivery until mounting returns successfully.

This guarantees clean generations, not state-preserving HMR or automatic rollback of arbitrary side effects. Keep the last good artifact for an explicit retry/recovery path.

A future staged visual swap may preserve the old tree until the new tree is ready, but it needs a separate resource and side-effect policy.

### 12.5 Window binding replacement

There is one HostBinding slot per zgui window. Installing a replacement does not automatically invoke the previous binding's shutdown hook or request a new frame.

Window close invokes the current binding's shutting_down before unmounting the Rust root. Use this ordering, but explicitly handle replacement and reload.

If multiple windows later share one runtime, closing one window must not destroy resources still owned by another. The initial single-window/single-session implementation avoids that additional ownership problem. [Z14]

## 13. Errors, limits, and observability

### 13.1 Error categories

Report these categories separately:

| Category | Required context |
| --- | --- |
| Build/type error | Source file, diagnostic, package build ID |
| Module/bootstrap error | Bundle ID, phase, source-mapped stack |
| Synchronous callback exception | Event/timer/request source and callback ID |
| Pending-job exception | Session generation and recovered JS exception |
| Unhandled Promise rejection | Reason, handling status, source context where available |
| Host validation error | Operation, expected contract, rejected handle/property |
| Interruption | Entry kind, configured deadline, elapsed time |
| Resource exhaustion | Allocator/limit configuration and affected session |
| Native integration failure | Window/document identity and native operation |

Capture exception details while their context is valid, then retain owned diagnostic data. Bound formatting work and handle exceptions thrown during formatting.

Install the runtime's host Promise rejection tracker. It runs during engine activity: use its supplied Ctx and do not re-enter Context::with. Track later handling so diagnostics can distinguish a transient unhandled state from a persistent rejection. [Q7]

### 13.2 Execution limits

Install a runtime interrupt handler once and update a small deadline/cancellation state before each JS entry or job turn.

The interrupt callback must remain short. It must not enter JS, mutate UI, or acquire the runtime again.

Interruption aborts an execution; it is not a resumable continuation. A mount interrupted halfway through becomes a failed mount requiring cleanup. A structurally incomplete update should fault the session or enter a defined recovery path.

Native functions must also be short and nonblocking. A JS interrupt hook does not preempt an arbitrary blocking Rust host call. [Q5]

### 13.3 Initial configurable policy

The implementation should expose named configuration values for:

- Normal JS entry deadline.
- Bootstrap and cleanup deadlines.
- Maximum jobs and elapsed time per pump.
- Maximum outstanding requests and queued completions.
- Maximum timers, subscriptions, callbacks, and nodes per session.
- JS memory limit and maximum stack size.
- Diagnostic rate limiting.

Choose initial values during Phase 0 and record them with the benchmark environment. They are host policy, not claims about expected application performance.

With the audited default allocator, set_memory_limit can configure an engine allocation limit. The upstream documentation says this call is a no-op with a custom allocator, including rust-alloc. Verify the chosen configuration with an actual exhaustion test.

Engine allocation limits do not account for all native nodes, image buffers, fonts, queued Rust data, or GPU resources. Account for those separately if quotas are part of the product. [Q5]

### 13.4 Measurements

Record enough information to locate costs:

- Bundle read and JS evaluation time.
- Mount and first native frame time.
- JS callback and job-pump duration.
- Host mutation count and time.
- Style/layout/paint timing where the native instrumentation exposes it.
- Callback, timer, request, observer, and live-node counts.
- JS heap accounting and process memory separately.
- Outstanding wake requests and idle wake frequency.

Report distributions and the tested hardware/build configuration. Do not advertise engine or renderer speed from a counter demo alone.

Use the native inspector/instrumentation where available, and retain source-map links from application diagnostics. A TSX source-location-to-node mapping is a useful later enhancement.

## 14. Installing the bridge in zgui

### 14.1 Public integration seam

The audited public path includes:

~~~text
zgui::App::into_handler
  Handler::into_runtime
  Runtime::windows_mut
  Window::install_binding
~~~

BuildCx::dom_handle() and host_handle() return cloned handles to a custom Rust root view. A Window also exposes its concrete Dom, DomHandle, and HostHandle. Clone the host handle before releasing the temporary window borrow; it supplies set_value, schedule, and request_frame_callback through ViewHost. [Z16]

Implement an application-handler wrapper that delegates native lifecycle calls and installs a binding when a window/document is actually created.

Windows may appear during surfaces_available, surface_event, or wake processing. Scanning only once at startup is insufficient. Recreated surfaces have new documents and require fresh session/node identity. [Z14]

### 14.2 Initial integration proof

Before building the full SDK, demonstrate:

1. A Rust root creates a dedicated JS mount region.
2. The wrapper captures the correct DomHandle and HostHandle.
3. The binding is installed before the first redraw.
4. A script adds and updates a native text node.
5. A timer updates that text while the window is otherwise idle.
6. Closing the window tears down the session cleanly.

If public ergonomics are insufficient, add a small, explicit factory/accessor extension in an isolated patch. Record why it is needed and keep the bridge independent of unrelated zgui internals.

### 14.3 Hook responsibilities

| Hook | Bridge responsibility |
| --- | --- |
| before_dispatch | Normally allow native dispatch; optional narrow bookkeeping |
| checkpoint | Bounded pending jobs/completions before style/layout |
| before_paint | Read-only timing/diagnostic work; no general layout-changing rAF |
| shutting_down | Begin explicit session teardown |

Do not intercept all native events simply to build a second event dispatcher. Normal listeners already provide target, phase, payload, and cancellation behavior.

## 15. Linux and Wayland shell profile

### 15.1 Ordinary desktop window first

Use the ordinary desktop path as the integration baseline. zgui::app::desktop selects the platform; ZGUI_PLATFORM can explicitly select wayland or winit.

The audited facade does not silently fall back after a platform connection error. Record which backend is actually used in diagnostics and tests. [Z2]

### 15.2 Layer-shell extension

The platform API includes SurfaceRole::Layer and placement information such as anchors, margins, layer, exclusive zone, keyboard mode, and monitor.

However, the high-level App/WindowOptions API does not expose every layer-role setting. A lower-level runtime path or a small reviewed API extension is required.

Treat these as separate tasks after the TSX runtime works:

- Create the intended layer surface on a compositor supported by upstream zgui.
- Verify exclusive-zone behavior and monitor selection.
- Handle fractional scale, monitor removal, and surface recreation.
- Define keyboard focus policy.
- Verify popups and menus using the actual parent surface type.

The audited Wayland popup path requires an xdg_surface parent and rejects a layer parent. An in-surface overlay is a different mechanism and does not establish support for an out-of-surface panel popup. [Z15]

### 15.3 Resource services and platform boundary

Expose manifest-declared resource access as typed host services, separate from
the node renderer. Keep request cancellation and owner-thread delivery within
the application lifecycle.

Do not add window-manager IPC, optional compositor packages, or a window-manager
abstraction to this UI library. Applications may build their own integrations;
platform and compositor capabilities remain the responsibility of upstream zgui.

## 16. Development phases and acceptance gates

Implement each phase as a reviewable change. A later phase must not conceal a failing lifecycle or scheduling gate from an earlier phase.

### Phase 0 — Establish the build and integration baseline

**Deliverables**

- Pinned Rust and JavaScript dependencies with lockfiles.
- A recorded compiler/runtime compatibility matrix.
- A normal zgui window built as a downstream application.
- A minimal rquickjs context executing a script and reporting an exception.
- A custom Rust mount root and an installed HostBinding.
- A verified native wake path.

**Acceptance**

The actual downstream build starts on the target Linux session, displays native text, and closes cleanly. The team has explicitly selected the Fontique patch policy and verified the resolved package graph. A signal/effect smoke test uses the reactive Solid implementation inside rquickjs.

### Phase 1 — Implement native handles and structural operations

**Deliverables**

- Opaque NativeNode wrappers.
- Session/document/generation validation.
- Element/text creation, text replacement, insertion, movement, and removal.
- Logical topology and debug consistency checks.
- A deterministic detached-node policy.
- Aborted-construction cleanup, including never-linked nodes.

**Acceptance**

Mixed text/elements and empty anchors behave correctly. A move preserves native identity within a turn. Wrong-document and stale handles fail predictably. A failing mount returns all allocation counters to the expected baseline after native reclamation.

### Phase 2 — Compile and render TSX

**Deliverables**

- The proposed @zgui/ui/renderer helper exports.
- Babel Universal transformation and esbuild bundling.
- Native JSX types and a curated reactive API.
- renderApp with coordinated reactive/native disposal.
- Counter, conditional-content, and list examples.

**Acceptance**

The bundled app runs in rquickjs, updates without rebuilding the entire tree, and contains no unresolved imports or accidental browser renderer. Supported conditional patterns pass delayed hide/show tests across native frames. The cached-node restriction is enforced and documented.

### Phase 3 — Add properties, events, and basic controls

**Deliverables**

- Explicit property/style/attribute/state/semantics dispatch.
- Handler replacement and removal.
- Event snapshots and live dispatch-control tokens.
- Button component using native control behavior.
- Source-linked event errors.

**Acceptance**

Removing a spread property clears native state. Replacing an event callback invokes only the new function. preventDefault suppresses a real native default action. Pointer, Enter, and Space activation produce exactly one intended action, and disabled controls do not activate.

### Phase 4 — Complete scheduling and host async work

**Deliverables**

- Bounded pending-job pumping with exception preservation.
- Native timers and native pre-layout frame callbacks.
- Request/resolver registry and HostInbox.
- Explicit wakes for completions and unfinished work.
- Deadline interruption and bounded-queue policies.

**Acceptance**

A timer and a Rust task update an idle window without input. A Promise job changes layout before the intended native layout pass. Infinite JS and microtask chains cannot indefinitely monopolize the owner thread. Failed jobs retain useful diagnostics.

### Phase 5 — Make teardown and reload reliable

**Deliverables**

- Idempotent disposal.
- Candidate preflight mode and full bundle reload.
- Generation validation for all late deliveries.
- Explicit HostBinding replacement.
- Rust-owned fallback UI.

**Acceptance**

Repeated mount/unmount/reload does not produce a monotonic increase in retained callbacks, timers, requests, or native nodes. A stale completion never updates the new generation. Evaluation failure keeps the active app; mount failure reaches a clean fallback state.

### Phase 6 — Integrate native editing and accessibility

**Deliverables**

- TextField/TextArea contracts over native field/editor.
- Controlled-value updates through ViewHost.
- Input/change event mapping.
- Keyboard focus behavior and native semantics.
- Layout observation/measurement API if required.

**Acceptance**

Typing, selection, clipboard operations, deletion, undo where exposed, and Chinese IME composition behave coherently. Controlled updates preserve the expected caret and do not produce feedback-loop input events. Accessibility semantics remain associated with the correct live nodes.

### Phase 7 — Validate the desktop shell profile

**Deliverables**

- Layer-shell creation through a chosen public/lower-level integration path.
- Monitor, scale, focus, and exclusive-zone configuration.
- A documented popup/menu strategy compatible with layer surfaces.

**Acceptance**

The generic wrapper passes layer configuration to a supported upstream zgui backend, including monitor, scale, focus and output lifecycle behavior. Layer popup limitations are explicit rather than hidden by a normal-window demo. A particular compositor or window-manager service is not a library acceptance requirement.

### Phase 8 — Optimize measured bottlenecks

**Possible changes**

- Property-write coalescing.
- Reduced bridge conversion/allocation overhead.
- Static template or slot-based creation.
- Explicit retained-node leases.
- JS worker execution with logical topology and batched commands.
- Versioned bytecode cache.
- Multiple independent sessions.

**Acceptance**

Each optimization targets a recorded bottleneck, preserves the existing correctness gates, and includes before/after measurements on the same workload.

## 17. Test plan

These tests target integration risks. Reuse upstream tests for upstream behavior; do not reproduce entire JS language, Solid, or zgui test suites.

### 17.1 Required cross-layer tests

| ID | Scenario | Required result |
| --- | --- | --- |
| T01 | Build a TSX counter and execute it in the real rquickjs host | Initial text renders and a signal update changes native text. |
| T02 | Inspect compiler output and dependency graph | Correct renderer helper imports, one Solid runtime, no server/web renderer. |
| T03 | Mixed static/dynamic children and empty text positions | Ordering and anchors remain correct. |
| T04 | Reorder retained items A,B,C to C,A,B | Existing native node identities move; no duplicates. |
| T05 | Remove a branch, finish a frame, recreate from supported lazy JSX | New live nodes appear and old handles remain invalid. |
| T06 | Hide a cached pre-created NativeNode and reuse it after a frame | Defined unsupported-operation failure until retention is implemented. |
| T07 | Mount fails after creating never-linked nodes | Nodes and callback roots are reclaimed; no orphan growth. |
| T08 | Remove properties from a spread and style object | Old attributes/styles/handlers are actually removed. |
| T09 | Replace an event callback repeatedly | Only the current callback executes; old roots are released. |
| T10 | Event callback mutates the same native tree | No bridge/runtime RefCell reentry failure. |
| T11 | preventDefault during native input | The chosen real native default is suppressed. |
| T12 | preventDefault after await | No false cancellation claim; defined expired-dispatch behavior. |
| T13 | Pointer/Enter/Space activation and disabled Button | Exactly-once activation and correct disabled/focus behavior. |
| T14 | Timer fires while application is idle | Callback executes without unrelated user input. |
| T15 | Rust service completes while application is idle | Completion wakes the host and updates UI. |
| T16 | Completion arrives after reload | Old generation is ignored without touching the new context. |
| T17 | Two completion paths race for one request | Exactly one settlement and complete resolver cleanup. |
| T18 | Promise job throws | Original exception is recovered and reported; later jobs remain usable. |
| T19 | Infinite synchronous loop | Interrupt policy returns control and faults/cleans the session. |
| T20 | Infinite or repeatedly throwing microtask chain | Time/count budget applies to successful and failed attempts. |
| T21 | rAF changes a layout-affecting property | Change participates in the scheduled native layout pass. |
| T22 | Occlusion and restoration with pending work | Defined housekeeping behavior, no busy loop, visual state catches up. |
| T23 | Controlled field with caret, selection, and IME | Native editing state remains coherent. |
| T24 | Dispose twice, then close the window | Idempotent cleanup; no persistent values survive runtime destruction. |
| T25 | Repeated full reload with timers/listeners/requests | Resource counts stabilize after warmup and reclamation. |
| T26 | Foreign-document or stale NativeNode | Validation error before unsafe/incorrect native mutation. |
| T27 | App type-check uses browser globals or unsupported tags | Compile-time rejection through native SDK declarations. |
| T28 | Stage a module that starts timers/services at top level | Preflight policy rejects or safely stages the operation. |
| T29 | Engine memory exhaustion under selected allocator | The configured policy is actually effective and reported. |
| T30 | New/recreated native window after initial startup | Correct binding and document-scoped identities are installed. |

Tests T05 and T06 must cross an actual native frame boundary. Immediate remove/reinsert tests cannot reveal end-of-frame reclamation mistakes.

For T25, compare live resource counters and engine memory after appropriate collection, and process memory after warmup. Allocator caching means RSS alone is not a precise leak oracle.

### 17.2 Test layers

- **Pure bridge tests:** Topology, handle validation, property normalization, request state transitions.
- **Engine integration tests:** Real rquickjs module evaluation, callback roots, jobs, interruption, Promise completion.
- **Native UI tests:** Event cancellation, frame timing, node recycling, editing, accessibility.
- **Build/type tests:** Compiler output, native declarations, package graph, source maps.
- **Compositor tests:** Wayland layer surfaces, scaling, monitor changes, popup behavior.

Use the smallest layer that can observe the failure. Event cancellation and native editing require real native integration; a mocked tree cannot prove them.

### 17.3 Completion criteria

A release is complete when:

1. The selected phase's acceptance gates pass.
2. The example app runs from the same package artifacts users will load.
3. Supported APIs and compatibility restrictions match the implementation.
4. Repeated lifecycle tests show stable resource ownership.
5. Runtime failures produce actionable diagnostics instead of silent stale UI.
6. Build/reproduction instructions identify exact dependency and platform choices.

## 18. Developer workflow and command contract

### 18.1 Proposed workspace commands

Implement these scripts or equivalent documented commands as part of the workspace:

| Command | Intended behavior |
| --- | --- |
| pnpm typecheck | Type-check SDK declarations and examples with their appropriate configurations |
| pnpm ui:build --app counter | Produce a validated counter package |
| pnpm ui:dev --app counter | Watch, rebuild atomically, and notify the native host |
| cargo test -p ui-bridge | Run bridge tests that do not need a live compositor |
| cargo test -p js-runtime | Run real-engine lifecycle/job tests |
| cargo run -p native-host -- --bundle apps/examples/counter/dist | Load the built package |
| env ZGUI_PLATFORM=wayland cargo run -p native-host -- --bundle apps/examples/counter/dist | Exercise direct Wayland explicitly |

These are proposed project command names, not commands already supplied by the upstream repositories.

Use environment-prefixed commands that work from common shells, including fish. Keep build-tool configuration in scripts instead of relying on shell-specific quoting.

### 18.2 Development cycle

1. Edit TSX, styles, SDK code, or Rust code.
2. Run the narrow relevant type/build/test step.
3. Load the actual compiled package in the native host.
4. Inspect source-mapped errors and bridge counters.
5. Run the affected integration gate.
6. Update the API contract if behavior intentionally changes.

Run broader tests when changing lifecycle, scheduling, identity, compiler selection, or platform integration. A style-only example change does not require rerunning unrelated compositor scenarios.

### 18.3 Build diagnostics

Print or expose:

- Host build/version and source revision.
- Application build ID and generation.
- Resolved JS runtime/compiler versions.
- Actual zgui platform backend.
- Active Fontique patch policy.
- Runtime feature selection and limit policy.
- Current session state and pending-work counters.

Do not expose implementation diagnostics in ordinary application flows unless they help a user resolve a failure. Keep them in logs, developer panels, or a diagnostic mode.

## 19. Rules for the implementing coding agent

Use this section as the execution contract when handing the specification to Codex or another coding agent.

### 19.1 Starting procedure

1. Read the repository's existing contributor instructions and inspect its current structure.
2. Determine whether this is a new workspace or an integration into an existing application.
3. Preserve unrelated work and existing application behavior.
4. Verify the dependency revisions and exact upstream signatures used by the first phase.
5. Implement Phase 0 and record its results before expanding the public API.
6. Keep each subsequent phase reviewable and update the acceptance checklist.

### 19.2 Implementation rules

- Treat zgui + rquickjs + TSX as fixed requirements.
- Treat Solid Universal as the default frontend specified here.
- Keep project APIs clearly separated from upstream APIs.
- Keep the bridge and runtime entry code small and explicit.
- Never erase JS lifetimes manually or store raw context/value pointers to bypass ownership.
- Never hold registry borrows across JavaScript execution.
- Use the current Ctx inside native callbacks.
- Preserve pending-job exceptions.
- Use real wakeups for external work.
- Use native frame callbacks for layout-affecting rAF work.
- Preserve synchronous cancellation semantics.
- Keep editor-owned children out of generic reconciliation.
- Make permanent resource disposal separate from temporary structural movement.
- Do not claim arbitrary Solid/DOM compatibility beyond the tested native profile.
- Do not copy internal Lynx protocols or introduce a second JS runtime without a measured requirement.
- Add upstream/fork patches only for concrete integration gaps; keep them narrow and documented.
- Prefer tests that expose cross-layer failures over tests that merely restate implementation details.

### 19.3 Reporting requirements

For each phase, report:

- The concrete behavior added.
- The affected public API and dependency choices.
- The meaningful checks performed and their outcomes.
- Any remaining gate that prevents the next phase from being considered complete.
- Measurements where the change is intended to improve performance.

A source audit is not a successful build, and a successful build is not a validated UI lifecycle. State the actual level of verification.

### 19.4 Handoff prompt

~~~text
Implement the attached zgui, rquickjs, and TSX Development Specification.

Use zgui as the native retained UI engine and rquickjs as the JavaScript
embedding layer. Use the proposed Solid Universal frontend and project-owned
@zgui/ui SDK for the first implementation. Keep the native bridge independent
of Solid.

Start by reading the repository instructions and implementing Phase 0.
Verify the pinned dependencies, public integration seams, and actual build
environment. Then implement subsequent phases in order, keeping the changes
reviewable.

Preserve the ownership and scheduling invariants. In particular, do not
re-enter a locked rquickjs context, do not pass JS values to worker threads,
do not lose pending-job exceptions, and do not implement layout-changing
requestAnimationFrame callbacks in the post-layout before_paint hook.

Use opaque native node objects and explicit generation validation. Respect
zgui's frame-end reclamation and native text-editor ownership. Implement
reactive cleanup and native resource cleanup together.

Treat every proposed SDK name and pseudocode contract in the specification
as functionality to implement, not an existing upstream API. Run the
meaningful acceptance gates for each completed phase. If source APIs differ,
verify the actual revision and make the smallest documented adaptation.

Deliver working code, updated development instructions, and a phase-by-phase
verification report. Do not claim later phases are complete until their
acceptance criteria are met.
~~~

## 20. Extension decisions

### 20.1 Moving JavaScript to a worker

A future worker design changes more than the Runtime's thread:

- Solid's synchronous topology queries must be answered from a logical tree on the JS side.
- Native mutations need an ordered transport.
- Events and layout results become messages.
- Synchronous default cancellation needs an explicit policy or a native fast path.
- Node/callback generations and message ordering remain mandatory.
- UI-thread animation and input behavior should remain responsive without JS round trips.

The rquickjs parallel feature does not supply this architecture or make zgui handles thread-safe.

### 20.2 Static templates and dynamic slots

Use profiling to determine whether node creation, property conversion, or bridge calls dominate. If they do, a compile-time static template with dynamic update positions may help.

Design a versioned project-owned representation. Keep component state and host rendering separate, and retain a fallback for dynamic structure. This is the most transferable lesson from the preceding Lynx discussion.

### 20.3 Explicit native retention

If cached JSX or keep-alive is required, add a deliberate retention API and ownership model. One candidate is a retain/release lease backed by a host-owned parking subtree while logical parent remains absent.

The lease must define:

- Who owns the retained resource.
- Whether state, focus, and native editor content persist.
- How styles and accessibility behave while parked.
- When the lease is released.
- How shutdown and resource limits reclaim it.

A ref alone must not silently retain an unbounded subtree.

### 20.4 Multiple applications or plugins

For independent reload and runtime-level quotas, evaluate one Runtime/Context per session. Several contexts in a shared runtime share heap/job/limit concerns and should not be described as independent resource isolation.

Measure aggregate startup time and memory before selecting the multi-session design. Cross-session communication uses owned data through host services.

Keep hostile plugins in a separately designed isolation model if they become a product requirement.

## 21. Source index

The links below identify the upstream contracts used by this specification. Version-specific behavior should be rechecked when a pinned dependency changes.

### zgui

- [Z1] Workspace manifest and toolchain baseline.
- [Z2] Facade dependencies, platform selection, and application wrapper.
- [Z3] Fontique vendor rationale and downstream patch instructions.
- [Z4] Public Dom mutation interface.
- [Z5] Native NodeId representation.
- [Z6] Concrete DocumentDom query interface.
- [Z7] Native unlink/recycling semantics.
- [Z8] Native element vocabulary and default styling.
- [Z9] Rust Button implementation.
- [Z10] ViewHost editing/scheduling contract.
- [Z11] Native input dispatch and synchronous listener sequencing.
- [Z12] Frame ordering and binding hook placement.
- [Z13] Platform wake routing.
- [Z14] Window handles, binding installation, and shutdown.
- [Z15] Wayland layer and popup implementation.
- [Z16] BuildCx handle access.
- [Z17] Actual event dispatcher.
- [Z18] Native editing model.
- [Z19] Native frame callback queue.
- [Z20] Native removal behavior for never-linked nodes.
- [Z21] Facade Cargo features and dependencies.
- [Z22] Native keyboard and pointer defaults.
- [Z23] Removed-subtree collection.
- [Z24] Concrete DOM frame-end ownership.
- [Z25] Platform context and waker.

### rquickjs and QuickJS-NG

- [Q1] rquickjs backend and host-environment description.
- [Q2] Context entry and runtime access.
- [Q3] Persistent values and lifetime requirements.
- [Q4] Module evaluation and exports.
- [Q5] Runtime jobs, limits, and interruption.
- [Q6] Async-runtime spawner requirements.
- [Q7] Runtime rejection/interrupt callback types.
- [Q8] QuickJS-NG embedding and bytecode guidance.
- [Q9] Ctx operations and context-level job behavior.
- [Q10] Published rquickjs 0.14.0 API documentation.
- [Q11] rquickjs manifest and feature selection.

### Solid and build tools

- [S1] Solid package export map and source version.
- [S2] Solid Universal setup and custom renderer exports.
- [S3] Solid component and JSX typing boundary.
- [S4] Universal runtime structure and disposal.
- [S5] Universal compiler property handling.
- [S6] List identity semantics.
- [S7] Reactive scheduler implementation.
- [S8] Cleanup lifecycle.
- [S9] Universal renderer TypeScript contract.
- [S10] Solid compiler package manifest.
- [S11] Universal component transformation and lazy children.
- [B1] Babel TypeScript transformation.
- [B2] esbuild platform, conditions, and bundling API.

[Z1]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/Cargo.toml
[Z2]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui/src/app/mod.rs
[Z3]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/vendor/README.md
[Z4]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-view/src/dom/mod.rs
[Z5]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-view/src/id/node.rs
[Z6]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-view-dom/src/dom/query.rs
[Z7]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-view-dom/src/dom/nodes.rs
[Z8]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-elements/src/tag/markers.rs
[Z9]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-ui/src/button/mod.rs
[Z10]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-view/src/host/mod.rs
[Z11]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-runtime/src/window/input.rs
[Z12]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-runtime/src/window/frame.rs
[Z13]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-runtime/src/wake.rs
[Z14]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-runtime/src/window/mod.rs
[Z15]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-platform-wayland/src/driver/surfaces.rs
[Z16]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-view/src/cx/mod.rs
[Z17]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-runtime/src/dispatch.rs
[Z18]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-runtime/src/editing.rs
[Z19]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-runtime/src/frame_callbacks.rs
[Z20]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-dom/src/mutate/edit/build.rs
[Z21]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui/Cargo.toml
[Z22]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-input/src/dispatch/defaults.rs
[Z23]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-dom/src/arena/recycle/subtree.rs
[Z24]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-view-dom/src/dom/mod.rs
[Z25]: https://github.com/zortax/zgui/blob/80baa3c7cb0fbbe1c058f10059e076e41c0a1b9a/crates/zgui-platform/src/cx.rs

[Q1]: https://github.com/DelSkayn/rquickjs/blob/73fcd6062ee615aeb2a4675cd32be67a56e93973/README.md
[Q2]: https://github.com/DelSkayn/rquickjs/blob/73fcd6062ee615aeb2a4675cd32be67a56e93973/core/src/context/base.rs
[Q3]: https://github.com/DelSkayn/rquickjs/blob/73fcd6062ee615aeb2a4675cd32be67a56e93973/core/src/persistent.rs
[Q4]: https://github.com/DelSkayn/rquickjs/blob/73fcd6062ee615aeb2a4675cd32be67a56e93973/core/src/value/module.rs
[Q5]: https://github.com/DelSkayn/rquickjs/blob/73fcd6062ee615aeb2a4675cd32be67a56e93973/core/src/runtime/base.rs
[Q6]: https://github.com/DelSkayn/rquickjs/blob/73fcd6062ee615aeb2a4675cd32be67a56e93973/core/src/runtime/opaque.rs
[Q7]: https://github.com/DelSkayn/rquickjs/blob/73fcd6062ee615aeb2a4675cd32be67a56e93973/core/src/runtime.rs
[Q8]: https://quickjs-ng.github.io/quickjs/developer-guide/intro/
[Q9]: https://github.com/DelSkayn/rquickjs/blob/73fcd6062ee615aeb2a4675cd32be67a56e93973/core/src/context/ctx.rs
[Q10]: https://docs.rs/rquickjs/0.14.0/rquickjs/
[Q11]: https://github.com/DelSkayn/rquickjs/blob/73fcd6062ee615aeb2a4675cd32be67a56e93973/Cargo.toml

[S1]: https://github.com/solidjs/solid/blob/99714d4e2875d2d7ef8023e82f4ef6aa63ff0752/packages/solid/package.json
[S2]: https://github.com/solidjs/solid/blob/99714d4e2875d2d7ef8023e82f4ef6aa63ff0752/packages/solid/universal/README.md
[S3]: https://github.com/solidjs/solid/blob/99714d4e2875d2d7ef8023e82f4ef6aa63ff0752/packages/solid/src/render/component.ts
[S4]: https://github.com/ryansolid/dom-expressions/blob/v0.40.14/packages/dom-expressions/src/universal.js
[S5]: https://github.com/ryansolid/dom-expressions/blob/v0.40.14/packages/babel-plugin-jsx-dom-expressions/src/universal/element.js
[S6]: https://docs.solidjs.com/reference/components/for
[S7]: https://github.com/solidjs/solid/blob/99714d4e2875d2d7ef8023e82f4ef6aa63ff0752/packages/solid/src/reactive/scheduler.ts
[S8]: https://docs.solidjs.com/reference/lifecycle/on-cleanup
[S9]: https://github.com/ryansolid/dom-expressions/blob/v0.40.14/packages/dom-expressions/src/universal.d.ts
[S10]: https://github.com/solidjs/solid/blob/99714d4e2875d2d7ef8023e82f4ef6aa63ff0752/packages/babel-preset-solid/package.json
[S11]: https://github.com/ryansolid/dom-expressions/blob/v0.40.14/packages/babel-plugin-jsx-dom-expressions/src/shared/component.js

[B1]: https://babeljs.io/docs/babel-preset-typescript
[B2]: https://esbuild.github.io/api/





