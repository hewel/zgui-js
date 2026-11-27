# `@hewel/zgui-services`

Capability-gated native host services for `zgui-js` applications.

## Install

```sh
bun add @hewel/zgui-services
```

`@hewel/zgui-services` is separate from `@hewel/zgui`. Its implementation and public types
are authored in strict ES2022 TypeScript without browser or Node globals, and compiled into
`dist/*.js` + `dist/*.d.ts` (`bun run build:packages`) with `#solid` zero-DOM isolation for
both `tsrs` and standard `tsc` consumers. Importing the module
does not inspect the host. Calling a service forwards to the private native
binding in the active session; start work from mount, lifecycle callbacks or
events, not module evaluation/preflight or disposal. Cleanup may cancel existing
work.

```ts
import { readTextAsset } from "@hewel/zgui-services";

const request = readTextAsset("assets/message.txt");
request.promise.then(
  text => console.log(text),
  error => console.error(error),
);
// In the UI owner's cleanup:
request.cancel();
```

`readTextAsset(path)` reads a manifest-declared UTF-8 asset, relative to the
application package. Put resources in the example's `assets/` directory: the
builder copies them and declares their paths in `manifest.json`. Absolute paths,
traversal, non-canonical paths and undeclared resources are not file access APIs.
Invalid arguments and undeclared paths throw synchronously before returning a
request. Worker I/O (`Io`), oversized assets (`TooLarge`), invalid UTF-8
(`InvalidUtf8`) and worker path failures (`InvalidPath`) reject `request.promise`
with an `Error` carrying a string `code`.

`loadTextAsset(path)` wraps `readTextAsset(path)` for reactive computations
(such as Solid 2.0 async memos under `<Loading>`/`<Errored>`): when called inside
a reactive owner, it cancels the in-flight request on `onCleanup` and resolves to
`""` if cancelled with code `Cancelled` so owner cancellation never triggers an
unhandled rejection under strict session policies; host I/O errors still reject.

`HostRequest<T>` has a readonly `promise: Promise<T>` and synchronous
`cancel(): void`. Cancellation rejects a pending promise with an `Error` whose
`code` is `Cancelled`; handle rejection even when cancelling during cleanup.
Cancellation is best effort in the worker and definitive at owner-thread
completion delivery: a cancelled result cannot later resolve the request.
Cancellation after settlement is a no-op, and repeated cancellation is safe.

`HostServiceError` describes a native failure as an `Error` with a readonly
string `code`. Codes are host-defined rather than a closed enumeration.

`registerImageAsset(path)` registers the integrity-verified bytes of a
manifest-declared package asset and returns an `ImageHandle`. Assign its opaque
`src` to an `Image`; the bytes stay outside the JavaScript heap. Call the
handle's synchronous, idempotent `release()` when it is no longer in use.
`loadImageAsset(path)` returns the same handle and releases it on cleanup when
called inside a Solid owner. Both helpers throw synchronously for undeclared
assets and are available during mount, lifecycle callbacks and events.

`registerImageBytes(bytes: Uint8Array)` copies the supplied view into the
session and returns an `ImageHandle` with a `zgui-blob://` `src`. Use it for
encoded PNG or SVG received as bytes, for example from an NDJSON message.
No asset declaration or preview fixture mapping is needed. Registration throws
synchronously for a wrong view type, a detached view, an empty view, or a view
larger than 8 MiB. Each session's active direct-image registry is capped at
64 registered handles and 64 MiB of registered bytes. Hitting either limit
throws.

`release()` frees registry capacity immediately; in-flight decode jobs can
finish using their own clone of the buffer reference. Use `loadImageBytes(bytes)`
to release on Solid owner cleanup. Outside an owner, both registration helpers
require explicit release. Preview reports unreleased handles at teardown.

## External communication

Declare only the authorities the application needs in the package's
`capabilities`. Omitting these declarations grants no socket, command, or
control-channel access. These services run during the application lifecycle, not
during package preflight. Callbacks run in the owning session's bridge turns.

### Portable paths and endpoint queries

Declare paths and endpoint names in `package.json` under `zgui.capabilities`:

```json
{
  "imagePrefixes": ["$XDG_RUNTIME_DIR/z-shell/media"],
  "unixSockets": ["$XDG_RUNTIME_DIR/z-shell/host-${WAYLAND_DISPLAY}.sock"],
  "namedSockets": ["niri", "status"]
}
```

Absolute literal paths still work. A template starts with `$XDG_RUNTIME_DIR`,
`$HOME`, or `$XDG_DATA_HOME`, followed by optional canonical path segments.
`${WAYLAND_DISPLAY}` can be embedded in a segment; `$WAYLAND_DISPLAY` must
occupy a whole segment. The display value must be one nonempty filename
component, not `.` or `..`. No other variables or shell syntax are expanded.
The builder seals templates verbatim; each session resolves them before
evaluating the module. A missing or invalid required value rejects the session.

Live `$XDG_RUNTIME_DIR` must be an existing, non-symlink directory owned by
the effective user with mode `0700`; there is no `/tmp` fallback. Explicit
`$HOME` and `$XDG_DATA_HOME` roots must be existing canonical directories.
When `XDG_DATA_HOME` is unset, it defaults to `$HOME/.local/share` under a
valid home, even if the default directory does not exist yet.

Use repeatable `zgui-host --bind-socket name=/canonical/path` options to
bind named endpoints. Each name can be supplied once. An explicit binding
overrides the allowlisted environment binding (`niri` → `NIRI_SOCKET`);
no other environment names are inferred. Only manifest-declared names can
grant access. Bindings are literal canonical absolute paths, not templates.
Live socket paths may be absent at startup, but existing targets and ancestors
must not contain symlinks.

`getResolvedCapabilities()` returns fresh copies of the resolved
`imagePrefixes`, `unixSockets`, and `namedSockets`. The named map has no
prototype: undeclared keys read as `undefined`. `getAuthorizedEndpoint(name)`
returns the path or `null` if the declared name is unbound; undeclared names
throw a `HostServiceError` with code `E_CAPABILITY`. Both queries are safe at
module evaluation and never expose other environment values. Mutating a
returned copy cannot change authority.

### Unix domain sockets

`connectUnixSocket(path, options)` connects asynchronously to an exact path
from resolved `unixSockets` or a bound `namedSockets` value. It returns a socket
with `send(text)` and `close()`. Set `options.framing` to match the peer:

- `"line"`: one UTF-8 message per LF (`0x0a`). `send` adds the LF; the text
  passed to it must not contain LF.
- `"length"`: a four-byte unsigned big-endian count followed by one kind byte
  and the data. The count includes the kind byte. Kind `0x00` is UTF-8 text
  delivered to `onMessage(text)`; `send` always uses this kind. Kind `0x01` is
  encoded image data, even if it is also valid UTF-8 (such as SVG). Image
  frames are ignored unless `onBinaryImage(handle)` is configured. Use an
  encoding supported by the native image loader and release each
  `ImageHandle` when no longer needed. A zero count or unknown kind closes the
  socket with `InvalidMessage`. Invalid UTF-8 text closes it with `InvalidUtf8`,
  never an image callback.

Each frame can carry up to 8 MiB of data, excluding the LF delimiter or the
length header and kind byte. `send` queues text without waiting for socket I/O
and throws `Busy` if its queue is full. Delivered socket images share a
64-handle, 64 MiB budget per session; exceeding it closes the receiving socket.
The optional `onClose(error)` receives `null` on peer EOF or a `HostServiceError`
on asynchronous failure. Explicit `close()` stops delivery without calling
`onClose` or waiting for the worker to close the connection. A Solid owner
closes the socket on cleanup; otherwise close it explicitly.

### Commands

`spawnCommand(program, args)` starts an executable whose exact name or path
appears in `capabilities.commands`. Each string in `args` is one argument;
there is no shell interpolation or argument splitting.
Bare names resolve through absolute directories in the host's `PATH`. The host
accepts up to 64 arguments and 64 KiB of argument text. Allowlisting an executable
authorizes it to run with those arguments and with the host's environment.

- By default, or with `{ stream: false }`, it returns a `HostRequest` whose
  promise resolves to `{ status, stdout, stderr }`. A nonzero exit code is
  returned as `status`, not thrown. A process that does not exit normally has
  `status: null`. Host failures reject the promise. The two UTF-8 output streams
  have a combined limit of 1 MiB.
- With `{ stream: true }`, it returns a handle with `cancel()` and its alias
  `kill()`. Supply `onStdout(line)`, `onStderr(line)`, `onExit(status)` and
  `onError(error)` as needed. Each UTF-8 line is at most 64 KiB, excluding LF.
  `onExit` receives the same kind of status as buffered mode.

A Solid owner cancels the command on cleanup. In buffered mode the SDK observes
the cleanup rejection but returns the original request: awaiting its promise
after cancellation still rejects with `Cancelled`. Handle ordinary failures,
and cancellation you request explicitly, on that promise. Without an owner,
cancel commands you no longer need. Cancellation and session retirement stop
delivery without waiting for process teardown. The worker terminates a
still-running command's process group and reaps its leader. Ordinary
completion reaps the leader without killing surviving group members or waiting
for them to close inherited output pipes. This does not sandbox commands that
deliberately leave their process group.

### Control channel

With `capabilities.controlChannel: true`, `onControlMessage(handler)` registers
one inbound handler per session. It takes a readonly array of argument strings and
returns synchronously. A string is sent as-is, another JSON value is encoded as
JSON, and `undefined` produces a successful empty reply. The return value of
`onControlMessage()` is an unsubscribe function; a Solid owner calls it
automatically on cleanup.

The default control socket is `$XDG_RUNTIME_DIR/zgui-js/<package-id>.sock`.
Start the application with `--control-socket /canonical/path` to override it.
Send a message to a running application with one of:

```sh
zgui-host msg --app-id example -- status "argument with spaces"
zgui-host msg --control-socket /canonical/path -- status "argument with spaces"
```

Use `--` to end the client options and preserve all subsequent argument
strings as payload. Requests accept at most 64 strings and 64 KiB of wire JSON.
Replies are limited to 64 KiB of UTF-8 text before framing and are written to
standard output. Reloading replaces the listener for the old session. Requests
queued on the old connection are rejected rather than delivered to the
successor; a client may see a disconnected socket during cutover.

In deterministic previews these services use scripts from the
[fixture schema](../../schemas/preview/fixture.schema.json) instead of real
sockets, processes, or a CLI listener. Fixtures do not grant capabilities.
For portable capabilities, set the fixture's `runtimeEnvironment` with
`xdgRuntimeDir`, `homeDir`, `xdgDataHome`, `waylandDisplay`, and/or
`namedSockets` (a name-to-literal-path object). Paths can be synthetic: preview
never uses live environment or host file access to resolve them. Socket scripts
and image mappings use the resolved literal paths. An undeclared endpoint
query records a sticky `E_CAPABILITY` failure even when the exception is caught.

### Application activation

Read `getInitialActivation()` during mount, an effect, or an event handler. It
returns `{ args, cwd }`: the UTF-8 arguments after `zgui-host --` and the
canonical launch directory (or `null` when unavailable). The initial value
does not change on reload. Calls during module evaluation throw.

Set `zgui.capabilities.singleInstance: true` in the base `package.json` to
route subsequent launches to the running app. Register `onActivation(info => …)`
to handle those invocations. It returns an idempotent unsubscribe, automatically
registered as cleanup under a Solid owner. Pending invocations survive generation
replacement. The secondary's success means the primary queued the invocation,
not that a handler ran or the compositor granted focus.

In live runs, the instance uses `$XDG_RUNTIME_DIR/zgui-js/<appId>.instance.sock`
(`0600` in a `0700` directory), independent of the control socket. Changing the
single-instance setting or its package ID in dev requires a restart. The host
removes `XDG_ACTIVATION_TOKEN` before creating workers or launching children; tokens
are never part of `ActivationInfo`.

`zgui-js dev [DIR] -- [args...]` forwards arguments from the caller's directory.
A packaged launcher accepts application arguments directly. Its standalone
`--help`, `--version` (optionally `--json`), and `--exit-after-ms N` forms are
reserved for host diagnostics; prefix them with `--` to pass them literally.
Both launchers accept at most 64 arguments totaling 64 KiB of UTF-8.

Preview uses the fixture's `activation` and scenario `service.emit` channel
`activation` without reading ambient launch state or opening sockets.

## Persistent filesystem grants

`listGrants()` returns this application's `GrantInfo` records (`token`, `kind`,
`label`, `mode`, `createdMs`), never the underlying pathname. `GrantToken` is
opaque; `GrantKind` is `workspace-file`, `workspace-dir` or `system-dir`,
and `GrantAccessMode` is `read` or `readwrite`. `revokeGrant(token)` returns
whether the entry existed. It prevents future restoration without closing
existing live handles or touching the underlying filesystem resource.

`persistHandle`, `restoreBookmark`, `listBookmarks` and `revokeBookmark` use the
same ledger. The bookmark inventory and revocation API remain workspace-only and
continue to use `file` / `dir`. The host upgrades existing bookmarks without
changing their tokens. `restoreBookmark` never restores a `system-dir` grant.

Set `capabilities.files.allowBookmarks: false` to disable workspace grant
persistence and restoration, or `capabilities.files.readOnlyGrants: true` to
deny only `readwrite` grants. A denied restore rejects with `E_CAPABILITY` and
leaves the record and target unchanged; those grants can still be listed and
revoked. Omission preserves legacy behavior.

## System files

Declare only the axes the application uses. Runtime extension overlays cannot
add system-file flags or expand roots beyond the base manifest.

```json
{
  "systemFiles": {
    "read": true,
    "write": true,
    "trash": true,
    "volumes": true,
    "thumbnails": true,
    "openExternal": true,
    "directories": [{ "path": "$HOME", "mode": "readwrite" }]
  }
}
```

- `openSystemDirectory` opens an exact declaration (literal path or `$HOME`,
  `$XDG_DATA_HOME`, `$XDG_DOCUMENTS_DIR` template). `openSystemDirectoryDialog`
  requires a gesture and grants read-only access unless write is declared and
  `readOnly` is not set. `openDroppedSystemDirectory(ev.directories[i])` redeems
  an accepted drop as a read-only system root. It does not convert arbitrary
  workspace handles. A Solid owner closes the system handle on cleanup.
- `lstat` and `readDir` expose symlink metadata without following the final link
  for `lstat`. Traversal requires an active root covering the resolved target;
  mutations require write authority at both ends. Use an entry's returned
  `relativePath` verbatim. Pages default to 2,000 entries (max 5,000); cursors
  continue the same sort and filter on the current listing, not a snapshot.
- `persistSystemGrant` and `restoreSystemGrant` use the same per-app ledger as
  workspace grants, without promoting either kind. `restoreSystemGrant` rejects
  a stored readwrite grant if the current manifest no longer declares write.
- `startFileJob` copies or moves each source into the destination directory and
  never reads file contents into JavaScript. It also supports Trash, permanent
  removal, mkdir, symlink creation, and chmod. A job pauses on conflict
  until `onConflict` returns `overwrite`, `skip`, `autoRename`, or `cancel`;
  no handler means cancel. Cancelling stops further work without rolling back
  entries already committed. Trash never falls back to permanent removal.
  `listTrash` shows entries within active roots; restoration needs a writable root.
- `listVolumes` inventories Linux mounts, and `watchVolumes` delivers an initial
  inventory and later changes until stopped or the Solid owner is disposed.
- `loadCachedThumbnail` returns a native `ImageHandle` for a fresh Freedesktop
  cache hit and `null` for a miss. Release the returned handle when no longer used.
- `openExternal` accepts a granted entry or an HTTP(S)/mailto URL during a user
  gesture or with a single-use token minted by this session via
  `requestActivationToken()`. Preview records the target in
  `services.json` instead of launching a process.
- `startOutboundSystemDrag` offers granted paths as `text/uri-list` from a live
  press, without buffering file contents. Move requires write authority. The
  host never deletes source data after a drag.

The separate `services.systemFiles` preview fixture supplies a virtual desktop
filesystem, generated entries, scripted directory dialogs, volumes, Trash
availability, and cached PNGs. It cannot grant authority or access host paths.

## Reactive persistence and query adapters

Keep Solid 2.0 as the sole client state model (`createSignal` → `createContext` → `createStore` / `createProjection` / `reconcile`). `@hewel/zgui-services` provides two narrow owner-scoped adapters where Solid meets Native Host I/O:

### `createKvState(kv, key, initial, options)`

Binds one versioned key of a borrowed `KvStateStorage` (`KvStore<JsonValue>` satisfies this directly) to Solid signals without opening or closing the database.

- Requires an active Solid owner, nonempty `key`, safe-integer `version` (default `1`), integer `debounceMs` in `0..0xffffffff` (default `180`), and synchronous `options.validate(raw)`.
- Publishes the validated frozen JSON `initial` snapshot synchronously for non-blocking first paint, then launches one `kv.get(key)`. Persists `{ v: version, data: value }` in one `kv.set`.
- Early `set` / `reset` calls before hydration finishes win as a whole record; functional updaters receive the last accepted local snapshot so same-turn updates compose cleanly.
- Serializes at most one in-flight `kv.set` with trailing debounce and coalesces newer edits into the latest pending snapshot. Write failures retain the dirty snapshot, surface `error()`, and stop automatic retries until a later edit or `flush()`.
- **`KvState.flush()` vs Solid `flush()`**: `KvState.flush(): Promise<void>` is an **asynchronous disk durability barrier** that bypasses debounce, waits for hydration and any preceding write, and resolves once the caller's revision (or a newer coalesced revision) persists (or rejects on write failure / disposal). It is unrelated to `@hewel/zgui`'s synchronous Solid render `flush()`.

### `createHostQuery(source, fetcher, options?)`

Non-suspending latest-wins adapter for replaceable cancellable `HostRequest<T>` reads (such as SQLite FTS queries or text asset loads — not mutations or an HTTP cache).

- Requires an active Solid owner; tracks only `source` and explicit `refetch()` calls. `0` and `""` are enabled; `null`, `undefined`, and `false` disable, cancel any pending request, and reset `data()` to `initialValue` regardless of `keepPreviousData`.
- `fetcher(input)` must synchronously return a `HostRequest<T>` (bare `Promise`s are rejected).
- On source change, `refetch()`, disabling, or owner cleanup, invalidates the previous generation before calling `old.cancel()`, coalesces same-turn source changes + `refetch()`, preserves previous `data()` by default (`keepPreviousData: true`), discards superseded settlements without clearing active `loading()`, and suppresses only its own generation cancellations (never blindly swallowing an external `Cancelled` error on the current request).

## Runtime boundaries

This package provides generic resource and communication services, not
window-manager integrations. Platform and compositor support, including
layer-shell capabilities, comes from upstream zgui.

The UI SDK declares `setTimeout`, `clearTimeout`, `requestAnimationFrame`,
`cancelAnimationFrame` and `queueMicrotask` as existing host globals. These are
native registrations, not browser APIs. Timer delays are minimum delays, not
exact deadlines; native work can delay execution. Frame callbacks run before
native layout and do not promise that presentation is complete. Register them
inside the UI lifecycle and explicitly cancel pending timers, frames and service
requests in `onCleanup`. Neither `window`, `document`, `fetch`, `process` nor
`setInterval` is part of this environment.
