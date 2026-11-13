import { readTextAsset } from "@zgui/services";
import type { HostRequest } from "@zgui/services";

const request: HostRequest<string> = readTextAsset("assets/message.txt");
const promise: Promise<string> = request.promise;
const cancelled: void = request.cancel();
const length: Promise<number> = request.promise.then(text => text.length);
const recovered: Promise<string> = request.promise.catch((reason: unknown) => {
  if (reason instanceof Error && "code" in reason && reason.code === "Cancelled") return "cancelled";
  throw reason;
});
const generic: HostRequest<{ value: number }> = {
  promise: Promise.resolve({ value: 1 }),
  cancel() {},
};

// @ts-expect-error The promise cannot be replaced by a caller.
request.promise = Promise.resolve("replacement");
// @ts-expect-error A request is not itself a Promise; callers await request.promise.
const implicitPromise: Promise<string> = request;
// @ts-expect-error Cancellation is synchronous and does not return a Promise.
const asyncCancellation: Promise<void> = request.cancel();
// @ts-expect-error The operation returns text, not a Node Buffer or byte array.
const bytes: Promise<Uint8Array> = request.promise;
// @ts-expect-error Only a string package path is supported.
readTextAsset({ path: "assets/message.txt" });
// @ts-expect-error Browser globals are not added by the services SDK.
window.fetch("https://example.test");
// @ts-expect-error Browser types are not added by the services SDK.
let element: HTMLElement;
// @ts-expect-error Node globals are not added by the services SDK.
process.cwd();
// @ts-expect-error The private host binding is not a public ambient global.
globalThis.__zguiServices.readTextAsset("assets/message.txt");

void [promise, cancelled, length, recovered, generic, implicitPromise, asyncCancellation, bytes, element];
