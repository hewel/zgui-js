import {
  For,
  Show,
  createCanvasScene,
  createEffect,
  createMemo,
  createSignal,
  createSpring,
  getWindowRect,
  observeGeometry,
  renderApp,
} from "@hewel/zgui";
import type {
  CanvasTransform,
  NativeNode,
  NativePointerDownEvent,
  NativePointerEvent,
  NativeScrollPosition,
  NativeWheelEvent,
} from "@hewel/zgui";
import {
  Badge,
  Button,
  Separator,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  ThemeProvider,
} from "@hewel/zgui/components";
import type { ColorScheme } from "@hewel/zgui/components";
import { buildArenaCommands, buildStrokeMatrixCommands } from "./scenes.js";

type LabTab = "arena" | "paint" | "stress";

interface SpringPreset {
  readonly id: string;
  readonly label: string;
  readonly stiffness: number;
  readonly damping: number;
  readonly mass: number;
}

const SPRING_PRESETS: readonly SpringPreset[] = [
  { id: "snappy", label: "Snappy", stiffness: 280, damping: 24, mass: 1 },
  { id: "bouncy", label: "Bouncy", stiffness: 180, damping: 9, mass: 1 },
  { id: "heavy", label: "Heavy", stiffness: 120, damping: 16, mass: 3 },
  { id: "critical", label: "Critical", stiffness: 200, damping: 28, mass: 1 },
];

const TILE_PALETTES: readonly (readonly string[])[] = [
  ["#2563eb", "#0891b2", "#059669", "#7c3aed", "#db2777", "#ea580c"],
  ["#4f46e5", "#0d9488", "#d97706", "#e11d48", "#65a30d", "#0284c7"],
  ["#334155", "#1e3a8a", "#065f46", "#6b21a8", "#9f1239", "#9a3412"],
];

export const hostApiVersion = 1;

export function mount(root: NativeNode): () => void {
  return renderApp(() => <MotionLabApp />, root);
}

function SpringArena(props: { preset: SpringPreset; dark: () => boolean }) {
  const [targetX, setTargetX] = createSignal(280);
  const [targetY, setTargetY] = createSignal(150);
  const [pulseTarget, setPulseTarget] = createSignal(0);
  const [zoom, setZoom] = createSignal(1);
  const [dragging, setDragging] = createSignal(false);
  const [pointerNote, setPointerNote] = createSignal(
    "Click or drag inside the stage; scroll wheel zooms CanvasTransform.",
  );

  const puckX = createSpring(targetX, props.preset);
  const puckY = createSpring(targetY, props.preset);
  const trailX = createSpring(puckX, { stiffness: 70, damping: 14, mass: 1.4 });
  const trailY = createSpring(puckY, { stiffness: 70, damping: 14, mass: 1.4 });
  const pulse = createSpring(pulseTarget, { stiffness: 90, damping: 12 });

  const scene = createCanvasScene();
  let stageNode: NativeNode | undefined;

  createEffect(
    () => ({
      targetX: targetX(),
      targetY: targetY(),
      puckX: puckX(),
      puckY: puckY(),
      trailX: trailX(),
      trailY: trailY(),
      pulse: pulse(),
      zoom: zoom(),
      dark: props.dark(),
    }),
    state => {
      const z = state.zoom;
      const tx = 280 * (1 - z);
      const ty = 150 * (1 - z);
      const matrix: CanvasTransform = [z, 0, 0, z, tx, ty];
      scene.update(buildArenaCommands(state), matrix);
    },
  );

  const toStageCoords = (event: NativePointerEvent) => {
    const rect = stageNode ? getWindowRect(stageNode) : null;
    const localX = rect ? event.position.x - rect.x : event.position.x;
    const localY = rect ? event.position.y - rect.y : event.position.y;
    const z = zoom();
    const x = (localX - 280 * (1 - z)) / z;
    const y = (localY - 150 * (1 - z)) / z;
    return {
      x: Math.max(20, Math.min(540, x)),
      y: Math.max(20, Math.min(280, y)),
    };
  };

  const onDown = (event: NativePointerDownEvent) => {
    event.capturePointer();
    setDragging(true);
    const pt = toStageCoords(event);
    setTargetX(pt.x);
    setTargetY(pt.y);
    pulse.jump(36);
    setPulseTarget(0);
    setPointerNote(`PointerDown captured -> (${Math.round(pt.x)}, ${Math.round(pt.y)})`);
  };

  const onMove = (event: NativePointerEvent) => {
    if (!dragging()) return;
    const pt = toStageCoords(event);
    setTargetX(pt.x);
    setTargetY(pt.y);
  };

  const onUp = (event: NativePointerEvent) => {
    if (!dragging()) return;
    event.releasePointer();
    setDragging(false);
    setPointerNote(`Released at (${Math.round(targetX())}, ${Math.round(targetY())})`);
  };

  const onWheel = (event: NativeWheelEvent) => {
    const step = event.delta.y > 0 ? -0.1 : 0.1;
    setZoom(z => Math.round(Math.max(0.6, Math.min(1.8, z + step)) * 100) / 100);
  };

  const snapLaunch = () => {
    puckX.jump(48);
    puckY.jump(250);
    setTargetX(490);
    setTargetY(62);
    pulse.jump(44);
    setPulseTarget(0);
    setPointerNote("Spring.jump(48, 250) -> launching to (490, 62)");
  };

  return (
    <row class="arena-row">
      <box
        class="arena-stage"
        ref={el => {
          stageNode = el;
        }}
        onPointerDown={onDown}
        onPointerMove={onMove}
        onPointerUp={onUp}
        onPointerCancel={onUp}
        onWheel={onWheel}
        testId="motion-arena-stage"
      >
        <vector canvas={scene} style={{ width: "560px", height: "300px" }} />
      </box>

      <column class="arena-controls">
        <text style={{ "font-weight": "600" }}>{`Active Physics: ${props.preset.label}`}</text>
        <text class="muted mono">
          {`stiffness=${props.preset.stiffness} damping=${props.preset.damping} mass=${props.preset.mass}`}
        </text>
        <text class="muted mono">
          {`puck=(${Math.round(puckX())}, ${Math.round(puckY())}) zoom=${zoom().toFixed(2)}x`}
        </text>
        <row style={{ gap: "8px", "flex-wrap": "wrap" }}>
          <Button size="sm" onClick={snapLaunch}>
            Spring.jump() Launch
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={() => {
              setTargetX(280);
              setTargetY(150);
              setZoom(1);
            }}
          >
            Recenter
          </Button>
        </row>
        <text class="muted">{pointerNote()}</text>
      </column>
    </row>
  );
}

function MotionLabApp() {
  const [scheme, setScheme] = createSignal<ColorScheme>("dark");
  const [tab, setTab] = createSignal<LabTab>("arena");
  const [presetId, setPresetId] = createSignal<string>("bouncy");

  const [shifted, setShifted] = createSignal(false);
  const [clipHits, setClipHits] = createSignal("Click any clipped shape to test hit-testing.");
  const [transitionLog, setTransitionLog] = createSignal(
    "Click Toggle Transition to observe linear(...) easing.",
  );

  const [tileCount, setTileCount] = createSignal<number>(150);
  const [paletteIdx, setPaletteIdx] = createSignal(0);
  const [compactTiles, setCompactTiles] = createSignal(false);
  const [lastTurnMs, setLastTurnMs] = createSignal<number | null>(null);
  const [scrollPos, setScrollPos] = createSignal<NativeScrollPosition | null>(null);

  const isDark = createMemo(() => scheme() === "dark");
  const activePreset = createMemo(
    () => SPRING_PRESETS.find(p => p.id === presetId()) ?? SPRING_PRESETS[0]!,
  );

  const strokeScene = createCanvasScene();
  createEffect(
    () => isDark(),
    dark => {
      strokeScene.update(buildStrokeMatrixCommands(dark));
    },
  );

  const tiles = createMemo(() => {
    const count = tileCount();
    const out: number[] = [];
    for (let i = 0; i < count; i++) out.push(i);
    return out;
  });

  const timedAction = (fn: () => void) => {
    const t0 = Date.now();
    fn();
    setLastTurnMs(Date.now() - t0);
  };

  return (
    <ThemeProvider scheme={scheme()}>
      <column class="lab-root" testId="motion-lab-root">
        <row class="lab-header">
          <column style={{ gap: "2px" }}>
            <text style={{ "font-size": "20px", "font-weight": "700" }}>
              zgui Motion, Vector &amp; Stress Lab
            </text>
            <text class="muted">
              Probing createSpring, CanvasScene transforms, clip-path masks, linear() easing, and
              node-count scaling.
            </text>
          </column>
          <row style={{ gap: "8px", "align-items": "center" }}>
            <Button
              size="sm"
              variant="outline"
              onClick={() => setScheme(s => (s === "dark" ? "light" : "dark"))}
            >
              {scheme() === "dark" ? "Light" : "Dark"}
            </Button>
          </row>
        </row>

        <Tabs value={tab()} onValueChange={v => setTab(v as LabTab)} label="Motion lab sections">
          <TabsList label="Motion lab sections">
            <TabsTrigger testId="lab-tab-arena" value="arena">
              Spring Arena
            </TabsTrigger>
            <TabsTrigger testId="lab-tab-paint" value="paint">
              Vectors, Clip &amp; Easing
            </TabsTrigger>
            <TabsTrigger
              testId="lab-tab-stress"
              value="stress"
            >{`Stress Grid (${tileCount()})`}</TabsTrigger>
          </TabsList>

          <TabsContent value="arena">
            <column class="lab-panel">
              <row style={{ gap: "8px", "align-items": "center", "flex-wrap": "wrap" }}>
                <text class="muted">
                  Physics preset (re-mounts springs because options are creation-time):
                </text>
                <For each={SPRING_PRESETS}>
                  {p => (
                    <Button
                      size="xs"
                      variant={presetId() === p.id ? "default" : "outline"}
                      onClick={() => setPresetId(p.id)}
                    >
                      {p.label}
                    </Button>
                  )}
                </For>
              </row>
              <Show when={activePreset()} keyed>
                {p => <SpringArena preset={p} dark={isDark} />}
              </Show>
            </column>
          </TabsContent>

          <TabsContent value="paint">
            <column class="lab-panel">
              <text style={{ "font-weight": "600" }}>
                Canvas Stroke Caps (butt / round / square), Joins (miter / round / bevel) &amp;
                Cubic Beziers
              </text>
              <vector canvas={strokeScene} style={{ width: "560px", height: "190px" }} />

              <Separator />

              <text style={{ "font-weight": "600" }}>
                clip-path Mask Clipping &amp; Hit-Test Probe
              </text>
              <box class="clip-grid">
                <box
                  class="clip-card"
                  role="button"
                  tabIndex={0}
                  style={{ "background-color": "#2563eb", "clip-path": "circle(45% at 50% 50%)" }}
                  onClick={() => setClipHits("Hit: circle(45%)")}
                >
                  <text>circle()</text>
                </box>
                <box
                  class="clip-card"
                  role="button"
                  tabIndex={0}
                  style={{ "background-color": "#059669", "clip-path": "inset(10% 8% round 22px)" }}
                  onClick={() => setClipHits("Hit: inset() round")}
                >
                  <text>inset()</text>
                </box>
                <box
                  class="clip-card"
                  role="button"
                  tabIndex={0}
                  style={{
                    "background-color": "#7c3aed",
                    "clip-path": "polygon(50% 4%, 94% 28%, 94% 72%, 50% 96%, 6% 72%, 6% 28%)",
                  }}
                  onClick={() => setClipHits("Hit: hexagon polygon()")}
                >
                  <text>hexagon</text>
                </box>
                <box
                  class="clip-card"
                  role="button"
                  tabIndex={0}
                  style={{
                    "background-color": "#db2777",
                    "clip-path":
                      "polygon(50% 2%, 63% 36%, 98% 36%, 70% 58%, 80% 94%, 50% 72%, 20% 94%, 30% 58%, 2% 36%, 37% 36%)",
                  }}
                  onClick={() => setClipHits("Hit: 5-point star polygon()")}
                >
                  <text>star</text>
                </box>
              </box>
              <text class="muted mono">{clipHits()}</text>

              <Separator />

              <row class="border-showcase">
                <box class="border-sample">
                  <text>Per-side borders: dashed / dotted / double / solid</text>
                </box>
                <Button size="sm" onClick={() => setShifted(v => !v)}>
                  Toggle CSS linear(...) Transition
                </Button>
              </row>

              <box class="transition-track">
                <box
                  class="transition-pill"
                  data-shifted={shifted() ? "true" : "false"}
                  onTransitionStart={e => setTransitionLog(`transitionstart: ${e.propertyName}`)}
                  onTransitionEnd={e =>
                    setTransitionLog(
                      `transitionend: ${e.propertyName} (${e.elapsedTime.toFixed(2)}s)`,
                    )
                  }
                >
                  <text>linear(...)</text>
                </box>
              </box>
              <text class="muted mono">{transitionLog()}</text>
            </column>
          </TabsContent>

          <TabsContent value="stress">
            <column class="lab-panel">
              <row
                style={{
                  "justify-content": "space-between",
                  "align-items": "center",
                  "flex-wrap": "wrap",
                  gap: "8px",
                }}
              >
                <row style={{ gap: "6px", "align-items": "center" }}>
                  <text class="muted">Tile count:</text>
                  <For each={[40, 150, 400, 800]}>
                    {count => (
                      <Button
                        testId={`stress-count-${count}`}
                        size="xs"
                        variant={tileCount() === count ? "default" : "outline"}
                        onClick={() => timedAction(() => setTileCount(count))}
                      >
                        {String(count)}
                      </Button>
                    )}
                  </For>
                </row>
                <row style={{ gap: "6px", "align-items": "center" }}>
                  <Button
                    size="xs"
                    variant="outline"
                    onClick={() =>
                      timedAction(() => setPaletteIdx(i => (i + 1) % TILE_PALETTES.length))
                    }
                  >
                    Rotate Palette
                  </Button>
                  <Button
                    size="xs"
                    variant="outline"
                    onClick={() => timedAction(() => setCompactTiles(v => !v))}
                  >
                    {compactTiles() ? "Standard Size" : "Compact Size"}
                  </Button>
                  <Badge variant="outline">
                    {lastTurnMs() === null ? "JS turn: —" : `JS turn: ${lastTurnMs()}ms`}
                  </Badge>
                </row>
              </row>

              <scroll
                class="stress-scroll"
                ref={node => {
                  observeGeometry(node, "scrollPosition", setScrollPos);
                }}
                testId="stress-scroll"
              >
                <box class="stress-grid">
                  <For each={tiles()}>
                    {idx => {
                      const color = createMemo(() => {
                        const pal = TILE_PALETTES[paletteIdx()]!;
                        return pal[idx % pal.length]!;
                      });
                      return (
                        <box
                          class="stress-tile"
                          data-compact={compactTiles() ? "true" : "false"}
                          style={{ "background-color": color() }}
                        >
                          <text>{`#${idx + 1}`}</text>
                          <text>{color()}</text>
                        </box>
                      );
                    }}
                  </For>
                </box>
              </scroll>

              <text class="muted mono">
                {scrollPos()
                  ? `scrollY=${Math.round(scrollPos()!.offset.y)} / contentH=${Math.round(scrollPos()!.contentSize.height)} / portH=${Math.round(scrollPos()!.scrollport.height)}`
                  : "scrollMetrics: waiting"}
              </text>
            </column>
          </TabsContent>
        </Tabs>
      </column>
    </ThemeProvider>
  );
}
