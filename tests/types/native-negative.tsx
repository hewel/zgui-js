import { renderApp } from "@zgui/ui";
import type { NativeNode } from "@zgui/ui";

// @ts-expect-error Browser document is not a host global.
document.createElement("div");
// @ts-expect-error Browser window is not a host global.
window.requestAnimationFrame(() => {});
// @ts-expect-error No browser element types leak through SDK declarations.
let browserElement: HTMLElement;
// @ts-expect-error Browser Event is not a native event.
let browserEvent: Event;
// @ts-expect-error Browser events do not leak through control-flow declarations.
let pointerEvent: PointerEvent;
// @ts-expect-error Node process is not a host global.
process.exit(0);
// @ts-expect-error Fetch is not implemented by this host.
fetch("https://example.test");
// @ts-expect-error No browser scheduler facilities are exposed.
new MessageChannel();
// @ts-expect-error Only implemented timer APIs are declared.
setInterval(() => {}, 1);
// @ts-expect-error Unknown tags must not be swallowed by an index signature.
const html = <div />;
// @ts-expect-error Native editing has not passed its own integration gate.
const editing = <field />;
// @ts-expect-error Image assets have not passed their integration gate.
const image = <image />;
// @ts-expect-error Arbitrary native properties are not accepted.
const property = <text arbitrary="unsupported" />;
// @ts-expect-error HTML spellings are not aliases for native properties.
const className = <box className="browser" />;
// @ts-expect-error Browser event names are not native aliases.
const handler = <control onDoubleClick={() => {}} />;
// @ts-expect-error Ref callbacks expose a NativeNode, not browser methods.
const ref = <text ref={node => node.focus()} />;
// @ts-expect-error Native styles do not coerce numbers or use camelCase.
const numericStyle = <box style={{ "font-size": 20 }} />;
// @ts-expect-error Browser-style property names are unsupported.
const camelStyle = <box style={{ backgroundColor: "red" }} />;
// @ts-expect-error Native cancellation has no listener-options tuple form.
const listenerOptions = <control onClick={[() => {}, { passive: true }]} />;
// @ts-expect-error Native node IDs are not numbers.
const id: NativeNode = 1;
// @ts-expect-error Fabricated objects are not native handles.
const fabricated: NativeNode = {};
// @ts-expect-error Child object literals are not renderable.
renderApp(() => ({ arbitrary: "object" }), fabricated);
// @ts-expect-error Only the curated reactive API is public.
import { createDeferred } from "@zgui/ui";
// @ts-expect-error Upstream DOM components are not exported.
import type { HTMLAttributes } from "@zgui/ui";
// @ts-expect-error Reactive roots are owned by renderApp, not the application API.
import { createRoot } from "@zgui/ui";
void [browserElement, browserEvent, pointerEvent, html, editing, image, property, className, handler, ref, id, createDeferred];
