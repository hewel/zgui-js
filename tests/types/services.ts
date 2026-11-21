import {
  clipboardReadImage,
  clipboardReadText,
  connectUnixSocket,
  getAuthorizedEndpoint,
  getResolvedCapabilities,
  loadImageBytes,
  loadTextAsset,
  onControlMessage,
  readClipboardImage,
  readClipboardText,
  readTextAsset,
  registerImageBytes,
  spawnCommand,
  writeClipboardImage,
  writeClipboardText,
} from "@hewel/zgui-services";
import type {
  ClipboardWriteOptions,
  CommandResult,
  HostRequest,
  HostServiceError,
  ImageHandle,
  ResolvedCapabilities,
} from "@hewel/zgui-services";

const request: HostRequest<string> = readTextAsset("assets/message.txt");
const loadedText: Promise<string> = loadTextAsset("assets/message.txt");
void loadedText;
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

const serviceError: HostServiceError = Object.assign(new Error("resource failure"), { code: "Io" });
const errorCode: string = serviceError.code;
const nativeError: Error = serviceError;
// @ts-expect-error Native service failure codes cannot be replaced by callers.
serviceError.code = "replacement";

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

const directImage: ImageHandle = registerImageBytes(new Uint8Array([1, 2]));
const scopedImage: ImageHandle = loadImageBytes(new Uint8Array([3, 4]));
const directSrc: string = directImage.src;
void directSrc;
directImage.release();
scopedImage.release();
// @ts-expect-error Use a Uint8Array view, not its backing buffer.
registerImageBytes(new ArrayBuffer(2));
// @ts-expect-error Byte arrays must be typed arrays.
loadImageBytes([1, 2]);

const resolved: ResolvedCapabilities = getResolvedCapabilities();
const optionalEndpoint: string | null = getAuthorizedEndpoint("niri");
if (optionalEndpoint !== null) connectUnixSocket(optionalEndpoint, { framing: "line", onMessage() {} });
// @ts-expect-error The namespace is keyed by a string name.
getAuthorizedEndpoint(12);
// @ts-expect-error A named endpoint can be unbound.
const requiredEndpoint: string = getAuthorizedEndpoint("niri");
// @ts-expect-error Image prefixes are readonly.
resolved.imagePrefixes.push("/tmp");
// @ts-expect-error Socket paths are readonly.
resolved.unixSockets[0] = "/tmp/socket";
// @ts-expect-error Named sockets are readonly.
resolved.namedSockets.niri = null;
const missingEntries: typeof resolved.namedSockets = { missing: undefined };
void missingEntries;
void requiredEndpoint;

const socket = connectUnixSocket("/run/example.sock", {
  framing: "length",
  onMessage(message) {
    const text: string = message;
    void text;
  },
  onBinaryImage(handle) {
    const image: ImageHandle = handle;
    image.release();
  },
  onClose(error) {
    const failure: HostServiceError | null = error;
    void failure;
  },
});
socket.send("refresh");
socket.close();
// @ts-expect-error The socket sends text, not binary payloads.
socket.send(new Uint8Array());
// @ts-expect-error The wire framing must be explicitly supported.
connectUnixSocket("/run/example.sock", { framing: "json", onMessage() {} });

const args = ["argument with spaces"] as const;
const command: HostRequest<CommandResult> = spawnCommand("/usr/bin/echo", args);
const buffered: Promise<CommandResult> = spawnCommand("/usr/bin/echo", args, { stream: false }).promise;
const output: Promise<{ status: number | null; stdout: string; stderr: string }> = command.promise;
const running = spawnCommand("/usr/bin/echo", args, {
  stream: true,
  onStdout(line) {
    const text: string = line;
    void text;
  },
  onStderr(line) {
    const text: string = line;
    void text;
  },
  onExit(status) {
    const code: number | null = status;
    void code;
  },
  onError(error) {
    const failure: HostServiceError = error;
    void failure;
  },
});
running.cancel();
running.kill();
// @ts-expect-error Streaming commands do not expose a promise.
void running.promise;
// @ts-expect-error Streaming callbacks require stream: true.
spawnCommand("/usr/bin/echo", args, { onStdout() {} });

const unsubscribe = onControlMessage(args => {
  // @ts-expect-error Control message arguments are readonly.
  args.push("extra");
  if (args.length === 0) return undefined;
  if (args[0] === "text") return "unchanged";
  if (args[0] === "caps") return { ok: true, caps: getResolvedCapabilities() };
  if (args[0] === "branch") return { ok: true, count: 1 };
  return { args, status: ["ok", null, 0, true] };
});
unsubscribe();
// @ts-expect-error Control replies are synchronous.
onControlMessage(async () => "reply");
// @ts-expect-error Objects containing functions are not JSON values.
onControlMessage(() => ({ callback() {} }));

void [command, buffered, output];

const clipTextReq: HostRequest<string | null> = readClipboardText();
const clipTextScoped: Promise<string | null> = clipboardReadText();
const clipImgReq: HostRequest<ImageHandle | null> = readClipboardImage();
const clipImgScoped: Promise<ImageHandle | null> = clipboardReadImage();
const clipWriteOpts: ClipboardWriteOptions = { secret: true };
const clipWriteTextReq: HostRequest<void> = writeClipboardText("hello", clipWriteOpts);
const clipWriteImgBytes: HostRequest<void> = writeClipboardImage(new Uint8Array([1, 2]), { secret: false });
const clipWriteImgHandle: HostRequest<void> = writeClipboardImage(directImage, clipWriteOpts);
// @ts-expect-error Clipboard write text requires a string.
writeClipboardText(123);
// @ts-expect-error Clipboard write image accepts Uint8Array or ImageHandle, not raw paths.
writeClipboardImage("/etc/passwd");
// @ts-expect-error secret option must be a boolean.
writeClipboardText("x", { secret: "yes" });

void [clipTextReq, clipTextScoped, clipImgReq, clipImgScoped, clipWriteTextReq, clipWriteImgBytes, clipWriteImgHandle];
void [promise, cancelled, length, recovered, generic, implicitPromise, asyncCancellation, bytes, element, errorCode, nativeError];
