import { Image, TextField, TextArea, createCanvasScene, createSpring, renderApp } from "@hewel/zgui";
import type { NativeElementProps, NativeNode } from "@hewel/zgui";

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
// @ts-expect-error Native vectors are childless replaced drawings.
const vectorChildren = <vector><text>Child</text></vector>;
// @ts-expect-error Native vectors also reject explicit children props.
const vectorPropChildren = <vector children={<text>Child</text>} />;
// @ts-expect-error Path data belongs only to native vector elements.
const boxPath = <box d="M0 0" />;
// @ts-expect-error The drawing coordinate box belongs only to native vector elements.
const boxViewBox = <control viewBox="0 0 16 16" />;
// @ts-expect-error SVG documents belong only to native vector elements.
const boxSvg = <box svg="<svg/>" />;
const canvasScene = createCanvasScene();
// @ts-expect-error Canvas scenes belong only to native vector elements.
const boxCanvas = <box canvas={canvasScene} />;
// @ts-expect-error A numeric token cannot bind a canvas scene.
const rawCanvas = <vector canvas={canvasScene.token} />;
// @ts-expect-error Canvas scenes are opaque native handles.
const fabricatedCanvas = <vector canvas={{ token: 1, update() {}, dispose() {} }} />;
// @ts-expect-error Canvas scene tokens cannot be reassigned.
canvasScene.token = 1;
// @ts-expect-error An affine transform has exactly six numbers.
canvasScene.update(null, [1, 0, 0, 1, 0]);
// @ts-expect-error checked is a boolean, "mixed", or empty.
const checkedString = <control checked="yes" />;
// @ts-expect-error Only implemented accessibility roles are accepted.
const roleString = <box role="paragraph" />;
// @ts-expect-error Role spellings use the native vocabulary exactly.
const roleCase = <box role="checkbox" />;
// @ts-expect-error Orientation is horizontal or vertical.
const orientationString = <control orientation="diagonal" />;
// @ts-expect-error Data attributes carry strings, not booleans.
const dataBoolean: NativeElementProps = { "data-count": true };
// @ts-expect-error Data attributes carry strings, not numbers.
const dataNumber: NativeElementProps = { "data-count": 3 };
// @ts-expect-error Arbitrary native properties are not accepted.
const property = <text arbitrary="unsupported" />;
// @ts-expect-error The StyleX className alias carries strings, not numbers.
const className = <box className={3} />;
// @ts-expect-error Browser event names are not native aliases.
const handler = <control onDoubleClick={() => {}} />;
// @ts-expect-error Ref callbacks expose a NativeNode, not browser methods.
const ref = <text ref={node => node.focus()} />;
// @ts-expect-error Non-unitless native styles require strings with explicit units.
const numericStyle = <box style={{ "font-size": 20 }} />;
// @ts-expect-error Numeric translate is not accepted, including unitless zero.
const numericTranslate = <box style={{ translate: 0 }} />;
// @ts-expect-error Numeric rotate is not accepted, including unitless zero.
const numericRotate = <box style={{ rotate: 0 }} />;
// @ts-expect-error Wheel exposes no pointer-capture authority.
const wheelCapture = <box onWheel={event => event.capturePointer()} />;
// @ts-expect-error Wheel delta is a read-only snapshot.
const wheelDelta = <box onWheelCapture={event => { event.delta.x = 0; }} />;
// @ts-expect-error Font variations require a CSS string.
const numericVariation = <box style={{ fontVariationSettings: 650 }} />;
// @ts-expect-error The spring targets a number, not a CSS length string.
createSpring(() => "20px");
// @ts-expect-error Spring options are numeric.
createSpring(() => 0, { damping: "26" });
// @ts-expect-error Unknown camelCase property names remain unsupported.
const camelStyle = <box style={{ backgroundColour: "red" }} />;
// @ts-expect-error Native cancellation has no listener-options tuple form.
const listenerOptions = <control onClick={[() => {}, { passive: true }]} />;
// @ts-expect-error Native node IDs are not numbers.
const id: NativeNode = 1;
// @ts-expect-error Fabricated objects are not native handles.
const fabricated: NativeNode = {};
// @ts-expect-error Child object literals are not renderable.
renderApp(() => ({ arbitrary: "object" }), fabricated);
// @ts-expect-error Only the curated reactive API is public.
import { createDeferred } from "@hewel/zgui";
// @ts-expect-error Upstream DOM components are not exported.
import type { HTMLAttributes } from "@hewel/zgui";
// @ts-expect-error Context menu requests are non-capturing pointer snapshots without releasePointer.
const contextRelease = <box onContextMenu={event => event.releasePointer()} />;
// @ts-expect-error Context menu requests are not pointerdown and cannot capturePointer.
const contextCapture = <box onContextMenu={event => event.capturePointer()} />;
// @ts-expect-error Reactive roots are owned by renderApp, not the application API.
import { createRoot } from "@hewel/zgui";
void [browserElement, browserEvent, pointerEvent, html, editing, image, property, className, handler, ref, id, createDeferred,
  vectorChildren, vectorPropChildren, boxPath, boxViewBox, boxSvg, boxCanvas, rawCanvas, fabricatedCanvas,
  checkedString, roleString, roleCase, orientationString, dataBoolean, dataNumber,
  contextRelease, contextCapture];
