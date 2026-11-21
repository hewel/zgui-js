# Design Wave 2 Network, Drag-and-Drop, Rich Document Selection, Canvas Extensions, and Scale Tooling Around Authority and Generation Isolation

**Status:** accepted

ADR-0004 defined Wave 1 to close the local-first desktop workbench loop (scrolling, UTF-16 selection, `@hewel/zgui/router`, window controls, clipboard, SQLite/KV, local files/bookmarks, and core workbench components) and sketched a three-stage Wave 2 roadmap. Deep review of `crates/js-runtime`, `crates/host-services`, `crates/native-host`, `crates/ui-bridge`, `packages/build`, and `vendor/zgui-*` shows that the critical architectural spine of Wave 2 is not rendering complexity—it is **how asynchronous external work is admitted, bounded in native memory, delivered across session generations, and revoked cleanly on reload or retirement**. Establishing that contract in Area 1 (network streams and DnD) directly shapes Area 3 (background JS workers and code-splitting).

We will implement Wave 2 in strict order, keeping Linux/Wayland first-class with platform-neutral JS APIs, sealed capabilities, fail-closed generation retirement, and zero-environment-leak deterministic previews.

## Area 1: Network Services and Drag-and-Drop

1. **Origin-allowlisted network services (`@hewel/zgui-services` + `host-services` + `native-host`, zero vendor patches):**
   - Run transport and decoding on a dedicated async network runtime off the QuickJS thread, never occupying `host-services`'s synchronous asset worker pool.
   - Seal exact normalized origins (`scheme`, `host`, and effective `port`) in `manifest.json`. Default to HTTPS/WSS with no ambient system proxy, cookie jar, ambient credentials, or implicit loopback/private-network access. Authorize before DNS, verify destination policy at connect time, and reauthorize every redirect and reconnect. A user gesture is never network authority.
   - Expose cancellable, owner-scoped core primitives: `httpRequest` (headers + bounded pull-driven body chunks), `openEventStream` (Rust-side incremental SSE parser), and `connectWebSocket` (full-duplex with explicit send backpressure and close), plus an explicit opt-in `createScopedFetch()` thin adapter (never mutating `globalThis`, no `Response.clone` buffering bombs, no cookies, no silent reconnect).
   - Bound **bytes** as well as event counts: decompressed body, incomplete SSE buffer, reassembled WebSocket message, outbound send queue, and aggregate native retention per session. Deliver via `Session`'s fair checkpoint with immediate generation + live-registration checks. During staged replacement (`SessionHost::mount_module_inner`), defer irreversible outbound sends until native acceptance so a rejected candidate mount never leaks side effects onto the wire.
   - In `native-host preview`, schedule chunks, failures, closes, and cancellations through the run-wide `Fixture` runtime; an unmatched request is a hard fixture failure before any live client is constructed.

2. **Drag-and-Drop (`vendor/zgui-platform*` + `vendor/zgui-runtime` + `crates/ui-bridge`):**
   - **Part A (Internal reorder + External file drops):**
     - Internal drag-and-drop uses one drag identity, native hit-tested target chains via `Window::chain_at` (respecting paint clips, transforms, and scale—never routing drops to keyboard focus or confusing source pointer capture with target hit-testing), an opaque session-local payload, synchronous target acceptance negotiation, and automatic cancellation on node detach, surface loss, or generation retirement.
     - External file drops fix the Wayland cross-offer answer-slot race (`vendor/zgui-platform-wayland/src/driver/mod.rs` and `src/input/dnd.rs`) by binding every pipe read result to `(offer_id, generation)`. Feed external `Entered`/`Moved`/`Left`/`Dropped` offers into the same native hit-testing pipeline. Hover exposes MIME/file types only—never raw paths or grants; an accepted drop mints read-only Wave 1 `FileHandle` capabilities.
   - **Part B (Separate follow-up — Outbound Copy DnD):** Implement outbound drag source in `vendor/zgui-platform-wayland/src/driver/data.rs` for `Copy` semantics only, gated on a live press serial on the originating seat/surface. Do not advertise `Move` without a destination-commit contract.

## Area 2: Rich Document Selection, CanvasScene, and CommandPalette

1. **Opt-in read-only `<text>` cross-block selection (`vendor/zgui-runtime` + `crates/ui-bridge` + `@hewel/zgui`):**
   - Do **not** fold `<text>` into `vendor/zgui-runtime/src/editing.rs` (which owns per-field undo, IME composition, and secure projection) and do not compute selection boxes in JS.
   - Introduce an explicit opt-in document selection container scoped to a single mount region, storing directional endpoints `(source_text_node, byte_offset, affinity)` on the window. Map Parley hits back through `vendor/zgui-layout/src/inline/resolved.rs`'s existing `TextMap` and source boxes, and paint highlights through `vendor/zgui-runtime/src/window/caret.rs`'s damage/transform/scroll pipeline.
   - Support continuous cross-block drag selection (paragraphs, headings, lists, tables) copied in document order with explicit block separators and no `\n` inserted for soft wraps. Respect CSS `user-select`, exclude editable/secure controls and other mount regions, clear selection safely if an endpoint detaches, and translate indices to UTF-16 strictly at the JS seam.

2. **`CanvasScene` extensions (explicitly split into Phase 2a and Phase 2b):**
   - **Phase 2a (zero new vendored crates — ship first):** Decode `Brush::Linear` / `Brush::Radial` (already present in pinned `zgui-canvas`) and dashed strokes via `kurbo::Stroke::with_dashes` inside `crates/ui-bridge/src/canvas.rs` with bounded stop counts, dash entries, and effective expansion. Verify both fast-path and general vector lowering (`vendor/zgui-paint/src/emit/vector/recognise.rs` declines dashed strokes). Preserve `update(null, transform)` as a view-only update.
   - **Phase 2b (separate optional track):** Add ordered retained text (Parley + session fonts, zero ambient preview font lookup) and `ImageHandle` blitting (session image policy + existing atlas residency) at the `zgui-canvas` / `vendor/zgui-paint` `Drawing::canvas` seam, keying invalidation on font/image identity as well as scene revision. Deferred to its own track because it requires vendoring `zgui-canvas` or coordinating an upstream `zgui` change.

3. **`CommandPalette` (`@hewel/zgui/components`, independent delivery):**
   - Pure `packages/ui` composition over `Dialog`, a focused `TextField`, `VirtualList`, and existing `components/generated/CommandStyle.ts`. Keep focus and IME composition inside the input field, track active selection by stable `commandId` (never recycled row nodes), scope command registries to the owning window, and restore focus on dismiss.

## Area 3: Engineering and Scale Tooling

1. **State-preserving dev reload (replacing AST HMR):**
   - Reject Solid-Refresh-style live AST signal patching; it cannot preserve native handle, lease, or effect ownership invariants across arbitrary source edits.
   - Layer a bounded, versioned, opt-in plain-data snapshot onto `SessionHost::mount_module_inner`: serialize declared dev state from a settled generation and hydrate fresh signals during candidate mount, never transferring closures, promises, native nodes, grants, or service subscriptions. Provide a dev-only `createDevState(key, initial)` primitive and wire `@hewel/zgui/router` to preserve active location/history automatically in dev mode. Never snapshot faulted or session-locked generations; cold-mount cleanly on schema or capability mismatch.

2. **Desktop runtime Inspector / Profiler (`crates/native-host`):**
   - Extract the native-only bounded tree/style snapshot reader from `crates/native-host/src/preview/pipeline/inspect.rs` and pair it with `RuntimeTelemetry`, `ResourceCounts`, and `vendor/zgui-runtime/src/probe.rs::CompletedFrame`.
   - Record JS entry durations, checkpoint queue depths, and Stylo/Taffy/Parley/Vello frame phases into a bounded ring buffer. Strictly dev-only, host-owned, and read-only: never evaluate app JS getters, redact secure inputs, forbid lock-surface capture, and keep wall-time diagnostics decoupled from deterministic preview assertions.

3. **Bundle code-splitting (`packages/build` + `crates/native-host` + `crates/js-runtime`):**
   - Implement as a sealed package-format and QuickJS `ModuleLoader` architecture: Rolldown emits a closed chunk graph with literal edges, per-chunk SHA-256 digests, and per-chunk source maps sealed in `manifest.json`.
   - Verify the complete chunk graph before native acceptance and load lazy chunks solely from verified package bytes under existing turn/memory budgets, preserving one Solid runtime, synchronous mount, and no top-level await.
   - **Keep all CSS (`base.css`, `components.css`, `app.css`) statically merged at build time** and lazy-load only JS chunks so Stylo stylesheet cascade order remains 100% deterministic with zero FOUC.

4. **Background JS Worker (`crates/host-services` + `crates/js-runtime` + `@hewel/zgui-services`):**
   - Reuse the sealed chunk loader and Area 1's generation-checked delivery contract. Each declared compute-worker entry runs on its own OS thread with its own dedicated QuickJS `Runtime` (never a second `Context` on the UI engine).
   - Zero UI bridge, zero shared Solid graph, zero inherited host capabilities, and no transferable native handles—only bounded value-copy messages across capped inbound/outbound mailboxes, bounded worker heap, aggregate session memory cap, and per-turn execution deadlines.
   - On session retirement, immediately close admission, trigger QuickJS interrupt, and discard late replies without blocking the UI thread on `join`.
   - In `native-host preview` virtual-clock mode, drive worker execution deterministically via lockstep quantum stepping on the run-wide `Fixture` scheduler (with seeded clock/randomness and real watchdog deadlines) to eliminate inter-thread scheduling flakiness.

## Wave 2 End-to-End Architectural Gate

Reject a staged reload while the predecessor holds an active network stream, then accept a successor that lazy-loads a code-split chunk and starts a background worker. The rejected candidate must produce zero external network sends, the predecessor must remain interactive until acceptance, and neither its retired stream nor the successor's later-retired worker may mutate a newer mount region.

## Considered Options

- **Ship a global WHATWG `fetch` / Cookie Jar / `Response.clone` polyfill:** Rejected because ambient global networking undermines capability-first design, cookies invite ambient session state, and `Response.clone` encourages unbounded buffering outside QuickJS's heap cap; an explicit `createScopedFetch()` adapter over `httpRequest` preserves SDK ergonomics without those hazards.
- **Add `<text>` directly to `vendor/zgui-runtime/src/editing.rs`:** Rejected because `editing.rs` couples selection to undo history, IME preedit, and password masking; a dedicated read-only document selection container over `TextMap` keeps both models clean.
- **Implement Vite/Solid-Refresh AST HMR:** Rejected because mutating live Solid owners in place conflicts with `zgui-js`'s strict per-generation native handle, timer, and service ownership model.
- **Lazy-load per-chunk CSS alongside JS code-splitting:** Rejected because dynamic stylesheet insertion order makes Stylo cascade order depend on runtime navigation order and risks FOUC, while CSS payloads are already small compared to JS parse/eval costs.
- **Run real background OS worker threads unconstrained in `native-host preview`:** Rejected because OS thread scheduling relative to virtual frames introduces nondeterministic capture flakiness; lockstep quantum stepping preserves determinism while keeping real watchdog limits.

## Consequences

- `packages/build` (`compiler.ts`, `validate.ts`) and `crates/native-host/src/package.rs` will extend `manifest.json` with `network` origins, sealed `chunks` graphs, and declared `workers` entries while retaining backwards compatibility with single-chunk packages.
- `vendor/zgui-platform-wayland` and `vendor/zgui-runtime` gain documented local patches for Wayland offer-scoped DnD pipe delivery, `Window::chain_at` drop hit-testing, and read-only `TextMap` document selection (tracked in `vendor/README.md` and `bun run test:vendor`).
- `crates/native-host` adds teardown counters for network streams, WebSockets, active drags, and background workers so `teardown.clean === true` remains enforceable across every Wave 2 feature.
