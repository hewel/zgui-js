# Crate & Package Map

Quick orientation for locating code across workspace packages, workspace Rust crates, vendored `zgui-*` crates, and unvendored upstream dependencies.

## End-to-End Dataflow

```text
packages/ui (TSX / Solid / styles.d.ts)
  → crates/ui-bridge (JS↔DOM ops, properties.rs, canvas.rs)
  → vendor/zgui-dom (arena, mutations, dirty bits)
  → vendor/zgui-style (cascade, animations, StyleX profile)
  → vendor/zgui-layout (boxtree, flex/grid/inline layout, fragment diff)
  → vendor/zgui-paint (lower, emit, ContentCache/atlas, walk/replay)
  → vendor/zgui-runtime (App, Window frame loop, editing/caret, images)
  → vendor/zgui-platform-{wayland,headless} + unvendored zgui-render-wgpu
  → crates/native-host (shell surfaces, session host, headless preview)
```

## Workspace Packages & Crates

| Path | Role |
| --- | --- |
| `packages/ui` | `@hewel/zgui` renderer, `styles.d.ts`, `shell.ts`, `@hewel/zgui/components` |
| `packages/services` | `@hewel/zgui-services` host service bindings |
| `packages/build` | `@hewel/zgui-build` bundler, StyleX pipeline, `validate.ts`, preview CLI |
| `crates/ui-bridge` | QuickJS↔DOM bridge (`properties.rs` CSS aliases, `canvas.rs` vector commands) |
| `crates/js-runtime` | QuickJS engine, bounded bridge turns, host globals |
| `crates/host-services` | Capability-gated Unix sockets, commands, control messages, image handles |
| `crates/native-host` | Wayland/headless runner, shell surface lifecycle (`session_host/surfaces.rs`, `shell/session.rs`), preview driver (`preview/`) |

## Vendored Upstream Crates (`vendor/`)

All 13 crates below are excluded from the root Cargo workspace (`Cargo.toml` `[workspace] exclude`) and wired via `[patch."https://github.com/zortax/zgui"]`. Run their tests with `bun run test:vendor` (or `bun run test:vendor <crate>`). Patch rationale and upstream provenance live in [`vendor/README.md`](../../vendor/README.md).

| Crate | Owns | Key entry files |
| --- | --- | --- |
| `vendor/zgui-dom` | DOM arena, mutations, dirty propagation | `src/mutate/edit/build.rs`, `src/mutate/edit/style/` |
| `vendor/zgui-style` | Stylo cascade, animations, StyleX profile | `tests/stylex_profile.rs` |
| `vendor/zgui-layout` | Box tree, layout, scroll regions, fragment diff | `src/boxtree/build.rs`, `src/measure.rs`, `src/fragment/diff/mod.rs` |
| `vendor/zgui-paint` | Style lowering, `clip-path`, vector/image emit, atlas cache, emit walk | `src/lower/clip.rs`, `src/emit/vector/mod.rs`, `src/content/cache.rs`, `src/walk/mod.rs` |
| `vendor/zgui-scroll` | Scroll physics, chaining, overscroll | `src/scroller/` |
| `vendor/zgui-text-parley` | Parley/fontique text shaping & glyph rasterization | `src/lib.rs` |
| `vendor/zgui-view` / `vendor/zgui-view-dom` | Reactive views & DOM observations | `src/lib.rs` |
| `vendor/zgui-runtime` | Multi-surface loop, frame pipeline, caret/editing, image decoding | `src/app.rs`, `src/window/frame.rs`, `src/window/caret.rs`, `src/images/input.rs` |
| `vendor/zgui-platform` | Surface, monitor, input-region traits | `src/surface/mod.rs`, `src/surface/region.rs` |
| `vendor/zgui-platform-headless` | Deterministic headless backend for tests & preview | `src/platform.rs`, `src/surface.rs` |
| `vendor/zgui-platform-wayland` | Wayland/SCTK backend (layer-shell, xdg-popup, session-lock) | `src/surface/mod.rs`, `src/driver/protocol.rs` |
| `vendor/zgui` | Top-level re-export facade | `src/lib.rs` |

## Unvendored Upstream Crates

These live in the pinned `zortax/zgui` git checkout (`~/.cargo/git/checkouts/zgui-*/`), **not** in `vendor/`:

`zgui-render-wgpu`, `zgui-wgpu`, `zgui-render`, `zgui-scene`, `zgui-atlas`, `zgui-geom`, `zgui-css`, `zgui-color`, `zgui-bits`, `zgui-a11y`, `zgui-edit`, `zgui-input`, `zgui-anim`, `zgui-elements`, `zgui-vocab`, `zgui-reactive`.

Check the table above before searching `vendor/` for a crate. When a feature touches GPU rendering (`zgui-render-wgpu`), prefer CPU/atlas-cached solutions in `vendor/zgui-paint` unless vendoring the renderer is explicitly justified.
