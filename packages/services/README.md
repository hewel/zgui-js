# Native services

`@hewel/zgui-services` is separate from `@hewel/zgui`. Its implementation and public types
share strict TypeScript source using ES2022 only; no browser or Node globals are
imported. This private workspace package exports that source directly, and the
application builder erases types before QuickJS execution. Importing the module
does not inspect the host. Calling a service forwards to the private native
binding in the active session; start work from mount, lifecycle callbacks or
events, not module evaluation/preflight or disposal. Cleanup may cancel existing
work.

```ts
import { readTextAsset } from "@hewel/zgui-services";

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

`HostServiceError` describes a native failure as an `Error` with a readonly
string `code`. Codes are host-defined rather than a closed enumeration.

This package provides resource services, not window-manager integrations.
Platform and compositor support, including layer-shell capabilities, comes from
upstream zgui; there is no compositor-specific service package or abstraction.

The UI SDK declares `setTimeout`, `clearTimeout`, `requestAnimationFrame`,
`cancelAnimationFrame` and `queueMicrotask` as existing host globals. These are
native registrations, not browser APIs. Timer delays are minimum delays, not
exact deadlines; native work can delay execution. Frame callbacks run before
native layout and do not promise that presentation is complete. Register them
inside the UI lifecycle and explicitly cancel pending timers, frames and service
requests in `onCleanup`. Neither `window`, `document`, `fetch`, `process` nor
`setInterval` is part of this environment.
