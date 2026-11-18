# Keep zgui-js Domain-Agnostic While Adding Multi-Surface Wayland Shell and Generic IPC Primitives

**Status:** accepted

Building a Wayland desktop shell (multi-output bars, edge-revealed docks, full-screen capture panels, notifications, lock screens, and hardware status panels) requires multi-surface Wayland protocols, richer vector/CSS/motion primitives, dynamic host images, and external daemon communication. However, desktop business integrations (NetworkManager, BlueZ, PipeWire, UPower, MPRIS, desktop notifications, SNI system tray, compositor WM clients, PAM authentication, Matugen palette generation, `ext-idle-notify-v1`, and `ext-data-control-v1` encrypted clipboard storage) can live outside the UI framework in independent system processes. We will keep `zgui-js` domain-agnostic: its Wayland backend will own only pixel/input surface protocols (multi-output inventory, dynamic `zwlr_layer_shell_v1` **Shell surfaces**, rectangular `wl_region` **Input regions**, and `ext-session-lock-v1` **Session lock** surfaces), while `@hewel/zgui-services` will provide only a minimal generic transport base (Unix domain socket streams, allowlisted subprocess `spawn` streams, and an inbound control channel), all modeled symmetrically in **Fixture runtime** for deterministic previews.

Shell packages will run inside a single QuickJS **Session**—optionally launched in a pure-shell mode with zero initial default window—that observes output hotplug and mounts multiple **Shell surfaces** as auxiliary **Mount regions** sharing one reactive graph, theme state, and IPC connection pool. All privileged primitives (`shellSurfaces`, `sessionLock`, Unix sockets, commands, control socket, and read-only `file://` media prefixes) remain zero-authority by default and require explicit **Capability** declarations sealed in `manifest.json`, paired with host-owned `zgui-blob://<id>` **Image handles** so large icons, wallpapers, and thumbnails stay outside QuickJS's 64 MiB heap.

## Considered Options

- **Embedding desktop D-Bus/Wayland-daemon protocols inside `zgui-js`:** Rejected because coupling `native-host` to NetworkManager, BlueZ, PipeWire, PAM, Matugen, or compositor-specific IPC turns a general UI runtime into a desktop-environment monolith and bloats preview fixtures.
- **One isolated QuickJS **Session** per surface with a host message bus:** Rejected because splitting the bar, dock, and panels into separate JS engines breaks shared Solid reactive state, forces JSON serialization across theme/selection updates, and duplicates IPC sockets and image caches.
- **Unrestricted filesystem paths or CLI flags instead of manifest **Capabilities**:** Rejected because ambient host authority undermines sealed **Package publication**, reproducible **Artifact inventory** checks, and strict **Fixture runtime** enforcement during preview.

## Consequences

- `GLOSSARY.md` generalizes **Session** and **Bridge turn** from one **Mount region** to zero or more **Mount regions** owned per generation, so single-window `mount(root)` applications and pure-shell `mount()` packages share one retirement and fault model.
- Any **Session** fault, timeout, or teardown under an active **Session lock** fails closed: `SessionHost` must never call `unlock_and_destroy` except on an explicit successful unlock command, keeping the compositor session locked even if JavaScript faults.
- Keeping large images and binary IPC payloads as Rust-owned **Image handles** (`zgui-blob://<id>`) prevents QuickJS heap exhaustion while preserving `replace_image_policy` generation isolation on reload.
