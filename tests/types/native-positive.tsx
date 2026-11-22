import {
  Image,
  Button,
  TextField,
  TextArea,
  For,
  Show,
  Loading,
  Errored,
  LayerSurface,
  PopupSurface,
  createCanvasScene,
  createContext,
  createDelay,
  createEffect,
  createInterval,
  createMemo,
  createSignal,
  createSpring,
  createStore,
  createTimeout,
  createProjection,
  merge,
  omit,
  onCleanup,
  onSettled,
  reconcile,
  refresh,
  renderApp,
  renderShell,
  snapshot,
  untrack,
  useContext,
  useMonitors,
  focus,
  getBackgroundBlurSupported,
  getReducedMotion,
  getSelection,
  getTextContent,
  getWindowRect,
  isAnimating,
  lockScroll,
  mountStylesheet,
  observeGeometry,
  precedes,
  registerDismissLayer,
  scrollBy,
  scrollIntoView,
  scrollTo,
  selectAll,
  setReducedMotion,
  setSelection,
  trapFocus,
  SessionLockSurface,
  WindowSurface,
  VirtualGrid,
  VirtualList,
  getSessionGeneration,
  requestSessionLock,
  beginWindowMoveDrag,
  closeWindow,
  getWindowState,
  onCloseRequest,
  setWindowDecorations,
  setWindowFullscreen,
  setWindowMaximized,
  setWindowMinimized,
  setWindowTitle,
  useWindowState,
} from "@hewel/zgui";
import type {
  BlurRegion,
  CanvasCommand,
  CanvasScene,
  CanvasTransform,
  CloseRequestEvent,
  CloseWindowOptions,
  Component,
  Context,
  NativeAnimationEvent,
  NativeChild,
  NativeContextMenuEvent,
  NativeFocusOptions,
  NativeNode,
  NativePointerBoundaryEvent,
  NativeRole,
  NativeTextControlProps,
  NativeTransitionEvent,
  NativeViewport,
  NativeWheelEvent,
  NativeWindowRect,
  ParentComponent,
  ProjectionOptions,
  ScrollIntoViewOptions,
  ScrollToOptions,
  Spring,
  SpringOptions,
  SpringPhysicsOptions,
  Store,
  StoreOptions,
  StoreReturn,
  StoreSetter,
  SurfaceInsets,
  TextFieldProps,
  TextAreaProps,
  TextSelectionRange,
  VectorProps,
  VirtualListController,
  WindowDecorations,
  WindowState,
} from "@hewel/zgui";
import type { Accessor } from "@hewel/zgui";
import type { SessionLockRequest, SessionLockState } from "@hewel/zgui";
import {
  RadioGroupPrimitive,
  RadioItemPrimitive,
  createRetainedPresence,
  createVirtualFocus,
} from "@hewel/zgui/primitives";
import type {
  PresenceController,
  RadioGroupPrimitiveProps,
  RadioItemPrimitiveProps,
  RetainedPresenceController,
  RetainedPresenceOptions,
} from "@hewel/zgui/primitives";

const Greeting: Component<{ name: string }> = props => <text>{props.name}</text>;
const Container: ParentComponent = props => <box>{props.children}</box>;
const [count, setCount] = createSignal(0);
const springPhysics: SpringPhysicsOptions = {
  stiffness: 170,
  damping: 26,
  mass: 1,
  precision: 0.01,
  velocityPrecision: 0.02,
};
const springOptions: SpringOptions = {
  ...springPhysics,
  initial: 0,
};
const sprungCount: Spring = createSpring(count, springOptions);
const sprungAccessor: Accessor<number> = sprungCount;
const springValue: number = sprungAccessor();
const springSettled: Accessor<boolean> = sprungCount.settled;
const settledFlag: boolean = springSettled();
const jumpResult: void = sprungCount.jump(0);
const impulseResult: void = sprungCount.impulse(120);
const configureResult: void = sprungCount.configure(springPhysics);
const [retainedInput] = createSignal<{ id: string } | null>(null);
const retainedOptions: RetainedPresenceOptions = { settled: sprungCount.settled };
const retained: RetainedPresenceController<{ id: string }> = createRetainedPresence(
  retainedInput,
  retainedOptions,
);
const asPresence: PresenceController = retained;
const retainedPresent: boolean = retained.present();
const retainedValue: { id: string } | undefined = retained.value();
void [
  springValue,
  settledFlag,
  jumpResult,
  impulseResult,
  configureResult,
  asPresence,
  retainedPresent,
  retainedValue,
];
const [optional, setOptional] = createSignal<string>();
setOptional();
const doubled = createMemo(() => count() * 2);
const previous = createMemo<number>(value => (value ?? 0) + count());
const [state, setState] = createStore({ items: ["a"] });
setState(draft => {
  draft.items.push("b");
});
interface ReadonlyPrefs {
  readonly fontSize: number;
  readonly note?: string;
  readonly nested: { readonly label: string };
}
const storeOpts: StoreOptions = { name: "prefs", shallow: true };
const prefsReturn: StoreReturn<ReadonlyPrefs> = createStore<ReadonlyPrefs>(
  { fontSize: 14, nested: { label: "v1" } },
  storeOpts,
);
const [prefsStore, setPrefsStore]: [Store<ReadonlyPrefs>, StoreSetter<ReadonlyPrefs>] = prefsReturn;
setPrefsStore(draft => {
  draft.fontSize = 16;
  const maybeNote: string | undefined = draft.note;
  draft.note = maybeNote ?? "updated";
  draft.nested = { label: "v2" };
});
const prefsSnap: ReadonlyPrefs = snapshot(prefsStore);
const projOpts: ProjectionOptions = { key: "id", seedLoadingValue: true };
const projected = createProjection(
  draft => {
    draft.total = count();
  },
  { total: 0 },
  projOpts,
);
const seedlessProj = createProjection(() => ({ count: count() }));
const asyncProj = createProjection(async () => ({ count: count() }));
const streamProj = createProjection(async function* () {
  yield { count: count() };
});
const [derivedStore, setDerivedStore] = createStore(
  async draft => {
    draft.total = count();
  },
  { total: 0 },
  { name: "derived" },
);
const [seedlessStore] = createStore(() => ({ active: count() > 0 }));
void refresh(projected);
void refresh(seedlessProj);
void refresh(asyncProj);
void refresh(streamProj);
void refresh(derivedStore);
void refresh(seedlessStore);
setDerivedStore(reconcile({ total: 5 }));
const reconciled: (state: string[]) => void = reconcile(["c"], null);
setState(draft => {
  reconciled(draft.items);
});
void [state, prefsSnap, projected, reconciled];
createEffect(
  () => [doubled(), previous()] as const,
  ([d, p]) => {
    console.log(d, p);
  },
);
createEffect<number>(
  value => (value ?? 0) + count(),
  (next, prev) => {
    console.log(next, prev);
  },
);
setCount(value => value + 1);
setOptional("native");
untrack(() => optional());

let borrowed: NativeNode | undefined;
let assigned!: NativeNode;
const tree = (
  <Container>
    <row
      ref={node => {
        borrowed = node;
      }}
    >
      <Greeting name="Native" />
    </row>
    <column>
      <stack>
        <label>{count()}</label>
      </stack>
    </column>
    <scroll
      onWheelCapture={(event: NativeWheelEvent) => {
        console.log(
          event.phase,
          event.scrollPhase,
          event.position.x,
          event.pointerId,
          event.pointerKind,
        );
        event.stopPropagation();
      }}
      onWheel={event => {
        const unit: "lines" | "pixels" = event.delta.unit;
        console.log(event.delta.x, event.delta.y, unit, event.modifiers.shift);
        event.preventDefault();
      }}
    >
      <control>
        <text>{optional()}</text>
      </control>
    </scroll>
    <text ref={assigned}>Variable ref</text>
    <control ariaRadioGroup={assigned} />
    <Button
      class="primary"
      title="Increase"
      disabled={false}
      ariaLabel="Increase counter"
      ariaKeyShortcuts="Control+I"
      style={{ "font-size": "20px", padding: "8px", "--accent": "blue" }}
      onClick={event => {
        event.preventDefault();
        setCount(value => value + 1);
      }}
      onKeyDownCapture={event => {
        const character: string | null = event.character;
        console.log(character, event.key, event.repeat, event.modifiers.shift);
      }}
      onPointerDown={event => {
        console.log(event.position.x, event.pointerId);
        event.capturePointer();
      }}
      onPointerMove={event => {
        event.releasePointer();
      }}
      onPointerMoveCapture={event => {
        console.log(event.coalesced.length);
      }}
      onPointerEnter={(event: NativePointerBoundaryEvent) => {
        console.log(event.type, event.position.x, event.pointerId);
      }}
      onContextMenu={(event: NativeContextMenuEvent) => {
        console.log(event.type, event.position.x, event.pointerId);
        event.preventDefault();
      }}
      onPointerLeave={event => {
        console.log(event.position.y, event.pointerKind);
      }}
      onPointerEnterCapture={event => {
        console.log(event.target, event.currentTarget);
      }}
      onPointerLeaveCapture={event => {
        console.log(event.coalesced.length);
      }}
      onPointerCancel={event => {
        event.releasePointer();
      }}
      onPointerCancelCapture={event => {
        console.log(event.pointerKind);
      }}
    >
      Increase
    </Button>
    <TextField
      value={optional()}
      defaultValue="initial"
      readOnly={false}
      ariaLabel="Name"
      ariaDescription="Native single-line field"
      onInput={event => {
        setOptional(event.value);
        const start: number = event.selection.start;
        const end: number = event.selection.end;
        const kind: "input" | "committed" = event.valueChange;
        console.log(start, end, kind, event.target === event.currentTarget);
      }}
    />
    <TextArea
      value={null}
      defaultValue={undefined}
      readOnly={null}
      disabled={false}
      onChange={event => console.log(event.value, event.selection)}
    />
    <field role="textInput" value={undefined} defaultValue="native" tabIndex={0} />
    <editor role="multilineTextInput" defaultValue={"one\n二"} readOnly />
    <Image
      src={optional()}
      ariaLabel="Package icon"
      style={{ width: "32px", height: "32px", clipPath: 'path("M 16 0 L 32 16 L 16 32 L 0 16 Z")' }}
    />
    <Image style={{ "object-fit": "cover", "object-position": "center top" }} />
    <Image
      style={{
        objectFit: "contain",
        objectPosition: "50% 50%",
        borderTopStyle: "dashed",
        borderRightStyle: "solid",
        borderBottomStyle: "dotted",
        borderLeftStyle: "solid",
        fontVariantNumeric: "tabular-nums",
      }}
    />
    <image src={null} />
    <vector
      d="M0 0 L8 8 L8 0 Z"
      viewBox="0 0 16 16"
      data-icon="save"
      role="image"
      ariaLabel="Save"
    />
    <vector d={null} viewBox={undefined} />
    <control
      checked="mixed"
      expanded={false}
      selected
      open={true}
      invalid={null}
      required
      ariaHidden={false}
      ariaModal
      orientation="vertical"
      data-action="save"
      data-count-1_x="two"
      role="slider"
    />
    <box
      onAnimationEnd={event => {
        const name: string = event.animationName;
        const elapsed: number = event.elapsedTime;
        const pseudo: string = event.pseudoElement;
        console.log(name, elapsed, pseudo, event.target === event.currentTarget);
      }}
      onAnimationStartCapture={(event: NativeAnimationEvent) => {
        event.stopPropagation();
      }}
      onTransitionRun={event => {
        console.log(event.propertyName, event.elapsedTime);
      }}
      onTransitionCancelCapture={(event: NativeTransitionEvent) => console.log(event.pseudoElement)}
    />
    <For each={[{ id: 1 }]}>
      {(item, index) => (
        <text>
          {item.id}: {index()}
        </text>
      )}
    </For>
    <For each={["a"]} keyed={false}>
      {(item, index) => (
        <text>
          {index}: {item()}
        </text>
      )}
    </For>
    <Show when={count()}>{value => <text>{value()}</text>}</Show>
    <Show when={{ name: "value" }} keyed>
      {value => <text>{value.name}</text>}
    </Show>
    <Show when={count() > 0}>
      <text>Created lazily by Universal compilation</text>
    </Show>
    <Loading fallback={<text>loading</text>}>
      <text>ready</text>
    </Loading>
    <Errored fallback={err => <text>{String(err())}</text>}>
      <text>ok</text>
    </Errored>
  </Container>
);

const Theme: Context<{ accent: string }> = createContext({ accent: "blue" });
const MaybeTheme = createContext<string>();
interface SplitControlProps {
  title: string;
  detail?: string;
  flag?: boolean;
}
const ThemedLabel: Component<SplitControlProps> = props => {
  const theme = useContext(Theme);
  const rest = omit(props, "flag");
  const merged = merge({ flag: false }, props);
  const flag: boolean | undefined = props.flag;
  const title: string = rest.title;
  return (
    <text>
      {title}
      {flag ?? merged.flag}
      {theme.accent}
      {rest.detail}
    </text>
  );
};
const provided = (
  <Theme value={{ accent: "red" }}>
    <ThemedLabel title="labeled" flag detail="extra" />
    <text>native children</text>
  </Theme>
);
const roles: NativeRole[] = [
  "alert",
  "checkBox",
  "menuItemRadio",
  "scrollView",
  "tabPanel",
  "tooltip",
];
const vectorProps: VectorProps = {
  d: "M0 0",
  viewBox: "0 0 8 8",
  svg: null,
  canvas: null,
  role: "image",
};
const canvasCommands: readonly CanvasCommand[] = [
  { kind: "rect", x: 0, y: 0, width: 8, height: 8, fill: "#abc" },
  { kind: "roundedRect", x: 1, y: 1, width: 6, height: 6, radius: 1, fill: "#1234" },
  { kind: "circle", x: 4, y: 4, radius: 3, stroke: { color: "#112233", width: 1 } },
  {
    kind: "arc",
    x: 4,
    y: 4,
    radius: 2,
    startAngle: 0,
    sweepAngle: Math.PI,
    stroke: { color: "#11223344", width: 1 },
  },
  {
    kind: "path",
    fill: "#fff",
    stroke: { color: "#000", width: 1 },
    segments: [
      { kind: "moveTo", x: 0, y: 1 },
      { kind: "lineTo", x: 3, y: 1 },
      { kind: "quadTo", cx: 4, cy: 1, x: 4, y: 2 },
      { kind: "cubicTo", c1x: 4, c1y: 3, c2x: 3, c2y: 4, x: 2, y: 4 },
      { kind: "close" },
    ],
  },
];
const canvasTransform: CanvasTransform = [1, 0, 0, 1, 2, 3];
const visual = (
  <box
    style={{
      boxShadow: "0 2px 4px black",
      "text-shadow": "1px 1px black",
      filter: "brightness(.7)",
      backdropFilter: "blur(4px)",
      mixBlendMode: "multiply",
      isolation: "isolate",
      fontVariationSettings: '"wght" 650',
      fontStyle: "italic",
      fontStretch: "condensed",
      whiteSpace: "pre-wrap",
      textOverflow: "ellipsis",
      pointerEvents: "none",
      userSelect: "none",
      translate: "10px 20%",
      rotate: "15deg",
      scale: 1.25,
      outline: "2px solid blue",
      outlineColor: "red",
      outlineStyle: "solid",
      outlineWidth: "2px",
      outlineOffset: "2px",
      letterSpacing: "0.05em",
      fontVariantNumeric: "tabular-nums",
      fontFeatureSettings: '"tnum" 1',
      textAlign: "center",
      textTransform: "uppercase",
      textDecoration: "underline",
      textDecorationLine: "none",
      gridTemplateColumns: "1fr 1fr",
      gridTemplateRows: "auto",
      gridColumn: "1 / 2",
      gridRow: "1",
      gridArea: "1 / 1",
      gridAutoFlow: "row",
      borderTop: "1px solid red",
      borderRight: "1px solid red",
      borderBottom: "1px solid red",
      borderLeft: "1px solid red",
      borderStyle: "solid",
      flex: 1,
      alignSelf: "center",
      justifySelf: "end",
      boxSizing: "border-box",
      overflowX: "hidden",
      overflowY: "auto",
    }}
  />
);
const children: NativeChild = [
  tree,
  provided,
  visual,
  "text",
  42,
  false,
  null,
  undefined,
  () => [true, "accessor"],
];
const nativeTextProps: NativeTextControlProps = {
  value: null,
  defaultValue: "seed",
  readOnly: false,
  role: "textInput",
};
const fieldProps: TextFieldProps = { value: optional(), disabled: false };
const areaProps: TextAreaProps = {
  defaultValue: "one\n二",
  onInput: event => console.log(event.selection.start),
};
const rect: NativeWindowRect | null = getWindowRect(assigned);
const textContent: string = getTextContent(assigned);
void textContent;
const ordered: boolean = precedes(assigned, assigned);
const moving: boolean = isAnimating(assigned);
void moving;
const unlockScroll: () => void = lockScroll(assigned);
unlockScroll();
const unmountStylesheet: () => void = mountStylesheet("positive-test", ":root { color: #111827; }");
unmountStylesheet();
const reveal: NativeFocusOptions = { reveal: true };
focus(assigned);
focus(assigned, reveal);
focus(assigned, { reveal: false });
const cancelBox: () => void = observeGeometry(assigned, "borderBox", rect => rect.x.toFixed(2));
const cancelBorder: () => void = observeGeometry(assigned, "borderSize", size =>
  size.width.toFixed(2),
);
const cancelContent: () => void = observeGeometry(assigned, "contentSize", size =>
  size.width.toFixed(2),
);
const cancelScroll: () => void = observeGeometry(assigned, "scrollPosition", position =>
  console.log(
    position.offset.x,
    position.offset.y,
    position.contentSize.width,
    position.scrollport.height,
  ),
);
const cancelViewport: () => void = observeGeometry(assigned, "viewport", viewport => {
  const snapshot: NativeViewport = viewport;
  return snapshot.width + snapshot.height + snapshot.scale;
});
void [cancelBox, cancelBorder, cancelContent, cancelScroll, cancelViewport];
void rect;
void ordered;
const fallbackTheme: string | undefined = MaybeTheme.defaultValue;
void [nativeTextProps, fieldProps, areaProps, roles, vectorProps, fallbackTheme];
export function mount(root: NativeNode): () => void {
  return renderApp(() => {
    const scene: CanvasScene = createCanvasScene();
    const token: number = scene.token;
    void token;
    scene.update(canvasCommands);
    scene.update(
      [
        {
          kind: "rect",
          x: 0,
          y: 0,
          width: 80,
          height: 40,
          fill: {
            kind: "linear",
            x0: 0,
            y0: 0,
            x1: 80,
            y1: 0,
            stops: [
              { offset: 0, color: "#ff0000" },
              { offset: 1, color: "#0000ff" },
            ],
          },
        },
        {
          kind: "circle",
          x: 40,
          y: 40,
          radius: 24,
          fill: {
            kind: "radial",
            cx: 40,
            cy: 40,
            radius: 24,
            repeating: false,
            stops: [
              { offset: 0, color: "#ffffff" },
              { offset: 1, color: "#000000" },
            ],
          },
          stroke: {
            color: {
              kind: "radial",
              cx: 40,
              cy: 40,
              radiusX: 24,
              radiusY: 16,
              stops: [
                { offset: 0, color: "#00ff00" },
                { offset: 1, color: "#0000ff" },
              ],
            },
            width: 3,
            lineCap: "butt",
            lineJoin: "miter",
            lineDash: [6, 4],
            lineDashOffset: 2,
          },
        },
        {
          kind: "arc",
          x: 12,
          y: 12,
          radius: 10,
          startAngle: 0,
          sweepAngle: 3.14,
          stroke: { color: "#ffffff", width: 2.2, lineCap: "round", lineJoin: "round" },
        },
      ],
      canvasTransform,
    );
    scene.update(null, canvasTransform);
    scene.update([]);
    onSettled(() => {
      const timer = setTimeout(() => console.info("timer"), 1);
      const frame = requestAnimationFrame(timestamp => console.debug(timestamp));
      queueMicrotask(() => console.warn("microtask"));
      onCleanup(() => {
        clearTimeout(timer);
        cancelAnimationFrame(frame);
      });
    });
    return [children, <vector {...vectorProps} svg='<svg viewBox="0 0 8 8"/>' canvas={scene} />];
  }, root);
}

export function mountShell(): () => void {
  return renderShell(() => {
    const monitors = useMonitors();
    return (
      <For each={monitors()}>
        {monitor => {
          const scale: number = monitor.scale;
          const logicalWidth: number = monitor.logicalWidth;
          const logicalHeight: number = monitor.logicalHeight;
          const refresh: number | null = monitor.refreshRateMilliHz;
          return (
            <LayerSurface
              name={monitor.name}
              output={monitor.id}
              layer="top"
              anchor={["top", "right", "left"] as const}
              width={0}
              height={32}
              margin={{ top: 4 }}
              keyboard="on-demand"
              exclusiveZone={32}
              inputRegion={[{ x: 0, y: 0, width: 100, height: 32 }] as const}
            >
              <text>
                {monitor.x}:{monitor.y}:{monitor.width}:{monitor.height}:{logicalWidth}:
                {logicalHeight}:{scale}:{refresh}
              </text>
            </LayerSurface>
          );
        }}
      </For>
    );
  });
}

export function mountLock(): () => void {
  return renderShell(() => {
    const lock: SessionLockRequest = requestSessionLock();
    const generation: number = getSessionGeneration();
    const lockId: number = lock.id;
    const monitors = useMonitors();
    return (
      <For each={monitors()} keyed={monitor => monitor.id}>
        {monitor => (
          <SessionLockSurface name={`lock-${monitor().id.toLowerCase()}`} output={monitor().id}>
            <text>
              {generation}:{lockId}:{((): SessionLockState => lock.state())()}
            </text>
          </SessionLockSurface>
        )}
      </For>
    );
  });
}

const hitNodes: readonly NativeNode[] = [assigned];
void (<LayerSurface name="nodes" inputRegion={hitNodes} />);
void (<PopupSurface name="menu" parent="nodes" anchorNode={borrowed} width={32} height={40} />);
void (
  <PopupSurface
    name="submenu"
    parent="menu"
    anchorRect={{ x: 1, y: 2, width: 10, height: 12 }}
    width={32}
    height={40}
    grab
    offset={{ x: 5, y: -3 }}
    placement={{ anchor: "right", gravity: "right", reactive: true }}
    onDismiss={() => {}}
  >
    <text>Popup</text>
  </PopupSurface>
);
void (
  <PopupSurface
    name="auto-menu"
    parent="nodes"
    anchorNode={borrowed}
    width={280}
    height="auto"
    minHeight={48}
    maxHeight={600}
  >
    <text>Auto</text>
  </PopupSurface>
);
void (
  <PopupSurface
    name="default-auto-menu"
    parent="nodes"
    anchorRect={{ x: 0, y: 0, width: 20, height: 20 }}
    width={240}
  >
    <text>Default auto</text>
  </PopupSurface>
);
const windowDecorations: WindowDecorations = "no-title-bar";
void (
  <WindowSurface
    name="settings"
    title="Settings"
    appId="z-shell-settings"
    width={820}
    height={600}
    minWidth={640}
    minHeight={480}
    maxWidth={1440}
    maxHeight={1080}
    resizable
    decorations={windowDecorations}
    transparent={false}
    onClose={() => {}}
  >
    <text>Settings</text>
  </WindowSurface>
);
void (
  <PopupSurface
    name="settings-menu"
    parent="settings"
    anchorRect={{ x: 8, y: 8, width: 40, height: 24 }}
    width={200}
    height="auto"
    maxHeight={400}
    inputRegion={[{ x: 4, y: 4, width: 192, height: 120 }] as const}
  >
    <text>Child</text>
  </PopupSurface>
);
const reducedMotionFlag: boolean = getReducedMotion();
setReducedMotion(reducedMotionFlag);
setReducedMotion(null);
const delayHandle = createDelay();
delayHandle.schedule(100, () => {});
delayHandle.after(50, () => {});
const isDelayPending: boolean = delayHandle.pending();
delayHandle.cancel();
const cancelOneShot: () => void = createTimeout(() => {}, 25);
cancelOneShot();
const cancelRepeating: () => void = createInterval(
  () => {},
  () => (count() > 0 ? 500 : false),
);
cancelRepeating();
const reactiveSpring = createSpring(count, { stiffness: () => 220, damping: () => 18, mass: 1 });
reactiveSpring.configure({ stiffness: 300, damping: 24 });
void (
  <VirtualList each={["a", "b"]} itemHeight={32} gap={4} overscan={2}>
    {(item, index) => (
      <text>
        {index()}:{item}
      </text>
    )}
  </VirtualList>
);
void (
  <VirtualList each={[{ id: 1, name: "a" }]} itemHeight={32} keyed={item => item.id}>
    {(item, index) => (
      <text>
        {index()}:{item().name}
      </text>
    )}
  </VirtualList>
);
void (
  <VirtualGrid each={[1, 2, 3]} itemHeight={64} minColumnWidth={120} gap={8}>
    {(item, index) => (
      <text>
        {index()}:{item}
      </text>
    )}
  </VirtualGrid>
);
void (
  <VirtualGrid each={[{ id: 1 }]} itemHeight={64} columns={2} keyed={item => item.id}>
    {(item, index) => (
      <text>
        {index()}:{item().id}
      </text>
    )}
  </VirtualGrid>
);
const [refNode, setRefNode] = createSignal<NativeNode | null>(null);
observeGeometry(refNode, "scrollPosition", pos => {
  void pos.offset.y;
});
trapFocus(refNode, { wrap: true });
lockScroll(refNode);
const dismissAccessor = registerDismissLayer(
  refNode,
  reason => {
    void reason;
  },
  { layer: "popover" },
);
dismissAccessor.update({ leaving: true });
const scrollOpts: ScrollToOptions = { top: 120, left: 0, behavior: "smooth" };
const intoViewOpts: ScrollIntoViewOptions = { block: "center", behavior: "instant" };
scrollTo(refNode, scrollOpts);
scrollBy(refNode, { top: 40 });
scrollIntoView(refNode, intoViewOpts);
const [listController, setListController] = createSignal<VirtualListController | null>(null);
void (
  <VirtualList each={["a"]} itemHeight={24} controller={setListController}>
    {item => <text>{item}</text>}
  </VirtualList>
);
listController()?.scrollToIndex(0, { align: "start", behavior: "instant" });
const currentSel: TextSelectionRange | null = getSelection(refNode);
setSelection(refNode, 0);
setSelection(refNode, 1, 3);
selectAll(refNode);
setWindowTitle("Studio");
setWindowFullscreen(true);
setWindowMaximized(false);
setWindowMinimized(false);
setWindowDecorations("no-title-bar");
beginWindowMoveDrag();
const closeOpts: CloseWindowOptions = { force: true };
closeWindow(closeOpts);
closeWindow();
const winSnap: WindowState = getWindowState();
const winAccessor: Accessor<WindowState> = useWindowState();
const stopCloseGuard = onCloseRequest((event: CloseRequestEvent) => {
  if (!event.defaultPrevented) event.preventDefault();
});
stopCloseGuard();
void [currentSel, winSnap, winAccessor];
void (<scroll ref={setRefNode} />);
void [isDelayPending, reactiveSpring];
void borrowed;

type AssertBoundary<T extends true> = T;
export type BoundaryHasNoPointerActions = AssertBoundary<
  Extract<
    "releasePointer" | "capturePointer" | "activateNow",
    keyof NativePointerBoundaryEvent
  > extends never
    ? true
    : false
>;
export type ContextMenuHasNoPointerActions = AssertBoundary<
  Extract<
    "releasePointer" | "capturePointer" | "activateNow",
    keyof NativeContextMenuEvent
  > extends never
    ? true
    : false
>;
const blurNone: BlurRegion = "none";
const blurFull: BlurRegion = "full";
const blurRects: BlurRegion = [{ x: 4, y: 8, width: 120, height: 40 }];
const blurNodes: BlurRegion = [assigned];
void (
  <WindowSurface
    name="glass"
    title="Glass"
    width={320}
    height={240}
    transparent
    blurRegion={blurFull}
  />
);
void (<LayerSurface name="glass-bar" blurRegion={blurRects} />);
void (
  <PopupSurface
    name="glass-pop"
    parent="glass-bar"
    anchorRect={{ x: 0, y: 0, width: 10, height: 10 }}
    width={120}
    blurRegion={blurNodes}
  />
);
void (
  <WindowSurface name="clear-glass" title="Clear" width={320} height={240} blurRegion={blurNone} />
);
void (
  <WindowSurface name="undef-glass" title="Undef" width={320} height={240} blurRegion={undefined} />
);
const scalarInsets: SurfaceInsets = 28;
const edgeInsets: SurfaceInsets = { top: 12, right: 24, bottom: 32 };
const blurSupported: boolean = getBackgroundBlurSupported();
void blurSupported;
void (
  <PopupSurface
    name="shadow-scalar"
    parent="glass-bar"
    anchorRect={{ x: 0, y: 0, width: 10, height: 10 }}
    width={360}
    geometryInsets={scalarInsets}
  />
);
void (
  <PopupSurface
    name="shadow-edges"
    parent="glass-bar"
    anchorRect={{ x: 0, y: 0, width: 10, height: 10 }}
    width={360}
    geometryInsets={edgeInsets}
  />
);
void (
  <PopupSurface
    name="shadow-undef"
    parent="glass-bar"
    anchorRect={{ x: 0, y: 0, width: 10, height: 10 }}
    width={360}
    geometryInsets={undefined}
  />
);
const activeOptionNode = undefined as unknown as NativeNode;
void (
  <field
    role="comboBox"
    ariaActiveDescendant={activeOptionNode}
    onKeyDown={event => {
      const composing: boolean = event.isComposing;
      void composing;
    }}
    onKeyUp={event => {
      const composing: boolean = event.isComposing;
      void composing;
    }}
  />
);
void (<box ariaActiveDescendant={null} />);
void (<box ariaActiveDescendant={undefined} />);
const vfController = createVirtualFocus<{ readonly id: string; readonly disabled?: boolean }>({
  items: () => [{ id: "a" }, { id: "b", disabled: true }],
  getId: item => item.id,
  isDisabled: item => !!item.disabled,
  columns: () => 4,
  wrap: false,
  horizontalKeys: "whenEmpty",
  textValue: () => "",
  onReveal: (id: string, itemIndex: number) => {
    void id;
    void itemIndex;
  },
  onActivate: item => {
    const activatedId: string = item.id;
    void activatedId;
  },
});
const vfId: string | null = vfController.activeId();
const vfNode: NativeNode | null = vfController.activeNode();
vfController.setActive(null);
vfController.setActive("a");
const unregVf: () => void = vfController.register("a", activeOptionNode);
void vfId;
void vfNode;
void unregVf;
void (
  <field
    role="comboBox"
    ariaActiveDescendant={vfController.activeNode()}
    onKeyDown={vfController.onKeyDown}
  />
);
const radioGroupPrimProps: RadioGroupPrimitiveProps = {
  value: "a",
  orientation: "horizontal",
  ariaLabel: "Segments",
  onValueChange: next => {
    const chosen: string = next;
    void chosen;
  },
};
const radioItemPrimProps: RadioItemPrimitiveProps = {
  value: "a",
  disabled: false,
  ariaLabel: "Alpha",
};
void (
  <RadioGroupPrimitive {...radioGroupPrimProps}>
    <RadioItemPrimitive {...radioItemPrimProps}>
      <row>
        <text>Alpha</text>
      </row>
    </RadioItemPrimitive>
  </RadioGroupPrimitive>
);
void (<box inert={true} />);
void (<box inert={false} />);
void (<box inert={null} />);
void (<box inert={undefined} />);
void (<control inert={!retained.present()} />);
