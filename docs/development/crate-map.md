# Crate & Package Map

Quick orientation for locating code across workspace packages, workspace Rust crates, vendored `zgui-*` crates, and unvendored upstream dependencies.

## End-to-End Dataflow

```text
packages/ui (TSX / Solid 2.0 / styles.d.ts / primitives / components)
  → packages/build (@solidjs/compiler Oxc + Lightning CSS Modules + Rolldown)
  → packages/cli (`zgui-js` CLI + @hewel/zgui-host-* prebuilt host resolution)
  → crates/ui-bridge (JS↔DOM ops, properties.rs, canvas.rs)
  → vendor/zgui-dom (arena, mutations, dirty bits)
  → vendor/zgui-style (cascade, animations, native CSS profile)
  → vendor/zgui-layout (boxtree, flex/grid/inline layout, TextMap, fragment diff)
  → vendor/zgui-paint (lower, emit, ContentCache/atlas, walk/replay)
  → vendor/zgui-runtime (App, Window frame loop, editing/caret, doc_select, images)
  → vendor/zgui-platform-{wayland,headless} + vendor/zgui-render-wgpu
  → crates/zgui-host (shell surfaces, session host, inspector, dev_banner, bytecode, preview)
```

## Workspace Packages, Apps & Crates

| Path                              | Role                                                                                                                                                                                 |
| --------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `packages/ui`                     | `@hewel/zgui` renderer, `styles.d.ts`, `shell.ts`, `@hewel/zgui/router`, `@hewel/zgui/primitives` (`*.ts`), `@hewel/zgui/components` (`*.ts`), `@hewel/zgui/oxlint`                  |
| `packages/services`               | `@hewel/zgui-services` capability-gated host services (sockets, commands, control, clipboard, SQLite/KV, local files, HTTP/SSE/WS, compute workers)                                  |
| `packages/build`                  | `@hewel/zgui-build` programmatic compiler (`@solidjs/compiler` Oxc + Lightning CSS Modules + Rolldown), `validate.ts`, `typecheck.ts`, `host.ts`, `package-app.ts`                   |
| `packages/cli`                    | `@hewel/zgui-cli` npm CLI (`zgui-js`: `init`, `dev`, `build`, `preview`, `package`, `doctor`, `inspect`, `smoke`)                                                                    |
| `packages/create-zgui-js`         | `create-zgui-js` scaffolding entrypoint (`bun create zgui-js`)                                                                                                                       |
| `packages/host-linux-{x64,arm64}` | Prebuilt `zgui-host` npm `optionalDependencies` platform packages                                                                                                                    |
| `apps/devtools/inspector`         | Self-hosted `zgui-js` visual Inspector window (`zgui-js inspect --ui`)                                                                                                               |
| `apps/examples/*`                 | Runnable example applications (`counter`, `css-modules`, `md-editor`, `gallery`, etc.)                                                                                               |
| `crates/ui-bridge`                | QuickJS↔DOM bridge (`properties.rs`, `canvas.rs`, `events.rs`)                                                                                                                       |
| `crates/js-runtime`               | QuickJS-NG engine, verified module/bytecode loader, bounded bridge turns, host globals                                                                                               |
| `crates/host-services`            | Async network (`network/`), SQLite (`sqlite.rs`), files/bookmarks (`files.rs`), compute workers (`compute_worker.rs`), sockets, commands, clipboard                                  |
| `crates/zgui-host`                | Wayland/headless runner, atomic/CSS-only reload (`session_host.rs`), dev banner (`dev_banner.rs`), visual inspector (`inspector.rs`), bytecode compiler, preview driver (`preview/`) |

## Vendored Upstream Crates (`vendor/`)

All 16 crates below are excluded from the root Cargo workspace (`Cargo.toml` `[workspace] exclude`) and wired via `[patch."https://github.com/zortax/zgui"]`. Run their tests with `bun run test:vendor` (or `bun run test:vendor <crate>`). Patch rationale and upstream provenance live in [`vendor/README.md`](../../vendor/README.md).

| Crate                                       | Owns                                                                          | Key entry files                                                                          |
| ------------------------------------------- | ----------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `vendor/zgui-dom`                           | DOM arena, mutations, dirty propagation, effective inert predicate            | `src/arena/store.rs`, `src/mutate/edit/build.rs`, `src/mutate/edit/interaction.rs`       |
| `vendor/zgui-input`                         | Hit-testing, pointer capture, gestures, IME state, focus order & focus traps  | `src/hit/mod.rs`, `src/focus/order.rs`, `src/focus/trap.rs`, `src/router/mod.rs`         |
| `vendor/zgui-a11y`                          | AccessKit tree projection, incremental diffing, relations, inbound AT intents | `src/world.rs`, `src/build/mod.rs`, `src/project/children.rs`                            |
| `vendor/zgui-style`                         | Stylo cascade, animations, native CSS profile                                 | `tests/stylex_profile.rs`                                                                |
| `vendor/zgui-layout`                        | Box tree, layout, scroll regions, inline `TextMap`, fragment diff             | `src/boxtree/build.rs`, `src/inline/resolved.rs`, `src/fragment/diff/mod.rs`             |
| `vendor/zgui-paint`                         | Style lowering, `clip-path`, vector/image emit, atlas cache, emit walk        | `src/lower/clip.rs`, `src/emit/vector/mod.rs`, `src/content/cache.rs`, `src/walk/mod.rs` |
| `vendor/zgui-render-wgpu`                   | GPU composition, frame upload, Vulkan DMA-BUF import                          | `src/frame_texture.rs`, `src/dmabuf.rs`, `src/renderer/draw.rs`                          |
| `vendor/zgui-scroll`                        | Scroll physics, chaining, overscroll                                          | `src/scroller/`                                                                          |
| `vendor/zgui-text-parley`                   | Parley/fontique text shaping & CJK-safe glyph rasterization                   | `src/raster/glyphs.rs`, `src/shape/glyphs.rs`                                            |
| `vendor/zgui-view` / `vendor/zgui-view-dom` | Reactive views & DOM observations                                             | `src/lib.rs`                                                                             |
| `vendor/zgui-runtime`                       | Multi-surface loop, frame pipeline, caret/editing, document selection, images | `src/app.rs`, `src/window/frame.rs`, `src/window/caret.rs`, `src/window/doc_select.rs`   |
| `vendor/zgui-platform`                      | Surface, monitor, input-region traits                                         | `src/surface/mod.rs`, `src/surface/region.rs`                                            |
| `vendor/zgui-platform-headless`             | Deterministic headless backend for tests & preview                            | `src/platform.rs`, `src/surface.rs`                                                      |
| `vendor/zgui-platform-wayland`              | Wayland/SCTK backend (layer-shell, xdg-popup, session-lock, clipboard, DnD)   | `src/surface/mod.rs`, `src/driver/protocol.rs`, `src/input/dnd.rs`                       |
| `vendor/zgui`                               | Top-level re-export facade                                                    | `src/lib.rs`                                                                             |

## Unvendored Upstream Crates

These live in the pinned `zortax/zgui` git checkout (`~/.cargo/git/checkouts/zgui-*/`), **not** in `vendor/`:

`zgui-wgpu`, `zgui-render`, `zgui-scene`, `zgui-atlas`, `zgui-image`, `zgui-geom`, `zgui-css`, `zgui-color`, `zgui-bits`, `zgui-edit`, `zgui-anim`, `zgui-elements`, `zgui-vocab`, `zgui-reactive`.

Check the table above before searching `vendor/` for a crate. Locate exact checkout paths offline with `cargo metadata --locked --offline --format-version 1 --filter-platform x86_64-unknown-linux-gnu`. Note that `KeyState`/`PointerAction` live in `zgui::vocab`, while `SurfaceEvent::Key` and `PointerEvent` live in `zgui::platform`. When a feature touches GPU rendering (`zgui-render-wgpu`), prefer CPU/atlas-cached solutions in `vendor/zgui-paint` unless vendoring the renderer is explicitly justified.
