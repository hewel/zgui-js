import {
  Button,
  Portal,
  Show,
  createSignal,
  focus,
  getWindowRect,
  lockScroll,
  mountStylesheet,
  renderApp,
} from "@hewel/zgui";
import type { NativeNode, NativeWindowRect } from "@hewel/zgui";
export const hostApiVersion = 1;
const buttonStyle = {
  width: "150px",
  height: "32px",
  padding: "6px",
  background: "#374151",
  color: "#ffffff",
};
const css = `:root { display: block; overflow-y: scroll; }
.scroll-page { width: 100%; height: 2400px; color: #1e3a8a; background-color: #eff6ff; font-size: 14px; }
.page-row { height: 180px; flex-shrink: 0; padding: 12px; }
`;
function read(node: NativeNode): NativeWindowRect {
  const rect = getWindowRect(node);
  if (!rect) throw Error("missing scroll geometry");
  return rect;
}
function compare(current: NativeWindowRect, saved: NativeWindowRect | undefined) {
  if (!saved) return "unsaved";
  if (current.x !== saved.x || current.width !== saved.width || current.height !== saved.height)
    return "geometry-changed";
  if (current.y === saved.y) return "unchanged";
  return current.y < saved.y ? "down" : "up";
}
function LeaseBox(props: { testId: string }) {
  const [node, setNode] = createSignal<NativeNode | null>(null);
  lockScroll(node);
  return <box testId={props.testId} ref={setNode} />;
}
function Leases(props: { source: boolean; outer: boolean; inner: boolean }) {
  return (
    <Show when={props.source}>
      <box style="position: absolute; width: 0px; height: 0px">
        <Show when={props.outer}>
          <LeaseBox testId="outer-lease" />
        </Show>
        <Show when={props.inner}>
          <LeaseBox testId="inner-lease" />
        </Show>
      </box>
    </Show>
  );
}
export function mount(root: NativeNode) {
  return renderApp(() => {
    mountStylesheet("scroll-lock-fixture", css);
    const [outer, setOuter] = createSignal(false);
    const [inner, setInner] = createSignal(false);
    const [source, setSource] = createSignal(true);
    const [pageStatus, setPageStatus] = createSignal("unsaved");
    const [innerStatus, setInnerStatus] = createSignal("unsaved");
    const [pageDelta, setPageDelta] = createSignal("0");
    let page!: NativeNode;
    let port!: NativeNode;
    let content!: NativeNode;
    let pageSaved: NativeWindowRect | undefined;
    let innerSaved: NativeWindowRect | undefined;
    return (
      <>
        <column
          ref={node => {
            page = node;
          }}
          testId="page"
          class="scroll-page"
        >
          {Array.from({ length: 12 }, (_, index) => (
            <box class="page-row" style={{ "background-color": index % 2 ? "#dbeafe" : "#eff6ff" }}>
              <text>Background row {index + 1}</text>
            </box>
          ))}
        </column>
        <Portal layer="popover">
          <column style="position: fixed; left: 12px; top: 12px; width: 740px; padding: 12px; gap: 8px; background-color: #ffffff; color: #18181b; font-size: 14px">
            <row style={{ gap: "8px" }}>
              <Button testId="outer-lock" style={buttonStyle} onClick={() => setOuter(true)}>
                Lock outer
              </Button>
              <Button testId="inner-lock" style={buttonStyle} onClick={() => setInner(true)}>
                Lock inner
              </Button>
              <Button testId="inner-release" style={buttonStyle} onClick={() => setInner(false)}>
                Release inner
              </Button>
              <Button testId="outer-release" style={buttonStyle} onClick={() => setOuter(false)}>
                Release outer
              </Button>
            </row>
            <row style={{ gap: "8px" }}>
              <Button testId="source-toggle" style={buttonStyle} onClick={() => setSource(v => !v)}>
                Toggle owner
              </Button>
              <Button
                testId="page-save"
                style={buttonStyle}
                onClick={() => {
                  pageSaved = read(page);
                  setPageStatus("saved");
                }}
              >
                Save page
              </Button>
              <Button
                testId="page-compare"
                style={buttonStyle}
                onClick={() => {
                  const current = read(page);
                  setPageStatus(compare(current, pageSaved));
                  setPageDelta(pageSaved ? String(current.y - pageSaved.y) : "unsaved");
                }}
              >
                Compare page
              </Button>
              <Button testId="focus-inner" style={buttonStyle} onClick={() => focus(port)}>
                Focus inner
              </Button>
            </row>
            <row style={{ gap: "8px" }}>
              <Button
                testId="inner-save"
                style={buttonStyle}
                onClick={() => {
                  innerSaved = read(content);
                  setInnerStatus("saved");
                }}
              >
                Save inner
              </Button>
              <Button
                testId="inner-compare"
                style={buttonStyle}
                onClick={() => setInnerStatus(compare(read(content), innerSaved))}
              >
                Compare inner
              </Button>
            </row>
            <row style={{ gap: "8px" }}>
              <text>Page:</text>
              <text testId="page-status">{pageStatus()}</text>
              <text>dy:</text>
              <text testId="page-delta">{pageDelta()}</text>
              <text>Inner:</text>
              <text testId="inner-status">{innerStatus()}</text>
              <text>Requested holds:</text>
              <text testId="holds">{source() ? Number(outer()) + Number(inner()) : 0}</text>
            </row>
            <Leases source={source()} outer={outer()} inner={inner()} />
          </column>
          <scroll
            ref={node => {
              port = node;
            }}
            testId="inner-scroll"
            tabIndex={0}
            style="position: fixed; left: 420px; top: 230px; width: 320px; height: 200px; overflow: auto; background-color: #ffffff; color: #18181b; font-size: 14px"
          >
            <column
              ref={node => {
                content = node;
              }}
              testId="inner-content"
              style="height: 900px; padding: 12px; gap: 80px"
            >
              <text>Inner scroll row 1</text>
              <text>Inner scroll row 2</text>
              <text>Inner scroll row 3</text>
              <text>Inner scroll row 4</text>
              <text>Inner scroll row 5</text>
              <text>Inner scroll row 6</text>
            </column>
          </scroll>
        </Portal>
      </>
    );
  }, root);
}
