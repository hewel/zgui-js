# Deepen Menu Hierarchy, Preview Output Sealing, and Package Publication

**Status:** resolved

## Problem Statement

Maintainers and agents face architectural friction across three remaining seams (Candidates 5–7 from the architecture review):

1. **Menu hierarchy state, surface orchestration, and item construction leak across five UI modules (`packages/ui/src/components/{menu,dropdown-menu,menu-sub,context-menu,menubar}.ts`)**:
   - `menu.ts` exports 10 internal-only symbols (`MenuContext`, `MenuSurfaceContext`, `MenuRadioContext`, `menuRoot`, `createTypeaheadHandler`, `menuSurface`, `menuRow`, `menuIndicator`, `createMenuCheckedState`, `createMenuRadioValue`) just to wire sibling menu modules.
   - `menu-sub.ts` (`subClosings`) and `context-menu.ts` (`anchorState`) patch cross-module state onto `OverlayState` through module-global `WeakMap` side channels.
   - Pressing `Escape` inside an open `MenuSubContent` closes the branch without returning focus to `MenuSubTrigger` (diverging from `ArrowLeft`), while selecting a leaf item with `closeOnSelect` calls only root `dismiss()` without cascading `setOpen(false)` across open `MenuSub` ancestors (failing to fire controlled `MenuSub` `onOpenChange(false)` or start submenu exit transitions synchronously).
   - `menubar.ts` duplicates surface focus/typeahead/arrows setup (`trap: false`) and 6 item/part builders (`MenubarItem`, `MenubarCheckboxItem`, `MenubarRadioItem`, `MenubarLabel`, `MenubarSeparator`, `MenubarShortcut`).

2. **Preview **Artifact inventory** sealing, controller finalization, and bounded file reads split across subcommands and six readers (`crates/native-host/src/preview/{output,supervisor,mod,run,compare,saved,evidence,profile,fixture,scenario}.rs`, `crates/native-host/src/{package,shell}.rs`)**:
   - `run` seals worker output on disk (`seal_worker` / `restore_worker` rehashing) while `doctor` passes an unverified in-memory `OutputState` over stdout (`adopt`).
   - `doctor` leaves `--out` directories unsealed when the controller summary exceeds `8 KiB`.
   - `compare` uses the repo's lone `E_INTERNAL` (exit `9`) instead of `E_OUTPUT_LIMIT` (exit `8`) on `8 KiB` stdout overflow, leaves `--out` unsealed, and drops `execute`'s `firstError` whenever `finish` fails.
   - `Output` stores untyped `Vec<Value>` inventory entries instead of strongly typed **Artifact inventory** records, and six modules duplicate bounded file-read loops while five omit `O_NOFOLLOW | O_NONBLOCK` guards.

3. **Bundle compilation, **Package publication**, and watch orchestration leak bundler and subprocess mechanics (`packages/build/src/{compiler,build,watch,typecheck}.mjs`)**:
   - `compiler.mjs` exports low-level `compilerOptions` / `outputOptions` / `compilationResult` fragments (including StyleX pass metadata/profile checks), forcing `build.mjs`, `watch.mjs`, and `tests/build/compiler.test.mjs` to assemble Rolldown plugins, manually delete `output` keys, and close bundle handles.
   - Atomic **Package publication** (`dist` symlink swap in `build.mjs` vs `.zgui-build-status.json` atomic write in `watch.mjs`) is split across two modules and re-implemented by hand inside `tests/preview/watch.test.mjs`.
   - `watchApplication` lacks the third-argument lifecycle adapter seam already established by `devApplication` in `dev.mjs`, forcing watch status-transition and error-recovery tests through slow `tsc` subprocesses.

## Solution

Deepen these three seams into high-leverage modules with strict locality and complete tests at their public interfaces:

1. Consolidate menu hierarchy contexts (chained root and branch state plus `ContextMenu` anchor placement), surface keyboard/focus/typeahead orchestration, and parameterized item builders inside a new internal `packages/ui/src/components/menu-core.ts` module—retaining all 34 public menu exports in `@hewel/zgui/components` and `docs/development/component-map.md` parity, retiring `menu.ts`'s 10 internal exports, deleting both `WeakMap` side channels, cascading leaf selection outward across open `MenuSub` branches, and returning focus to `MenuSubTrigger` on submenu `Escape`.
2. Deepen `Output` with typed **Artifact inventory** records, move `doctor --out` onto `seal_worker` / `restore_worker` disk rehashing (retiring `OutputState`, `Output::into_state`, `Output::adopt`, and `supervisor::Report.output_state`), unify controller `8 KiB` stdout enforcement (`E_OUTPUT_LIMIT`, exit `8`), `originalError` preservation, and `Output` sealing across `doctor`, `run`, and `compare`, and centralize bounded file reading in `crates/native-host/src/bounded_fs.rs` behind two explicit entry points (`read_contained` and `read_input`).
3. Deepen `packages/build/src/compiler.mjs` into one-shot `compileBundle` and persistent `watchBundle` entries (retiring `compilerOptions`, `outputOptions`, and `compilationResult`), extract atomic **Package publication** (`publishGeneration`, `publishBuildStatus`, and status filename constants) into `packages/build/src/publication.mjs` shared by `build.mjs`, `watch.mjs`, and tests, and add a third-argument lifecycle seam to `watchApplication` matching `dev.mjs`.

## Implementation Decisions

### 1. Internal `menu-core.ts` Hierarchy, Surface Orchestration, and Item Builders

- **Public interface preservation**: Keep all 34 menu/menubar/context-menu/dropdown-menu/submenu exports in `packages/ui/src/components/index.ts` and `docs/development/component-map.md` 100% intact (including `MenuTypeahead`, `MenubarArrows`, `headingToward`, `useContextMenuPlace`, and `MenuGroup`/`MenubarGroup`).
- **Module consolidation (`packages/ui/src/components/menu-core.ts`)**:
  - **Root & branch hierarchy context**: Replace `MenuContext`, `subClosings` (`WeakMap` in `menu-sub.ts`), and `anchorState` (`WeakMap` in `context-menu.ts`) with explicit internal Solid contexts in `menu-core.ts`:
    - Root/branch hierarchy tracks the active `OverlayState`, parent branch reference, optional root/branch dismiss action, and submenu `closing` timer (`createDelayed`), eliminating both `WeakMap` globals.
    - When a leaf menu item with `closeOnSelect` triggers dismissal, walk the branch chain from the innermost active `MenuSub` outward—canceling each branch's deferred `closing` timer and invoking `state.setOpen(false)` (firing controlled `MenuSub` `onOpenChange(false)` and starting exit transitions synchronously)—before invoking root menu dismissal (`DropdownMenu`/`ContextMenu`: `setOpen(false)`; `MenubarMenu`: `setOpen(false)` + focus `MenubarTrigger`).
  - **Submenu `Escape` focus contract**: Supply `onDismiss` on `MenuSubContent`'s surface configuration so `reason === "escape"` closes only that innermost open `MenuSub` and returns focus to its `MenuSubTrigger` (matching `ArrowLeft`), while `outside-press` closes without forcing focus onto `MenuSubTrigger`. Keep `MenuContent` / `MenubarContent` root dismiss focus semantics unchanged (`trap: true` relies on focus trap teardown; `MenubarContent` uses `trap: false` and focuses `MenubarTrigger` only on `escape` or item dismissal, never on `outside-press`).
  - **Unified surface & item builders**: Internalize `menuSurface` (`trap: true` for standalone/sub menus; `trap: false` with optional `Menubar` arrow rail for `MenubarContent`), `menuRow`, `menuIndicator`, `createMenuCheckedState`, `createMenuRadioValue`, and parameterized `item` / `checkboxItem` / `radioItem` / `label` / `group` / `separator` factories. Preserve exact defaults and styling semantics: `MenuCheckboxItem` defaults `closeOnSelect` to `true` while `MenubarCheckboxItem` defaults `closeOnSelect` to `false`; preserve `__zguiClass` composition so root-forwarding StyleX class propagation keeps working.
- **Internal symbol retirement**: Retire all 10 non-public exports from `menu.ts` (`MenuContext`, `MenuSurfaceContext`, `MenuRadioContext`, `menuRoot`, `createTypeaheadHandler`, `menuSurface`, `menuRow`, `menuIndicator`, `createMenuCheckedState`, `createMenuRadioValue`).

### 2. Typed **Artifact inventory**, Unified Preview Output Sealing, and `bounded_fs`

- **Typed **Artifact inventory** & retired `OutputState`**:
  - Replace `Vec<serde_json::Value>` entries inside `crates/native-host/src/preview/output.rs` with typed Rust structs/enums representing each entry's `path`, `role`, `status` (`complete`, `historical`, `partial`), `bytes`, and `digest`, serializing identically to the existing `artifacts.json` / `worker-artifacts.json` schema.
  - Retire `OutputState`, `Output::into_state`, `Output::adopt`, and `supervisor::Report.output_state`.
  - Update `doctor_worker` (in `crates/native-host/src/preview/mod.rs`) to seal `--out` directories via `Output::seal_worker()`, matching `run::worker`.
- **Unified controller finalization (`doctor`, `run`, `compare`)**:
  - Extract a shared controller finalization helper on `Output` (or `preview::output`) used by `doctor`, `run`, and `compare`:
    1. Enforces the `8 KiB` (`MAX_STDOUT_BYTES`) bound with `Error::new("E_OUTPUT_LIMIT", 8, ...)` (replacing `compare.rs:160`'s lone `E_INTERNAL` exit `9`).
    2. Preserves the earliest cause in `originalError` whenever stdout serialization/size checks or `Output::finish` fail.
    3. Seals `--out` (when present) by writing `artifacts.json` and `result.json` even when an earlier summary or `restore_worker` check fails, without erasing `originalError`.
  - `doctor` and `run` both restore sealed worker inventories via `restore_worker()` (with failure recovery via `recover()` + `evidence::recover` where applicable).
- **Single bounded regular-file reader (`crates/native-host/src/bounded_fs.rs`)**:
  - Separate two explicit contracts instead of conflating symlink policy across callers:
    1. `read_contained(root: &Path, relative: &str, limit: u64) -> Result<Vec<u8>, BoundedReadError>`: Validates canonical relative path components (no `.`, `..`, `\`, empty segments, or non-normal components), rejects symlinks at every traversed directory component via `symlink_metadata`, opens with `O_NOFOLLOW | O_NONBLOCK` on Unix, `fstat`-verifies `is_file()` and `len() <= limit`, and reads through `take(limit + 1)` to catch concurrent growth. Replaces open-coded readers in `package.rs`, `preview/saved.rs` (`open_file`/`read_file`), and `preview/profile.rs` (`contained` / `read_bounded`).
    2. `read_input(path: &Path, limit: u64) -> Result<Vec<u8>, BoundedReadError>`: Permits top-level symlink arguments passed on the CLI (for example `--profile`, `--fixture`, `--scenario`, `--policy`, `--layer-config`, or controller-owned evidence files), opens with `O_NONBLOCK` on Unix (preventing hangs on FIFOs/devices), `fstat`-verifies `is_file()` and `len() <= limit`, and reads through `take(limit + 1)`. Replaces open-coded readers in `shell.rs`, `preview/profile.rs`, `preview/fixture.rs`, `preview/scenario.rs`, `preview/evidence.rs`, and `preview/saved::read_path`.
  - Each caller maps typed `BoundedReadError` variants (`InvalidPath`, `Symlink`, `NotRegularFile`, `TooLarge`, `Io`) to its established protocol error code (`E_BUNDLE`, `E_SCHEMA`, `E_FONT`, `E_OUTPUT_IO`, `E_OUTPUT_LIMIT`) with zero contract drift.

### 3. Deepened `compiler.mjs`, Shared `publication.mjs`, and `watchApplication` Seam

- **`packages/build/src/compiler.mjs`**:
  - Retire `compilerOptions`, `outputOptions`, and `compilationResult` exports.
  - Export two high-leverage bundler lifecycle entries that hide Rolldown plugin assembly, StyleX pass/origins validation, `sourcemapPathTransform`, `delete output[filename]` suppression, persistent Babel/StyleX transform caching, module pruning, and `compiler.close()` / `event.result?.close()` cleanup:
    - `compileBundle(entry, { root, appDirectory, hostEntry, filename, babelTransform, stylexDiagnostics, plugins })`: performs a one-shot Rolldown build in memory and closes the compiler handle.
    - `watchBundle(entry, { root, appDirectory, temporaryDirectory, filename, buildDelay, stylexDiagnostics, babelTransform, watchFiles, onAttempt, onCompile, onError })`: owns the persistent Rolldown watcher, suppresses disk writes during `generateBundle`, notifies callers with the validated `{ files, graph, solidRuntime, styleSystem }` result, closes `event.result`, and returns `{ close() }`.
- **`packages/build/src/publication.mjs`**:
  - Extract atomic **Package publication** primitives from `build.mjs` and `watch.mjs`: `BUILD_STATUS_FILE` (`.zgui-build-status.json`), `NATIVE_STATUS_FILE` (`.zgui-native-status.json`), `publishGeneration(appDirectory, files)` (atomic `.zgui-generations` + `.dist-<token>` symlink swap), and `publishBuildStatus(appDirectory, status)` (bounded `16 KiB` write + `.tmp` atomic rename).
  - Keep `buildApplication`, `bundle`, `contentDigest`, `publishGeneration` (re-exported from `publication.mjs`), `buildInfo`, `dependencyVersions`, `optionalFile`, `publishApplication`, and `workspaceRoot` intact on `packages/build/src/build.mjs` so existing preview/build callers outside this change remain unaffected.
- **`packages/build/src/watch.mjs` lifecycle adapter seam**:
  - Give `watchApplication(app, options, adapters)` an optional third-argument adapter object matching `devApplication`'s pattern in `packages/build/src/dev.mjs`, defaulting in production to `{ watchBundle, typecheck: typecheckProjectAsync, publish: publishApplication, publishStatus: publishBuildStatus }`. Keep the two-argument call signature and behavior identical.

## Testing Decisions

- **Principle ("the interface is the test surface" — replace internal plumbing tests cleanly)**:
  - Migrate `tests/build/compiler.test.mjs` off `compilerOptions`, `outputOptions`, and `compilationResult` to exercise `compileBundle`, `watchBundle`, and `publishGeneration`/`publishBuildStatus` directly, plus add fast deterministic `watchApplication` adapter tests covering `building -> failed -> published` status transitions, `previousDirectory` retention across failures, and `.zgui-build-status.json` publication errors.
  - Migrate `tests/preview/watch.test.mjs` helper functions (`publication`, `status`) to import `publishGeneration` and `publishBuildStatus` from `packages/build/src/publication.mjs` instead of hand-rolling `.dist-*` symlinks and `.zgui-build-status.json.*.tmp` writers.
  - Extend `tests/build/menu.test.mjs`, `tests/build/menubar.test.mjs`, and `tests/build/context-menu.test.mjs` through public JSX exports to verify submenu `Escape` returns focus to `MenuSubTrigger`, nested controlled/exit-deferred `MenuSub` branches cascade `onOpenChange(false)` on leaf selection, and `MenubarCheckboxItem` vs `MenuCheckboxItem` defaults remain preserved alongside StyleX class forwarding.
  - Add Rust unit tests covering:
    - `bounded_fs`: `read_contained` traversal/symlink escape rejection, `read_input` permitted top-level symlink vs non-regular Unix FIFO rejection (`O_NONBLOCK` without blocking) and byte-limit overflow (`TooLarge` before and during read).
    - `output` / `doctor` / `compare`: typed `Artifact inventory` serialization parity, `doctor --out` worker sealing + stdout overflow sealing with `originalError`, and `compare` `8 KiB` stdout overflow returning `E_OUTPUT_LIMIT` (exit `8`), sealing `Output`, and retaining `originalError`.

## Out of Scope

- `SX20` (CSS-only hot reload without full **Session** reload).
- Any modifications inside `vendor/` crates.
- Public JSX API or `docs/development/component-map.md` export changes in `@hewel/zgui/components`.

## Further Notes

All three tickets (`01` in `packages/ui`, `02` in `crates/native-host`, `03` in `packages/build` + build/watch tests) touch completely disjoint files and have no blocking dependencies (`Blocked by: none`), so they can execute in parallel or in any order.
