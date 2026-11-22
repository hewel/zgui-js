# Native dependency patch

For a crate-by-crate ownership table (vendored vs unvendored upstream crates), see [`docs/development/crate-map.md`](../docs/development/crate-map.md).

`zgui-runtime/` is copied from `zortax/zgui` revision `ee29c7dc66206eb04ba7bcf514058a278c9070ba`, with its Apache-2.0 LICENSE and NOTICE. The manifest is standalone; every sibling zgui dependency retains that revision. The root Cargo patch selects this copy for the entire dependency graph. Upstream source and tests are retained.

## Upstream `ee29c7dc` rebase notes (retired vs retained contracts)

Upgrading the pin from `80baa3c7` to `ee29c7dc66206eb04ba7bcf514058a278c9070ba` (`wgpu 30.0.2`, `vello 0.11.0`, partial present, subpixel AA `#70`, resize pacing `#57`, Wayland shell/popup/session-lock `#66`, inbound DnD `#40/#41/#44`, caret reveal & IME `#38/#39`, window state `#42/#43`, frame reports `#48`, RTL `ViewHost::is_rtl` `#62`) retired overlapping downstream shims while retaining every `zgui-js` contract:

- **Retired**: `AppHandler::monitors_changed` (replaced by upstream `WakeReason::MonitorsChanged`), bespoke `LayerPlacement` struct literals (adopted upstream builders, `ExclusiveZone::{Ignore, Avoid, Reserve}`, and `MonitorTarget`), bespoke `PopupPlacement::grab: bool` (adopted upstream `Option<InputSerial>` populated from `parent.input_serial()`), and ad-hoc startup without a primary window (adopted `App::without_primary_window().with_exit_policy(ExitPolicy::Explicit)`).
- **Disambiguated coexistence**: Immediate-commit shell input regions use `Surface::set_shell_input_region(&InputRegion)` alongside upstream frame-staged `Surface::set_input_region(Option<&[Rect<CssPx, Css>]>)` (sharing one wire encoder). Fail-closed session unlock uses `PlatformCx::try_unlock_session() -> Result<(), PlatformError>` alongside upstream `PlatformCx::unlock_session() -> ()`, both operating on the **single** lock owner (`driver/session_lock.rs` + `surface/role/lock.rs` on Wayland, `Desktop.session_lock` on headless; upstream `zgui-platform-wayland/src/lock.rs` is intentionally excluded).
- **Retained**: All 13 vendored crates remain patched for our contracts below, including `fontique 0.11.1` from `ee29c7dc`, `#76` per-contour synthetic bold winding (now preserving `#70` `STRIPE_OFFSETS` subpixel AA) and `>= 600` weight threshold, `#48/#52` `ImagePolicy` / SVG `fontdb`, `#56/#68` `doc_select`, `#67` outbound Copy DnD, `LayerPatch` + atomic stretch-axis reconfiguration, `PopupPlacement` `offset`/`reactive`/`geometry_insets` (`xdg_surface.set_window_geometry` paired with buffer commits, nested anchor translation, and visual auto-height settling), probe-gated `CompletedFrame::phases` alongside upstream `FrameReport`, and `ViewHost::is_rtl` reading inherited CSS `direction`.

## Disabled activation

Upstream `Router::key` keeps the focused node after it becomes disabled. Its default activation is computed before event listeners, and `Window::carry_out_default` previously synthesized Click without rechecking current state. Real Wayland input reproduced Enter/Space activation of a disabled, already-focused control.

The activation patch checks the live node's disabled state immediately before `FrameworkDefault::Activate` executes. This also prevents activation when a key listener disables the control during that same event. It does not synthesize JS events, alter keyboard repeat policy, or replace native controls.

Regression gate: `ZGUI_PLATFORM=wayland cargo test -p native-host native_tests::native_button_actions -- --ignored --exact --nocapture`. The gate includes cached focus, same-event disabling, pointer/Enter/Space activation and native cancellation.

## Occluded frame callbacks

Upstream `Window::run_frame_callbacks` drained callbacks even in a frame run for hidden-window housekeeping, despite the public `ViewHost` contract promising retention until visibility resumes. A controlled occlusion gate on the real Wayland runtime reproduced rAF and its Promise layout work running while `Window::is_occluded()` remained true.

The frame-callback patch returns before draining when the window is occluded. Native timers, owner checkpoints and task completions continue; unocclusion releases the preserved callbacks before layout.

Historical verification used the now-retired `async_tests::native_async_delivery`
gate to move only its PID/app-ID-verified window to an inactive niri workspace,
wait for native occlusion, then restore it. Timers and Rust completion finished
while rAF remained pending; the first visible frame included the Promise layout
change. That compositor-specific gate is no longer shipped. The patch remains a
generic upstream runtime correctness fix, not a niri support requirement.

## Cancelled timer retention

Upstream cancellation kept heap entries and closures until their original deadlines. Repeatedly creating and cancelling long timers therefore bypassed the host's live-callback limit; `peek_for` only filtered tombstones without reclaiming them.

Cancellation now removes the entry in place with `BinaryHeap::retain`, immediately releasing its closure. Obsolete tombstones are removed entirely. Cancellation is O(n) in currently scheduled native timers, rather than retaining unbounded historical cancellations; a more complex indexed queue is not justified without measurement. Already-selected native callback batches retain their original semantics; the host's ready queue separately enforces JS cancellation before invocation.

Regression checks in `timer::tests` cover 4,096 long-deadline cancellations without polling, immediate captured-resource release, repeated cancellation, and cancellation of a rearmed repeat.

## Repeated IME preedit

Upstream `Editor::preedit` returns a splice even when the provisional text has
not changed. The runtime treated every splice as a value update, so repeated
preedit notifications (including cursor-only updates from an IME such as fcitx5)
could repeatedly dispatch `input` and feed the same value back through a controlled
binding.

The runtime compares incoming preedit bytes with the active composition without
allocating a text copy. Identical text updates only the selection; it does not
rewrite document text or dispatch another `input`. Changed provisional text,
cancellation that removes provisional text, and final commits still announce
their values. This is not a commit-only input policy.

Regression gate:
`cargo test --manifest-path vendor/zgui-runtime/Cargo.toml --test controlled --test value --test editing`.
The headless native-window tests cover repeated and cursor-only preedit, UTF-8
and multiline text, controlled echoes, identical-text commit, undo, cancellation,
and resumed typing. They do not automate a live fcitx5 session.

## IME positioning across caret blink

`Window::caret_rect` previously read the painted highlight rectangles. While the
caret was blinked off, it returned no rectangle (or a selection band), so IME
positioning fell back to the field box instead of the insertion point. Alternating
blink phases therefore sent different cursor rectangles to Wayland even without
typing, allowing candidate-window repositioning and preedit replies to feed back.

The caret plan now retains logical insertion geometry independently of its painted
marks. IME positioning uses only that geometry, transformed with the current line
fragment; hidden phases still paint no caret. Actual cursor movement continues to
update the candidate position.

Regression gate:
`cargo test --manifest-path vendor/zgui-runtime/Cargo.toml --test controlled --test caret --test editing --test secure`.
The native headless regression advances four blink phases during composition,
with and without a selected preedit range, and checks stable surface IME geometry,
continued visual blinking, and subsequent cursor movement. Live fcitx5 popup
rendering remains a manual verification boundary.

## Primary layer configuration

`Runtime::primary_surface_attributes_mut` exposes the retained primary surface
specification only while that surface is absent. The native host uses it before
opening a layer and after output loss, without replacing the standard graphics,
text, or document factories. It refuses mutation while open or permanently closed.

`zgui-platform-wayland/` is also copied from the same pinned revision, with its
LICENSE and NOTICE and standalone manifest. Four focused protocol changes apply:

- The layer namespace comes from `SurfaceAttributes.application_id`, retaining
  `zgui` only when no application ID was supplied. This permits compositor rules
  to identify a native layer surface.
- Explicit zero width/height remains zero in `set_size`, as required to stretch
  between opposite anchors. Positive dimensions retain upstream rounding.
- One initial roundtrip completes bound output descriptions before
  `surfaces_available`. Without it, a connected named monitor was incorrectly
  reported as unavailable because its name had not reached the client yet.
- Output removal excludes the departing proxy when rebuilding monitor and scale
  inventory. SCTK invokes `output_destroyed` before removing that proxy from its
  own cache; reading the cache unchanged republished a disconnected monitor and
  prevented the host from suspending/recreating its layer.

The host validates stretch anchors and an explicitly named available monitor.
It always supplies a nonnegative exclusive zone: the pinned backend's `None`
emits protocol `-1`, which ignores other exclusive zones rather than reserving
the layer's extent.

Historical verification used the former panel example and the now-retired
`shell::proof::real_panel_layer` gate. It opened a real layer, observed niri
service delivery without input, rejected keyboard focus and checked resource
cleanup. It reproduced the initial-output race before the roundtrip fix; an
explicitly authorized output off/on run reproduced the stale output-cache bug
before its fix. The panel, niri service and compositor-specific gate are no
longer shipped. These generic layer-shell patches remain upstream wrappers:
platform and window-manager capabilities are determined by zgui, with no
compositor-specific package or abstraction in this library.

## Dynamic shell surfaces and input regions

`zgui-platform/` and `zgui-platform-headless/` retain the same pinned revision,
LICENSE and NOTICE. The platform contract adds `#[non_exhaustive]` `InputRegion::{Full, Empty, Rects}` (satisfying `zgui-platform::api::every_public_enumeration_is_extensible`)
in surface-local logical pixels, in-place layer reconfiguration, and a monitor
inventory callback. Other backends can leave unsupported operations unimplemented.

`zgui-platform-wayland/` copies a rectangle union into `wl_surface`, destroys the
temporary `wl_region`, and commits immediately. Full input uses a null region;
pass-through uses a non-null empty region. Live layer updates preserve the role
and its output, send positive transitional dimensions before removing an old
stretch axis, and commit the final anchors and dimensions together. Layer
creation and reconfiguration reject sizes above the unsigned layer-shell limit;
ordinary resize requests reject sizes above the signed buffer and viewport
limit. Both checks widen the CSS dimensions before comparing to avoid rounding
the protocol maxima up. An unknown named output fails creation; moving to
another output requires closing and reopening. Backend notifications coalesce
output additions, removals, and geometry updates, using core `wl_output`
geometry when logical position is unavailable.

`zgui-runtime/` can start without its primary window and exit explicitly instead
of exiting when its last surface closes. Synchronous open and close reuse the
existing window lifecycle, and a retained layer token keeps updated placement
across a platform suspend. Application-scoped timers use the platform clock and
park deadlines even when no window can draw. The offscreen backend accepts
resolved positive layer sizes and holds input regions for scripted input; its
driver supplies monitor mutations and notifies the handler.
Preview can dispatch wakes without drawing or moving the clock, so it can
configure newly opened surfaces before their first frame.

Rectangle bindings must stop accepting input when their node loses its layout
box. `window/observe.rs` sends a zero border box on that transition; other
geometry quantities keep their existing absence behavior.

The protocol regression uses a private Unix socket and asserts real Wayland
requests and commits from the backend without a desktop, GPU, or external server
crate. It checks hits in both committed rectangles and pass-through in their gap
and beyond them, and rejects oversized resize requests. Existing live compositor
regressions still run when a Wayland display is configured.

    cargo test --offline --manifest-path vendor/zgui-platform-wayland/Cargo.toml --test layer_protocol
    cargo test --offline --manifest-path vendor/zgui-runtime/Cargo.toml --test shell_lifecycle

## Layer-anchored and nested popups

`zgui-platform` adds a zero-default logical offset and opt-in reactive
positioning to popup placement, plus an in-place reconfiguration contract.
`zgui-platform-wayland` assigns a layer parent before the first bufferless
commit and retains the XDG path for toplevel and popup parents. It quotes an
eligible seat press for grabs; nesting a grab requires the parent to own the
topmost grab. Popup buffers wait for their parents to map. XDG version 3
supports reactive positioners and live repositioning; updates staged before
the first buffer apply once it is committed, using the parent's current
extent. Older versions retain the initial placement. The host reconciles
`popup_done` before issuing placement changes or opening children, even
when the destroyed-surface event is still queued.

Wayland retires popup roles newest-first, even if callers retain surface
handles. Closing an older popup can also retire higher sibling branches in
the same XDG stack. Headless closes descendants before their parents,
validates parent roles, and retains the requested placement and size; it
does not emulate edge flipping or input grabs. `zgui-runtime` closes child
documents first, forwards live placement changes, and discards transient
popups on suspend rather than resuming against stale parent IDs. On primary
close, the host disposes the popup tree before the primary document.

    cargo test -j 4 --manifest-path vendor/zgui-platform-wayland/Cargo.toml --test popup_protocol --offline

## Declarative background blur (`ext_background_effect_v1`)

`zgui-platform` adds `#[non_exhaustive] BlurRegion<'a> { None, Full, Rects(&'a [Rect<CssPx, Css>]) }`, `Surface::set_blur_region` (default no-op), and `PlatformCapabilities::supports_background_blur` (default `false` in `none()`). `zgui-platform-headless` records normalized blur state (`RecordedBlurRegion`) on `OffscreenSurface` for preview evidence while ignoring lock surfaces.

`zgui-platform-wayland` binds optional `ext_background_effect_manager_v1` (`1..=1`) from pinned `wayland-protocols 0.32.13` (no local XML), starts `supports_background_blur` at `false` until the live manager advertises `Capability::Blur`, tracks dynamic registry add/remove while ignoring late events from retired managers, and stages updates through `send_regions` / `owes_surface_change` / `pre_present` / `finish_redraw` / remap without touching input or opaque pending state or committing unconditionally in the setter. Each eligible surface (`Toplevel`, `Layer`, `Popup`) lazily creates at most one `ext_background_effect_surface_v1` when non-empty blur is desired and `Blur` is advertised, encodes `Full` as `add(0, 0, i32::MAX, i32::MAX)` and `Rects` via `region::outward` after finite/range validation, destroys temporary `wl_region` objects immediately, preserves desired intent across capability loss/regain, destroys `ext_background_effect_surface_v1` strictly before `wl_surface.destroy()` on both early role retirement and `Drop`, and hard-excludes `SessionLockSurface` (both persistent lock parent and `Role::LockContent` subsurface).

    cargo test -j 4 --manifest-path vendor/zgui-platform-wayland/Cargo.toml --test background_effect_protocol --offline

## Package image authority and capture readiness

Package images retain verified immutable bytes rather than reopening ambient
filesystem paths. `ImagePolicy::package_assets` preserves ordinary asynchronous
native publication; `package_only` additionally stages complete worker batches
in launch order for deterministic preview checkpoints. Neither path fabricates
decoded pixels or substitutes a separate image renderer.

`with_file_prefixes` grants Linux worker reads for `file:///` URIs only. Paths
use canonical slash components and percent-encoded URI characters such as
`%20` for spaces. Linux resolves from the granted directory with `openat2`;
relative symlinks confined beneath it are allowed, but prefix symlinks, magic
links and escapes are denied. Only regular files up to 64 MiB are read. The
live reader requires `openat2` and `/proc/self/fd` and fails closed otherwise.
`with_fixture_files` instead requires an exact URI mapped to owned package
bytes under a granted component prefix, with no ambient filesystem fallback.

Only `file://` and `zgui-blob://` sources accept UTF-8 SVG as well as raster
images. Plain package-relative sources retain the raster decoder. The pinned
CPU renderer (`resvg` with `raster-images`, `text`, and `system-fonts`) uses
the same decode limit and native atlas as raster images. SVG DTDs, nested SVGs
and external resources are disabled; bounded embedded PNG/JPEG/WebP data URIs
are verified before decode. SVG `<text>` resolves against an `ImagePolicy`-bound
`fontdb::Database` (`ImagePolicy::with_svg_fonts`, fed from `Fonts::svg_font_snapshot`):
live desktop sessions bind cached system fonts plus active session/package font
leases, while deterministic preview sessions (`Fonts::shipped_only`) bind only
the profile and package font faces mapped onto CSS generic families with no
ambient host font lookup or network access.

Each session owns one `ImageBlobs` table. Clones share its encoded bytes;
the URI is opaque and never hits the process-global `zgui-bytes:` registry.
Release unregisters the bytes immediately. On the next native settle, bound
pixels are retired and pending results for that handle are discarded.
Superseded blob assignments are skipped before policy checks at settle.

Policy replacement retires the old completion domain, blob table, pending
source writes, bindings, intrinsics, shared tiles and replay leases. The host
commits after the staged mount succeeds and the previous session retires,
retaining only source writes from the staged mount. Attribute-hook queue
identity and monotonic request/atlas identities survive, so same-path reloads
cannot reuse old bytes or accept delayed old-generation completions.

Resource snapshots distinguish decode/attachment from actual atlas allocation,
successful texture upload and current residency. Native placed geometry, viewport,
ancestor clips and current opacity determine which images are required. Offscreen
images do not block capture, but become upload requirements when visible. Image
retirement runs after DOM reclamation and before completed-frame observation,
including paint-skipped frames; removed nodes cannot remain required resources.

Regression gates in the standalone runtime:

```sh
cargo test --manifest-path vendor/zgui-runtime/Cargo.toml --test image_policy --test images_end_to_end
cargo test --manifest-path vendor/zgui-runtime/Cargo.toml --lib delayed_
cargo test --manifest-path vendor/zgui-runtime/Cargo.toml --lib live_package_images_
```

These cover real atlas refusal, viewport/clip/opacity transitions, controlled
delayed decode publication, replacement/clear retirement, file capability and
symlink enforcement, fixture-only SVG and blob disposal. The host's real GPU and Wayland pixel evidence is
recorded in [preview verification](../docs/development/preview-verification.md#p3--shared-time-fixture-providers-and-native-resources).

## Geometry observation deliveries

`zgui-view-dom/` is copied from the same pinned revision `ee29c7dc66206eb04ba7bcf514058a278c9070ba`, with its Apache-2.0 LICENSE and NOTICE and standalone manifest; the same patch pattern selects it for the whole graph. Its delivery patch applies:

- Observation callbacks are invoked outside the registry's `RefCell` borrow, so a callback may register, cancel itself or cancel a sibling.
- Each registration receives its own fresh post-layout sample, including a first zero measurement and subscribers added inside another delivery; a prepared callback whose registration was cancelled is skipped.
- Duplicate values are suppressed per watched quantity through a cached last sample, so equal geometry is not replayed.
- A device-pixel-ratio change invalidates pending samples, so unchanged device geometry is redelivered under the new scale.
- The two-pass delivery budget and next-frame settlement bound are unchanged.

Regression gates:

```sh
cargo test --manifest-path vendor/zgui-view-dom/Cargo.toml --test observations
cargo test --manifest-path vendor/zgui-runtime/Cargo.toml --test geometry_observation --test observe_owed --test observed_place --test scale
```

Remove each local patch only when an upgraded pinned upstream revision implements its contract and passes the corresponding gate. Do not silently drop patches on dependency updates.

## Text replay content identity

`zgui-paint/` is copied from the same pinned revision `ee29c7dc66206eb04ba7bcf514058a278c9070ba`, with its Apache-2.0 LICENSE and NOTICE and standalone manifest; the same patch pattern selects it for the whole graph. Its replay signature patch applies:

- A `Line`/`TextRun` fragment's painted content signature carries the shaped paragraph key, because layout reclaims and reuses paragraph slots across the prepaint layout passes a single frame can run. Without it, a recycled slot replays the previous frame's glyph pixels for different shaped content.

Regression gates:

```sh
cargo test --manifest-path vendor/zgui-runtime/Cargo.toml --test geometry_observation --test damage --test shared_text
cargo test --manifest-path vendor/zgui-paint/Cargo.toml --lib walk::replay
ZGUI_KEEP_PREVIEW_OUTPUT=1 node --test tests/preview/geometry.test.mjs
```

Remove each local patch only when an upgraded pinned upstream revision implements its contract and passes the corresponding gate. Do not silently drop patches on dependency updates.

## Inherited colour on raw text items

`zgui-layout/` is copied from revision `ee29c7dc66206eb04ba7bcf514058a278c9070ba` with its LICENSE and NOTICE. Standalone manifests and the root patch retain the pinned dependency graph.

Raw text items in Flex and Grid own text-change marks while inheriting their parent's appearance. Fragment damage now includes the style generator's repaint/reshape marks for these items. Colour transitions repaint their glyphs without remounting or forcing full-surface damage.

Regression gates:

    cargo test -p zgui-runtime --test damage raw_
    ZGUI_REPAINT_WALK=off cargo test -p zgui-runtime --test damage raw_
    ZGUI_REPAINT_WALK=shadow cargo test -p zgui-runtime --test damage raw_
    ZGUI_KEEP_PREVIEW_OUTPUT=1 node --test tests/preview/disclosure.test.mjs

The recording renderer checks every incremental frame; the last frame alone can contain no glyphs when its damage is empty.

## Cross-parent node moves

`zgui-dom/` is copied from revision `ee29c7dc66206eb04ba7bcf514058a278c9070ba` with its LICENSE and NOTICE and a standalone manifest.

Cross-parent insertion marks the old parent for child-list rebuilding, layout, repaint, and accessibility updates. This retires the moved subtree's old layout and paint records even when its new parent is detached. Same-parent reorders retain their existing path.

Regression gates:

    cargo test -p zgui-runtime --test damage raw_portal_
    ZGUI_REPAINT_WALK=off cargo test -p zgui-runtime --test damage raw_portal_
    ZGUI_REPAINT_WALK=shadow cargo test -p zgui-runtime --test damage raw_portal_
    ZGUI_KEEP_PREVIEW_OUTPUT=1 node --test tests/preview/portal.test.mjs

## Focus restoration after native defaults

Automatic focus-trap restore commands retain their source scope and recheck focus when they
are carried out. A trap removed synchronously from a capture listener — before the pointer
framework default has applied — is restored against the focus that remains after the default:
a press that landed on a control outside the surface keeps the focus it was given, and
`preventDefault` on that press keeps the restore.

## Animation counts on generated content

`vendor/zgui-style` is pinned upstream zgui rev `ee29c7dc` with one change:
`Animations::running_on` sums the running animation/transition count across the
element's own set and its native `::before`/`::after` sets. Each motion counts
individually, finished motions kept for a lifecycle edge are not running, and
descendants never contribute. The lookup reads three fixed rows — no table
scan.

Regression gates:

    cargo test -p zgui-runtime --test anim_lifecycle
    cargo test -p zgui-style --lib driver::animations
    ZGUI_KEEP_PREVIEW_OUTPUT=1 node --test tests/preview/animation.test.mjs

## Frozen elastic scrolling

`zgui-scroll/` is copied from revision `ee29c7dc66206eb04ba7bcf514058a278c9070ba` with its LICENSE and NOTICE. Its standalone manifest and the root patch retain the pinned graph.

The runtime pauses elastic advancement on a frozen window root and excludes that paused edge from animation deadlines. It preserves displacement and resumes released edges on thaw. Other containers keep advancing. Existing root input filtering and motion cancellation remain in place.

Regression gates:

    cargo test --manifest-path vendor/zgui-scroll/Cargo.toml --lib
    cargo test --manifest-path vendor/zgui-runtime/Cargo.toml --test scroll_root --test scroll_touchpad --test scroll_owed
    ZGUI_KEEP_PREVIEW_OUTPUT=1 node --test tests/preview/scroll-lock.test.mjs

## StyleX stylesheet preflight and revisions

`zgui-style/` and `zgui-runtime/` retain upstream revision `ee29c7dc66206eb04ba7bcf514058a278c9070ba` and their upstream licenses. The style crate exposes preflight through the actual sheet parser, returning structured diagnostics without installing a candidate. The `zgui-stylex-v1` profile rejects parser recovery and unsupported browser-root RTL wrappers, typed/general `@property`, container/reduced-motion queries, CSS resource URLs/imports/image-set, and `@font-face`; official priority selectors and keyframes remain unchanged. Official dynamic-function `@property --* { syntax:"*"; inherits:false }` rules use Stylo's registration machinery, including descendant noninheritance and replacement/removal cleanup.

The runtime advances a per-document style revision only for applied stylesheet mutations. Identical replacements and absent removals are no-ops, same-name replacement retains cascade position, and frame observations include the consumed revision plus pending-sheet state. Removal also invalidates stale rule-chain caches before early frame filtering.

Regression gates:

    cargo test -j 4 --manifest-path vendor/zgui-style/Cargo.toml --test stylex_profile --offline
    cargo test -j 4 --manifest-path vendor/zgui-runtime/Cargo.toml --test style_revisions --offline

The runtime performs SDK stylesheet preflight in its native command phase rather than inside the JavaScript entry budget. Mount-generated candidates are additionally validated after the JS mount turn and before the previous session retires. `HostBinding::stylesheet_rejected` reports the slot and structured diagnostics before any cascade mutation; rejected replacements preserve the previous complete sheet and style revision. The regression test covers invalid replacement, recovery, and diagnostic delivery.

## Atomic application inline styles

`zgui-view/`, `zgui-view-dom/`, and `zgui-dom/` retain the pinned upstream revision with their upstream licenses. `Dom::replace_inline_style` validates the complete application contribution before replacing the native declaration block. Application, SDK-default, direct, and invariant declarations remain separately owned, with shorthand expansion and string/object transitions handled by the actual native parser. Rejection changes neither declarations nor style hints.

Regression gates:

    cargo test -j 4 --manifest-path vendor/zgui-dom/Cargo.toml --lib mutate::edit::style --offline
    cargo test -j 4 -p ui-bridge --test properties --offline

## Session-scoped application fonts

`zgui/` and `zgui-text-parley/` retain the pinned upstream revision and licenses. `Fonts::register_scoped` returns a `FontLease` whose drop restores the predecessor or removes only that registration, so an old session cannot unregister its successor's alias. Permanent Preview/profile faces remain intact. `Fonts::contains_family` provides exact family existence checks without treating fallback as registration.

Regression gates:

    cargo test -j 4 -p native-host package:: --offline
    cargo test -j 4 -p native-host session_host::tests --offline
    node --test --test-concurrency=1 tests/preview/stylex.test.mjs

## Compositor session lock

`zgui-platform` distinguishes the lock role from layer shell and offers
explicit request, state query and unlock operations. `zgui-runtime` forwards
those operations without making lock ownership a window lifetime.

`zgui-platform-wayland` requires ext-session-lock-v1 and `wl_subcompositor`.
It acknowledges configure before the first non-null commit and retains an opaque
parent surface on each output. Application content is a disposable subsurface;
releasing it exposes the native fallback without changing the lock role.
Only explicit unlock of an acquired lock sends `unlock_and_destroy`, with a
display sync before destroying its surfaces. Acquiring, refusing, hotplugging
and disconnecting without unlocking are exercised on a private socket.

`zgui-platform-headless` models the acquisition decision, per-output coverage,
fallback and explicit unlock for the Preview fixture; no live desktop is locked.

    cargo test -j 4 --manifest-path vendor/zgui-platform-wayland/Cargo.toml --test session_lock_protocol --offline

## New text-node dirty propagation

Inserting a brand-new `NodeKind::Text` node into an existing element via `insert_before` previously marked only `Dirty::CHILDREN` on the parent and `Dirty::RESTYLE | Dirty::A11Y` on the child. Unlike new elements, text nodes do not run the style engine's first-time cascade branch that marks `Dirty::RELAYOUT | Dirty::REPAINT`, so `fragment::diff` could skip an unchanged-size ancestor (such as `<column><field /></column>` when `Editors::adopt` inserts an empty text node into a focused `<field>`) and leave the rebuilt box with zero fragments.

`vendor/zgui-dom/src/mutate/edit/build.rs` now marks `Dirty::RELAYOUT | Dirty::REPAINT` on the parent when inserting a `NodeKind::Text` child, preserving element-insertion first-cascade invariants while ensuring rebuilt text hosts receive fragments and painted carets.

Regression gates:

    cargo test -j 4 --manifest-path vendor/zgui-dom/Cargo.toml --test engine --offline
    cargo test -j 4 --manifest-path vendor/zgui-runtime/Cargo.toml --test caret --offline

## Desktop portal reduced-motion preference

`zgui-platform` adds a default `PlatformCx::reduced_motion(&self) -> Option<bool>` query so backends that do not expose a system reduced-motion signal remain source-compatible.

`zgui-platform-wayland` extends its existing `zgui-appearance` D-Bus watcher on `org.freedesktop.portal.Settings` (`org.freedesktop.appearance`) to read and watch `reduced-motion` (`0` = no preference / `Some(false)`, `1` = reduce / `Some(true)`, other = `None`) alongside `color-scheme`. Changes wake the driver loop with `WakeReason::AppWork` so `SessionHost` propagates the preference live to mounted bridges unless overridden by `--reduced-motion` or a preview profile.

Regression gate:

    cargo test -j 4 --manifest-path vendor/zgui-platform-wayland/Cargo.toml --lib theme --offline

## Clipped background bounds and per-side border styles

`zgui-paint` clips `clip-path` background `VectorItem`s in both vector-local space (`VectorClip` for `0..width, 0..height`) and scene clip space (`ClipLink::shaped` for `border_box` with radii/corner shape) so shapes extending past `border_box` (e.g., `circle(45%)` on a wide/short box) never paint or bleed into neighboring elements.

`BorderPaint` also retains per-side `styles: [LineStyle; 4]` including `LineStyle::Double`. Uniform non-double borders keep the single-quad fast path; mixed per-side styles or `double` borders emit independent per-style border quads (with `double` rendered as outer + inset inner solid bands when width >= 3 device px).

Regression gate:

    cargo test -j 4 --manifest-path vendor/zgui-paint/Cargo.toml --lib --offline

## Editable text query for UTF-16 selection conversion

`zgui-view` adds a default `ViewHost::editable_text(&self, node: NodeId) -> Option<String>` method (implemented on `StubHost` and `RuntimeHost`) returning the unmasked editable text joined across paragraphs with `\n` in the exact UTF-8 byte coordinate space used by `ViewHost::selection` / `ViewHost::set_selection`.

`zgui-runtime` mirrors `RuntimeHost::set_value` and `Editors` text updates into `RuntimeHost::values` (falling back to `selection::text_of` on the DOM subtree) so `editable_text`, `set_selection`, and `select_all` stay accurate across multiline text, secure fields, and same-frame `set_value` + `set_selection` calls.

Regression gate:

    cargo test -j 4 --manifest-path vendor/zgui-runtime/Cargo.toml --offline && cargo test -j 4 -p ui-bridge --offline

## Window state tracking & synchronous close-veto hook

`zgui-platform-headless` extends `OffscreenSurface` to record `set_decorations`, `set_maximized`/`is_maximized`, `set_minimized`/`is_minimized`, `set_fullscreen`/`fullscreen`, and `begin_move_drag` (`move_drag_count`) so headless preview scenarios can assert runtime window control changes deterministically.

`zgui-runtime` adds `Runtime::surface_event_with_close_veto` (wiring an external synchronous `CloseResponse::Veto` check into `SurfaceEvent::CloseRequested`) and `Runtime::close_window_surface` (closing a live window by `SurfaceId` and applying exit policy for forced closes).

Regression gate:

    bun run test:vendor zgui-platform-headless && bun run test:vendor zgui-runtime

## Clipboard PNG image MIME & write-options tracking

`zgui-platform-wayland` extends clipboard MIME negotiation to advertise and serve `image/png` (`ClipboardFormat::Image`) alongside plain text, and advertises/serves `x-kde-passwordManagerHint` (`"secret"`) when `ClipboardWriteOptions::exclude_from_history` is set.

`zgui-platform-headless` records `ClipboardWriteOptions` per clipboard slot on `MemoryClipboard` (`snapshot` and `last_write_options`) for deterministic preview verification, and `zgui-runtime` exposes `ImageBlobs::lookup` as `pub` so host services can read encoded bytes for a session-owned `ImageHandle` without routing pixels through the QuickJS heap.

Regression gate:

    bun run test:vendor zgui-platform-wayland && bun run test:vendor zgui-platform-headless && bun run test:vendor zgui-runtime

## Hit-tested drag-and-drop and Wayland offer-slot isolation

`zgui-platform-wayland` replaces the single shared `Option<Vec<PathBuf>>` drag-read slot in `input::dnd::Drag` (`DragTracker<O>`) with a bounded queue of `ReadCompletion` items tagged by `OfferKey { offer_id, generation }`. Delayed pipe completions from a superseded or source-cancelled offer are strictly discarded instead of contaminating a newer offer (even when the compositor reuses a protocol ID), and `unescape` decodes percent-encoded `file://` URIs as UTF-8 bytes.

`zgui-runtime` routes `SurfaceEvent::Drag(DragEvent::Dropped { position, .. })` via `world.chain_at(device_point)` instead of keyboard focus, and exposes `Window::chain_at_css` and `Window::node_box_css` so internal reorder DnD and external `Entered`/`Moved`/`Left`/`Dropped` hover/drop negotiation share the exact native paint-clip, transform, scroll-offset, and scale hit-test pipeline.

Regression gate:

    bun run test:vendor zgui-platform-wayland && bun run test:vendor zgui-runtime && bun test --isolate tests/preview/dnd.test.mjs

## Outbound Copy-only Wayland drag source

`zgui-platform` adds `OutboundDragPayload`, `DragEvent::OutboundFinished { accepted }`, and default `Surface::begin_outbound_drag` / `Surface::cancel_outbound_drag` methods.

`zgui-platform-wayland` tracks live active pointer/touch presses on `SeatLink` (`press_started`/`press_ended`/`live_drag_serial`, cleared on release, leave, or cancel so stale serials are rejected) and wires all `DataSourceHandler` callbacks in `driver/data.rs` (`send_request`, `accept_mime`, `action`, `dnd_dropped`, `dnd_finished`, `cancelled`) to `Selections`. Outbound drags create a `DragSource` advertising `DndAction::Copy` only (never `Move`) and explicitly destroy `wl_data_source` on finish, cancellation, or drop. `zgui-platform-headless` tracks `live_press` and outbound drag counts on `OffscreenSurface` for deterministic preview verification.

Regression gate:

    bun run test:vendor zgui-platform && bun run test:vendor zgui-platform-wayland && bun run test:vendor zgui-platform-headless && bun test --isolate tests/preview/dnd.test.mjs

## Per-contour synthetic bold winding & CSS weight threshold

`swash 0.2.10`'s `LayerMut::embolden` calls `compute_winding(self.points)` over the entire flattened point buffer without splitting contours, so cross-contour jumps between contour `k` and contour `k + 1` can invert the winding sign on multi-contour CJK glyphs and erode strokes instead of thickening them. `vendor/zgui-text-parley/src/raster/glyphs.rs` bypasses `Render::embolden` when `synthetic_bold != 0.0`: it extracts the scaled outline, computes layer winding strictly by summing each closed contour's individual signed area, emboldens each contour with that winding sign, applies synthetic slant, and rasterises via `zeno::Mask` while preserving color-layer and bitmap-only fallbacks.

`vendor/zgui-text-parley` shaping (`src/shape/glyphs.rs`, `src/shape/frozen.rs`, `src/shape/line.rs`) also aligns synthetic bolding with CSS Fonts 4 browser conventions by enabling `SYNTHETIC_BOLD_RATIO` only when `requested_weight >= 600` and the matched face weight is `< 600` (so `font-weight: 500` on a `400` face no longer applies heavy faux-bold).

Regression gate:

    bun run test:vendor zgui-text-parley

## Opt-in cross-block read-only `<text>` document selection

`zgui-view` adds `DocumentSelectionSnapshot` and default `ViewHost::{document_selection, set_document_selection, clear_document_selection, select_document_all, node_text}` hooks.

`zgui-runtime` adds a dedicated `window::doc_select` module (strictly separate from `editing.rs`) storing directional `(source_text_node, byte_offset, affinity)` endpoints on `Window` scoped to an opt-in `data-zgui-document-selection="true"` container. Pointer hit-testing maps through each inline formatting context's `TextMap` (`Located::hit_source`) and renders multi-block selection highlights via `caret::Plan` (`plan_for_document_bands` + `Located::bands_for_sources`) through `window/caret.rs`'s scroll/transform/damage pipeline. Selection respects `user-select`, excludes editable/secure inputs and nested containers, copies in document order (`\t` across `role="row"` cells, `\n` across blocks, never inserting `\n` on soft line wraps), recomputes bands on reflow, and clears safely when an endpoint detaches.

Regression gate:

    bun run test:vendor zgui-runtime && bun test --isolate tests/preview/document-selection.test.mjs

## Probe-gated frame phase timings (`CompletedFrame::phases`)

`vendor/zgui-runtime/src/probe.rs` and `src/window/frame.rs` add `FramePhaseTimings` (`style_us`, `layout_us`, `paint_us`, `total_us`) to `CompletedFrame`, sampled strictly when a `FrameProbe` is attached (`self.probe.is_some()`). Non-probed production and release frames incur zero timing syscalls and zero overhead. Used by `native-host`'s dev-only read-only runtime inspector (`--dev-inspect`).

Regression gate:

    cargo test -p native-host inspector --offline && bun run test:vendor zgui-runtime
