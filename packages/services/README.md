# Native services

`@zgui/services` is separate from `@zgui/ui`. Its implementation and public types
share strict TypeScript source using ES2022 only; no browser or Node globals are
imported. This private workspace package exports that source directly, and the
application builder erases types before QuickJS execution. Importing the module
does not inspect the host. Calling a service forwards to the private native
binding in the active session; start work from mount, lifecycle callbacks or
events, not module evaluation/preflight or disposal. Cleanup may cancel existing
work.

```ts
import { readTextAsset } from "@zgui/services";

const request = readTextAsset("assets/message.txt");
request.promise.then(text => console.log(text), error => console.error(error));
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

`HostRequest<T>` has a readonly `promise: Promise<T>` and synchronous
`cancel(): void`. Cancellation rejects a pending promise with an `Error` whose
`code` is `Cancelled`; handle rejection even when cancelling during cleanup.
Cancellation is best effort in the worker and definitive at owner-thread
completion delivery: a cancelled result cannot later resolve the request.
Cancellation after settlement is a no-op, and repeated cancellation is safe.

## niri snapshots

```tsx
import { createSignal, onCleanup, onMount } from "@zgui/ui";
import { watchNiri } from "@zgui/services";
import type { HostServiceError, HostSubscription, NiriSnapshot } from "@zgui/services";

// Inside a mounted component:
const [snapshot, setSnapshot] = createSignal<NiriSnapshot>();
const [error, setError] = createSignal<HostServiceError>();
let subscription: HostSubscription | undefined;
onCleanup(() => subscription?.cancel());
onMount(() => {
  subscription = watchNiri(setSnapshot, setError);
});
```

`watchNiri(onSnapshot, onError)` subscribes to the compositor over the host's
`NIRI_SOCKET`. Both callbacks are required; starting the subscription can throw
synchronously for invalid arguments, lifecycle violations or the per-session
limit of four subscriptions. IPC failures are delivered as `HostServiceError`,
an `Error` with a readonly string `code`. The error callback is terminal: there
are no later snapshots and no silent reconnect. Missing IPC is an error, not an
empty or simulated desktop.

`HostSubscription` exposes only synchronous, idempotent `cancel(): void`.
Cancellation releases the subscription and prevents either callback from running
again. Cancel from `onCleanup`; import alone neither reads host globals nor
opens a connection.

The first snapshot arrives asynchronously after both initial workspace and
window state are available. Later snapshots can coalesce intermediate compositor
events; this is current state, not an event log. No polling or UI timer is needed
for idle compositor updates.

`NiriSnapshot` has readonly `windows` and `workspaces` arrays. Every DTO field is
readonly in TypeScript; treat snapshots as input state, not mutable UI models.
All window/workspace IDs, including `workspaceId` and `activeWindowId`, are
lossless decimal strings (or `null` for an absent relationship), never JS numbers.
Nullable titles, names, outputs, app IDs and PIDs remain `null` when unknown.
Windows expose focus, floating and urgency flags; workspaces expose index,
output, focus, activation, urgency and the active window ID.

Key native lists by those string IDs, then look up reactive data by ID. Passing
the newly allocated DTO objects directly to identity-keyed `For` recreates rows
on every snapshot. `apps/examples/panel` demonstrates retained rows, visible
waiting/error/live states and lifecycle-owned cancellation.

The UI SDK declares `setTimeout`, `clearTimeout`, `requestAnimationFrame`,
`cancelAnimationFrame` and `queueMicrotask` as existing host globals. These are
native registrations, not browser APIs. Timer delays are minimum delays, not
exact deadlines; native work can delay execution. Frame callbacks run before
native layout and do not promise that presentation is complete. Register them
inside the UI lifecycle and explicitly cancel pending timers, frames and service
requests in `onCleanup`. Neither `window`, `document`, `fetch`, `process` nor
`setInterval` is part of this environment.
