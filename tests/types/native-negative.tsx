import { Image, TextField, TextArea, renderApp } from "@zgui/ui";
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
// @ts-expect-error Native text fields own their descendants.
const editing = <field>Authored child</field>;
// @ts-expect-error Native editors also reject explicit children props.
const editorChildren = <editor children={<text>Child</text>} />;
// @ts-expect-error TextField cannot reconcile authored children.
const fieldChildren = <TextField><text>Child</text></TextField>;
// @ts-expect-error TextArea cannot reconcile authored children.
const areaChildren = <TextArea>Child</TextArea>;
// @ts-expect-error Child spreads cannot bypass the native editor boundary.
const childSpread = <TextField {...{ children: "Child" }} />;
// @ts-expect-error The component owns its native textInput role.
const fieldRole = <TextField role="button" />;
// @ts-expect-error The component owns its native multilineTextInput role.
const areaRole = <TextArea role="textInput" />;
// @ts-expect-error Native controls accept strings, not DOM value coercion.
const numericValue = <TextField value={42} />;
// @ts-expect-error Value is an editing-control prop, not a generic element property.
const genericValue = <box value="text" />;
// @ts-expect-error Native controls do not expose HTML input properties.
const htmlField = <TextField type="password" />;
// @ts-expect-error Native controls do not expose DOM selection setters.
const selectionSetter = <TextArea selectionStart={0} />;
// @ts-expect-error Event selection is an owned read-only snapshot.
const mutableSelection = <TextField onInput={event => { event.selection.start = 1; }} />;
// @ts-expect-error Native image sources are strings, never browser source objects.
const image = <image src={{ src: "assets/icon.png" }} />;
// @ts-expect-error src belongs only to native image elements.
const boxSource = <box src="assets/icon.png" />;
// @ts-expect-error Native replaced images cannot reconcile children.
const imageChildren = <Image src="assets/icon.png">Child</Image>;
// @ts-expect-error Browser responsive image selection is not a supported contract.
const imageSet = <image srcSet="assets/icon.png 2x" />;
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
