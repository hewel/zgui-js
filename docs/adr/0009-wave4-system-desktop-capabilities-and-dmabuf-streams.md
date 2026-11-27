# Wave 4 System Desktop Capabilities: Activation, Jiff/ICU4X, Dual-Track `systemFiles`, and Zero-Copy DMABUF Frame Streams

**Status:** accepted

Waves 1–3 (`ADR-0004`, `ADR-0005`, `ADR-0008`) completed the local-first workbench loop, generation-isolated networking/workers/code-splitting, and pure-npm developer tooling, proving `zgui-js` on showcase workbenches (`md-editor`, `sysmon`, `workspace`) and Wayland shell surfaces (`../z-shell`). Evaluating a full suite of core Linux desktop applications (`../gnome-apps`: `gnome-logs`, `gnome-calendar`, `nautilus`, and `sushi`) as modern native rewrites in a sibling monorepo (`../z-apps`) exposes six structural platform boundaries where stock `zgui-host` cannot serve system-grade desktop workflows: single-instance CLI activation, missing QuickJS `Intl`/timezone/collation fundamentals, runtime user grants vs. static capabilities, confined workspace `DirHandle` rules vs. system file management, missing data/date UI primitives, and 60fps video texture presentation without bundling media codecs into `zgui-host`.

We will deliver **Wave 4** in three ordered stages (`4a` → `4b` → `4c`) locked to the `../z-apps` rollout (`z-logs` → `z-calendar` → `z-files` + `@z-apps/previewer`), keeping `zgui-js` codec-free and domain-neutral while delegating stateful sync (`z-calendard`) and heavy media/PDF/archive codecs (`z-mediad`) to companion Unix-socket daemons in `../z-apps`, and preserving 100% deterministic `Fixture runtime` simulation across every new host boundary.

## Stage 1 (`Wave 4a` — Unblocking `z-logs`)

1. **Single-instance `Activation` (`crates/zgui-host` + `@hewel/zgui-services`):**
   - Add opt-in single-instance coordination over `$XDG_RUNTIME_DIR/zgui-js/<appId>.sock` (`0600`).
   - Cold starts expose the initial `Activation` (`args`, `cwd`, optional `activationToken`); subsequent launches forward `Activation` to the running session and hand off compositor focus instead of failing with `AlreadyExists` or rejecting application positional arguments.
   - Simulate cold and live `Activation` events deterministically in `zgui-host preview`.
2. **Host Time/Locale Bridge & Standard Runtime Globals (`crates/js-runtime` + `crates/host-services` + `@hewel/zgui-services`):**
   - Integrate `jiff` + `jiff-icu` + `icu4x` (`datetime` + `collator`) on the Rust host (~1.5 MiB baked data, reading system `/usr/share/zoneinfo` / `TZDIR` at runtime with deterministic timezone/locale pinning in `Fixture runtime`) to expose IANA timezone/DST civil arithmetic, localized date/time formatting, and multi-language natural string collation (`img2.png < img10.png`).
   - Add `URL`, `URLSearchParams`, `TextEncoder`, and `TextDecoder` to the validated QuickJS runtime global set (`packages/build/src/validate.ts` and `crates/js-runtime`).
3. **Variable-Height Virtualization & Tabular Components (`@hewel/zgui` + `@hewel/zgui/components`):**
   - Extend `VirtualList` to support measured variable-height rows alongside fixed `itemHeight` fast paths, and promote `Table` / `DataTable` (column resize/sort) and `AlertDialog` from synced upstream stylesheets.
   - `z-logs` runs directly on stock `zgui-host` via cursor-windowed `spawnCommand("journalctl", ["--output=json", "--after-cursor=..."])` queries + streaming `journalctl -f` tail + SQLite starred cursors, keeping `zgui-host` free of `libsystemd` FFI.

## Stage 2 (`Wave 4b` — Unblocking `z-calendar` + `z-calendard`)

1. **`Capability` vs. `Grant` Domain Split (Deferring Dynamic Network/Secret Grants):**
   - Formalize `Capability` as the compile-time manifest ceiling and `Grant` as host-persisted per-`appId` runtime user authorization.
   - **Explicitly defer dynamic network-origin and Secret `Grant`s out of Wave 4**: because CalDAV/Webcal synchronization, custom/enterprise TLS CAs, `org.freedesktop.secrets` credentials, background RRULE expansion, and closed-window `VALARM` notifications live in `../z-apps`'s long-lived `z-calendard` user service (shared over `connectUnixSocket` by both `z-calendar` UI and `../z-shell`), Wave 4 `Grant`s stay focused on filesystem locations while static `capabilities.network.origins` continues to serve fixed public endpoints (e.g., weather APIs).
2. **Date & Calendar Input Primitives (`@hewel/zgui/components`):**
   - Promote `Calendar` and `DatePicker` into `@hewel/zgui/components` (backed by the Wave 4a `jiff`/`icu4x` bridge); keep application-specific Month/Week/Agenda timeline views inside `../z-apps`.

## Stage 3 (`Wave 4c` — Unblocking `z-files` + Dual-Mode `@z-apps/previewer` + `z-mediad`)

1. **Dual-Track `systemFiles` Service & `openExternal` (`crates/host-services` + `@hewel/zgui-services`):**
   - Keep Wave 1's `DirHandle` (`files`) strictly confined (`E_PATH_ESCAPE` on external symlinks, 16 MiB atomic whole-file I/O) for ordinary document/workbench apps.
   - Introduce a distinct capability-gated `systemFiles` family backed by filesystem `Grant`s: symlink-aware `lstat` and paginated `readDir` (exposing `modifiedMs`, POSIX mode, symlink target, and MIME type without the 10,000-entry cliff), cancellable background `copy` / `move` / `trash` / `restore` / `mkdir` / `symlink` / `chmod` jobs with byte-progress and conflict-resolution callbacks, `/proc/mounts` + `UDisks2` volume discovery (relying on system FUSE mounts for network shares rather than reimplementing GVFS), and gesture/token-aware `openExternal` (`xdg_activation_v1`).
2. **Split Fast-Path Thumbnail Reads vs. `z-mediad` Generation & Archives:**
   - `zgui-host` serves existing Freedesktop `~/.cache/thumbnails/*.png` cache hits directly into native `ImageHandle`s with zero helper IPC hop for high-FPS grid scrolling.
   - Cache-miss thumbnail generation (images, video posters, PDF covers), on-demand PDF/font page rendering (`connectUnixSocket` `0x01` `ImageHandle`), and `.zip` / `.tar.*` archive compress/extract jobs live in `../z-apps`'s `z-mediad` daemon.
3. **Full File-Manager Drag-and-Drop & Multi-Selection (`vendor/zgui-platform-wayland` + `@hewel/zgui`):**
   - Extend Wayland `wl_data_device` / `wl_data_source` to admit directory drops, negotiated `Move` + `Copy` effects, and path-backed `text/uri-list` outbound drags without the 4 MiB in-memory payload cap.
   - Promote `Breadcrumb` into `@hewel/zgui/components` and extract a headless multi-select / rubberband selection primitive into `@hewel/zgui/primitives`.
4. **Zero-Copy Linux `DMABUF` `Frame stream` (`vendor/zgui-render-wgpu` + `vendor/zgui-paint` + `crates/zgui-host`):**
   - Deliberately vendor `zgui-render-wgpu` (and minimal `zgui-render` / `zgui-scene` external-texture seams) to import `DMA-BUF` file descriptors (`SCM_RIGHTS` over Unix socket with DRM fourcc/modifier/plane metadata and explicit/implicit sync) exported by `z-mediad` hardware decoders directly into Vulkan/`wgpu` textures with zero CPU readback, while `z-mediad` outputs audio directly to PipeWire.
   - Maintain a deterministic software frame fallback (`memfd` / synthetic RGBA `Fixture runtime` producer) so `zgui-host preview` runs identically on `llvmpipe` without GPU video hardware.
   - Package `@z-apps/previewer` (`sushi` rewrite) as a dual-mode TypeScript UI package: loaded as a lazy Rolldown chunk inside `z-files` (giving instant `<Space>` preview and live arrow-key selection tracking without Wayland `xdg_foreign` cross-process window parenting) and buildable as a standalone `z-preview` CLI utility.

## Considered Options

- **Target GNOME D-Bus drop-in protocols (`FileManager1`, `NautilusPreviewer2`, `SearchProvider2`, EDS, GVFS):** Rejected because carrying legacy GNOME IPC/plugin contracts and `xdg_foreign` cross-process preview positioning adds heavy desktop coupling that our `../z-shell` Wayland ecosystem replaces cleanly with shared Unix-socket services (`z-calendard`) and same-session lazy chunks (`@z-apps/previewer`).
- **Put all codecs, PDF rendering, Archive I/O, and CalDAV sync directly inside `zgui-host`:** Rejected because linking `ffmpeg`/`GStreamer`, PDF engines, and archive codecs into `zgui-host` would bloat `@hewel/zgui-host-linux-*` npm packages and expand the host attack surface for non-media apps.
- **Loosen existing `DirHandle` in place instead of adding `systemFiles`:** Rejected because editor/notes apps rely on `DirHandle`'s canonical root confinement and atomic rewrite invariants; mixing unconfined symlinks, permanent vs. Trash semantics, and multi-gigabyte streaming jobs into `DirHandle` weakens safety for ordinary apps.
- **Enable full ICU `Intl` inside QuickJS or rely on pure-JS timezone/iCal polyfills:** Rejected in favor of `jiff` + `jiff-icu` + `icu4x` on the Rust host, which reads OS zoneinfo natively, avoids burning QuickJS's 64 MiB heap and 100 ms turn budget on large tzdata tables, solves natural filename collation (`icu_collator`) in the same dependency, and stays deterministic in `preview`.
- **Keep Dynamic Network-Origin & Secret `Grant`s in Wave 4:** Deferred once Calendar adopted `z-calendard`, eliminating speculative host complexity that none of the four target apps immediately exercise.
- **Use CPU `memfd` shared memory first to avoid vendoring `zgui-render-wgpu`:** Rejected for Stage 3's primary desktop path because 4K60 video playback over CPU `write_texture` wastes memory bandwidth and battery compared to direct VAAPI/Vulkan `DMA-BUF` zero-copy import; vendoring `zgui-render-wgpu` is explicitly justified here while retaining software buffers for headless `llvmpipe` previews.

## Consequences

- `GLOSSARY.md` sharpens `Capability` and introduces `Grant`, `Activation`, and `Frame stream`.
- `Cargo.toml` and `vendor/` will add `jiff`, `jiff-icu`, `icu4x` components (`Wave 4a`) and vendored `zgui-render-wgpu` (+ minimal render/scene texture-import seams in `Wave 4c`, tracked in `vendor/README.md`, `docs/development/crate-map.md`, and `bun run test:vendor`).
- `@hewel/zgui`, `@hewel/zgui/primitives`, `@hewel/zgui/components`, and `@hewel/zgui-services` expand across Waves `4a`–`4c` with corresponding `Fixture runtime` adapters, `docs/development/component-map.md` entries, and `llms.txt` updates.
- Application code, app-specific domain glossaries/ADRs, `z-calendard`, and `z-mediad` will reside in the sibling `../z-apps` repository rather than `zgui-js/apps/*`.
