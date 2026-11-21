import {
  For,
  Show,
  VirtualList,
  createCanvasScene,
  createEffect,
  createInterval,
  createMemo,
  createSignal,
  createSpring,
  getReducedMotion,
  observeGeometry,
  onCleanup,
  renderApp,
  setReducedMotion,
} from "@hewel/zgui";
import type { NativeNode, NativeScrollPosition } from "@hewel/zgui";
import {
  Alert,
  AlertDescription,
  AlertTitle,
  Badge,
  Button,
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Icon,
  InputGroup,
  InputGroupAddon,
  InputGroupInput,
  Progress,
  Separator,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  ThemeProvider,
} from "@hewel/zgui/components";
import type { ColorScheme, ThemePreset } from "@hewel/zgui/components";
import { SEARCH } from "@hewel/zgui/icons";
import { getResolvedCapabilities, onControlMessage, spawnCommand } from "@hewel/zgui-services";
import type { CommandStream, HostRequest } from "@hewel/zgui-services";
import { buildGaugeCommands, buildTimelineCommands } from "./charts.js";
import {
  formatGiB,
  parseDfOutput,
  parseFreeOutput,
  parsePsOutput,
  parseVmstatLine,
  sortProcesses,
} from "./telemetry.js";
import type { CpuSample, DiskRow, MemoryStats, ProcessRow, ProcessSortKey } from "./telemetry.js";

type SysmonTab = "overview" | "processes" | "storage";

const PRESETS: readonly ThemePreset[] = ["base", "ocean", "midnight", "ember"];
const REFRESH_RATES = [1000, 2000, 5000] as const;

export const hostApiVersion = 1;

export function mount(root: NativeNode): () => void {
  return renderApp(() => <SysmonApp />, root);
}

function GaugeCard(props: {
  label: string;
  percent: () => number;
  detail: () => string;
  accent: string;
  dark: () => boolean;
}) {
  const scene = createCanvasScene();
  createEffect(
    () => ({ pct: props.percent(), dark: props.dark() }),
    ({ pct, dark }) => {
      scene.update(buildGaugeCommands(pct, props.accent, dark ? "#272a33" : "#e5e7eb"));
    },
  );
  return (
    <box class="gauge-card">
      <stack class="gauge-canvas-wrap">
        <vector canvas={scene} style={{ width: "104px", height: "104px" }} />
        <text class="gauge-center-value">{`${Math.round(props.percent())}%`}</text>
      </stack>
      <column class="gauge-meta">
        <text class="gauge-label">{props.label}</text>
        <text class="gauge-detail">{props.detail()}</text>
      </column>
    </box>
  );
}

function SysmonApp() {
  const [scheme, setScheme] = createSignal<ColorScheme>("dark");
  const [preset, setPreset] = createSignal<ThemePreset>("ocean");
  const [tab, setTab] = createSignal<SysmonTab>("overview");
  const [live, setLive] = createSignal(true);
  const [refreshMs, setRefreshMs] = createSignal<number>(2000);
  const [kernel, setKernel] = createSignal("Probing host...");
  const [samples, setSamples] = createSignal<readonly CpuSample[]>([]);
  const [memory, setMemory] = createSignal<MemoryStats | null>(null);
  const [processes, setProcesses] = createSignal<readonly ProcessRow[]>([]);
  const [disks, setDisks] = createSignal<readonly DiskRow[]>([]);
  const [errorMsg, setErrorMsg] = createSignal<string | null>(null);
  const [tickCount, setTickCount] = createSignal(0);

  const [query, setQuery] = createSignal("");
  const [sortBy, setSortBy] = createSignal<ProcessSortKey>("cpu");
  const [sortDesc, setSortDesc] = createSignal(true);
  const [selectedProc, setSelectedProc] = createSignal<ProcessRow | null>(null);
  const [scrollInfo, setScrollInfo] = createSignal<NativeScrollPosition | null>(null);

  const latest = createMemo(() => samples()[samples().length - 1] ?? null);
  const cpuTarget = createMemo(() => latest()?.total ?? 0);
  const waitTarget = createMemo(() => latest()?.wait ?? 0);
  const memTarget = createMemo(() => {
    const m = memory();
    return m && m.totalMb > 0 ? (m.usedMb / m.totalMb) * 100 : 0;
  });
  const swapTarget = createMemo(() => {
    const m = memory();
    return m && m.swapTotalMb > 0 ? (m.swapUsedMb / m.swapTotalMb) * 100 : 0;
  });

  const cpuSpring = createSpring(cpuTarget, { stiffness: 120, damping: 20 });
  const memSpring = createSpring(memTarget, { stiffness: 120, damping: 20 });
  const swapSpring = createSpring(swapTarget, { stiffness: 120, damping: 20 });
  const waitSpring = createSpring(waitTarget, { stiffness: 120, damping: 20 });

  const timelineScene = createCanvasScene();
  const isDark = createMemo(() => scheme() === "dark");

  createEffect(
    () => ({ data: samples(), dark: isDark() }),
    ({ data, dark }) => {
      timelineScene.update(buildTimelineCommands(data, dark));
    },
  );

  const pending = new Set<HostRequest<unknown>>();
  const runBuffered = (
    program: string,
    args: readonly string[],
    onOutput: (stdout: string) => void,
  ) => {
    try {
      const req = spawnCommand(program, args);
      pending.add(req);
      void req.promise
        .then(result => {
          pending.delete(req);
          if (result.status === 0) onOutput(result.stdout);
        })
        .catch(err => {
          pending.delete(req);
          const msg = err instanceof Error ? err.message : String(err);
          if (!msg.includes("Cancelled")) setErrorMsg(`${program}: ${msg}`);
        });
    } catch (err) {
      setErrorMsg(err instanceof Error ? err.message : String(err));
    }
  };

  const refreshSnapshot = () => {
    runBuffered("/usr/bin/free", ["-m"], out => {
      const parsed = parseFreeOutput(out);
      if (parsed) setMemory(parsed);
    });
    runBuffered("/usr/bin/ps", ["-eo", "pid,pcpu,pmem,rss,stat,comm", "--sort=-pcpu"], out =>
      setProcesses(parsePsOutput(out)),
    );
    runBuffered(
      "/usr/bin/df",
      ["-h", "--output=target,pcent,used,size", "-x", "tmpfs", "-x", "devtmpfs"],
      out => setDisks(parseDfOutput(out)),
    );
  };

  runBuffered("/usr/bin/uname", ["-srm"], out => setKernel(out.trim()));
  refreshSnapshot();

  // Owner-scoped polling via createInterval (#43): pauses cleanly when stream is paused.
  createInterval(refreshSnapshot, () => (live() ? refreshMs() : false));

  let streamHandle: CommandStream | null = null;
  let vmstatLines = 0;

  createEffect(
    () => live(),
    isLive => {
      if (!isLive) {
        streamHandle?.kill();
        streamHandle = null;
        return;
      }
      vmstatLines = 0;
      const stream = spawnCommand("/usr/bin/vmstat", ["1"], {
        stream: true,
        onStdout(line) {
          const parsed = parseVmstatLine(line);
          if (!parsed) return;
          vmstatLines += 1;
          // Skip the first vmstat line (boot-time average) once real 1s ticks begin.
          if (vmstatLines === 1 && samples().length > 0) return;
          setSamples(prev => [...prev.slice(-59), parsed]);
          setTickCount(n => n + 1);
        },
        onStderr(line) {
          if (line.trim()) setErrorMsg(`vmstat: ${line.trim()}`);
        },
        onError(err) {
          setErrorMsg(`vmstat stream error: ${err.message}`);
        },
      });
      streamHandle = stream;
      return () => {
        stream.kill();
        if (streamHandle === stream) streamHandle = null;
      };
    },
  );

  onCleanup(() => {
    streamHandle?.kill();
    for (const req of pending) req.cancel();
    pending.clear();
  });

  // Multi-branch object returns & ResolvedCapabilities work directly with JsonValue (#46).
  onControlMessage(args => {
    const [cmd, arg] = args;
    if (cmd === "refresh") {
      refreshSnapshot();
      return { ok: true, ticks: tickCount() };
    }
    if (cmd === "pause") {
      setLive(false);
      return { live: false };
    }
    if (cmd === "resume") {
      setLive(true);
      return { live: true };
    }
    if (cmd === "tab" && (arg === "overview" || arg === "processes" || arg === "storage")) {
      setTab(arg);
      return { tab: arg };
    }
    return {
      kernel: kernel(),
      live: live(),
      ticks: tickCount(),
      cpu: latest()?.total ?? null,
      processCount: processes().length,
      capabilities: getResolvedCapabilities(),
    };
  });

  const filteredProcesses = createMemo(() => {
    const q = query().trim().toLowerCase();
    const base = q
      ? processes().filter(p => p.command.toLowerCase().includes(q) || String(p.pid).includes(q))
      : processes();
    return sortProcesses(base, sortBy(), sortDesc());
  });

  const toggleSort = (key: ProcessSortKey) => {
    if (sortBy() === key) setSortDesc(v => !v);
    else {
      setSortBy(key);
      setSortDesc(key !== "pid");
    }
  };

  const resetSprings = () => {
    cpuSpring.jump(0);
    memSpring.jump(0);
    swapSpring.jump(0);
    waitSpring.jump(0);
  };

  const cyclePreset = () => {
    const idx = PRESETS.indexOf(preset());
    setPreset(PRESETS[(idx + 1) % PRESETS.length]!);
  };

  const bindScroll = (node: NativeNode) => {
    observeGeometry(node, "scrollPosition", pos => setScrollInfo(pos));
  };

  return (
    <ThemeProvider scheme={scheme()} light={{ preset: preset() }} dark={{ preset: preset() }}>
      <column class="sysmon-shell" testId="sysmon-root">
        <row class="sysmon-header">
          <column style={{ gap: "2px" }}>
            <row style={{ gap: "8px", "align-items": "center" }}>
              <text class="sysmon-title">zgui System Monitor</text>
              <Badge variant={live() ? "default" : "secondary"}>
                {live() ? `LIVE · tick ${tickCount()}` : "PAUSED"}
              </Badge>
            </row>
            <text class="sysmon-subtitle">{kernel()}</text>
          </column>

          <row class="sysmon-controls">
            <Button
              size="sm"
              variant={live() ? "outline" : "default"}
              onClick={() => setLive(v => !v)}
            >
              {live() ? "Pause stream" : "Resume stream"}
            </Button>
            <Button size="sm" variant="outline" onClick={refreshSnapshot}>
              Refresh now
            </Button>
            <Button size="sm" variant="outline" onClick={resetSprings}>
              Spring.jump(0)
            </Button>
            <Button
              size="sm"
              variant={getReducedMotion() ? "default" : "outline"}
              onClick={() => setReducedMotion(!getReducedMotion())}
            >
              {getReducedMotion() ? "Motion: Reduced" : "Motion: Full"}
            </Button>
            <Button size="sm" variant="outline" onClick={cyclePreset}>
              {`Preset: ${preset()}`}
            </Button>
            <Button
              size="sm"
              variant="outline"
              onClick={() => setScheme(s => (s === "dark" ? "light" : "dark"))}
            >
              {scheme() === "dark" ? "Light" : "Dark"}
            </Button>
          </row>
        </row>

        <Show when={errorMsg()}>
          <Alert variant="destructive">
            <AlertTitle>Host Service Notice</AlertTitle>
            <AlertDescription>{errorMsg()}</AlertDescription>
          </Alert>
        </Show>

        <box class="gauge-grid">
          <GaugeCard
            label="CPU Busy"
            percent={cpuSpring}
            detail={() => {
              const s = latest();
              return s ? `us ${s.user}% · sy ${s.sys}% · id ${s.idle}%` : "Waiting for vmstat...";
            }}
            accent="#3b82f6"
            dark={isDark}
          />
          <GaugeCard
            label="Memory"
            percent={memSpring}
            detail={() => {
              const m = memory();
              return m ? `${formatGiB(m.usedMb)} / ${formatGiB(m.totalMb)}` : "Reading free -m...";
            }}
            accent="#10b981"
            dark={isDark}
          />
          <GaugeCard
            label="Swap"
            percent={swapSpring}
            detail={() => {
              const m = memory();
              return m
                ? `${formatGiB(m.swapUsedMb)} / ${formatGiB(m.swapTotalMb)}`
                : "Reading swap...";
            }}
            accent="#a855f7"
            dark={isDark}
          />
          <GaugeCard
            label="I/O Wait"
            percent={waitSpring}
            detail={() => {
              const s = latest();
              return s ? `run ${s.runningProcs} · blk ${s.blockedProcs}` : "Sampling...";
            }}
            accent="#f59e0b"
            dark={isDark}
          />
        </box>

        <Tabs
          value={tab()}
          onValueChange={v => setTab(v as SysmonTab)}
          label="System monitor views"
        >
          <TabsList label="System monitor views">
            <TabsTrigger testId="sysmon-tab-overview" value="overview">
              CPU Timeline
            </TabsTrigger>
            <TabsTrigger testId="sysmon-tab-processes" value="processes">
              {`Processes (${processes().length})`}
            </TabsTrigger>
            <TabsTrigger testId="sysmon-tab-storage" value="storage">
              {`Storage (${disks().length})`}
            </TabsTrigger>
          </TabsList>

          <TabsContent value="overview">
            <column class="panel-card">
              <row class="chart-row">
                <column style={{ gap: "2px" }}>
                  <text style={{ "font-weight": "600" }}>
                    Live 60-Second CPU Telemetry (CanvasScene)
                  </text>
                  <text class="muted">
                    Streaming vmstat 1 + createInterval() snapshot polling ({refreshMs() / 1000}s).
                  </text>
                </column>
                <row class="legend-row">
                  <row style={{ gap: "6px", "align-items": "center" }}>
                    <box class="legend-dot" style={{ "background-color": "#3b82f6" }} />
                    <text class="muted">User+Sys</text>
                  </row>
                  <row style={{ gap: "6px", "align-items": "center" }}>
                    <box class="legend-dot" style={{ "background-color": "#ef4444" }} />
                    <text class="muted">Kernel</text>
                  </row>
                  <row style={{ gap: "6px", "align-items": "center" }}>
                    <box class="legend-dot" style={{ "background-color": "#f59e0b" }} />
                    <text class="muted">Total + IO Wait</text>
                  </row>
                </row>
              </row>

              <vector canvas={timelineScene} style={{ width: "620px", height: "160px" }} />

              <box class="metric-strip">
                <column class="metric-pill">
                  <text class="muted">Interrupts / s</text>
                  <text class="metric-pill-value">{latest()?.interrupts ?? 0}</text>
                </column>
                <column class="metric-pill">
                  <text class="muted">Context Switches / s</text>
                  <text class="metric-pill-value">{latest()?.contextSwitches ?? 0}</text>
                </column>
                <column class="metric-pill">
                  <text class="muted">Available RAM</text>
                  <text class="metric-pill-value">
                    {memory() ? formatGiB(memory()!.availableMb) : "—"}
                  </text>
                </column>
                <column class="metric-pill">
                  <text class="muted">Samples Buffered</text>
                  <text class="metric-pill-value">{`${samples().length} / 60`}</text>
                </column>
              </box>
            </column>
          </TabsContent>

          <TabsContent value="processes">
            <column class="panel-card">
              <row class="proc-toolbar">
                <box style={{ width: "260px" }}>
                  <InputGroup>
                    <InputGroupAddon>
                      <Icon icon={SEARCH} size="sm" />
                    </InputGroupAddon>
                    <InputGroupInput
                      value={query()}
                      onValueChange={setQuery}
                      placeholder="Filter PID or process..."
                      label="Filter processes"
                    />
                  </InputGroup>
                </box>

                <row style={{ gap: "6px", "align-items": "center", "flex-wrap": "wrap" }}>
                  <text class="muted">Sort:</text>
                  <Button
                    size="xs"
                    variant={sortBy() === "cpu" ? "default" : "outline"}
                    onClick={() => toggleSort("cpu")}
                  >
                    CPU
                  </Button>
                  <Button
                    size="xs"
                    variant={sortBy() === "mem" ? "default" : "outline"}
                    onClick={() => toggleSort("mem")}
                  >
                    MEM
                  </Button>
                  <Button
                    size="xs"
                    variant={sortBy() === "rss" ? "default" : "outline"}
                    onClick={() => toggleSort("rss")}
                  >
                    RSS
                  </Button>
                  <Button
                    size="xs"
                    variant={sortBy() === "pid" ? "default" : "outline"}
                    onClick={() => toggleSort("pid")}
                  >
                    PID
                  </Button>
                  <Separator orientation="vertical" />
                  <text class="muted">Poll:</text>
                  <For each={REFRESH_RATES}>
                    {ms => (
                      <Button
                        size="xs"
                        variant={refreshMs() === ms ? "default" : "outline"}
                        onClick={() => setRefreshMs(ms)}
                      >
                        {`${ms / 1000}s`}
                      </Button>
                    )}
                  </For>
                </row>
              </row>

              <box class="proc-header-row">
                <text>PID</text>
                <text>COMMAND</text>
                <text>CPU %</text>
                <text>MEM %</text>
                <text>RSS</text>
                <text>STAT</text>
              </box>

              <VirtualList
                class="proc-scroll"
                ref={bindScroll}
                testId="sysmon-proc-scroll"
                each={filteredProcesses()}
                keyed={p => p.pid}
                itemHeight={32}
                overscan={6}
                initialHeight={360}
                fallback={
                  <box style={{ padding: "16px" }}>
                    <text class="muted">No matching processes.</text>
                  </box>
                }
              >
                {proc => (
                  <box
                    class="proc-row"
                    role="button"
                    tabIndex={0}
                    onClick={() => setSelectedProc(proc())}
                  >
                    <text>{String(proc().pid)}</text>
                    <text class="proc-cmd">{proc().command}</text>
                    <text>{proc().cpu.toFixed(1)}</text>
                    <text>{proc().mem.toFixed(1)}</text>
                    <text>{`${proc().rssMb.toFixed(1)}M`}</text>
                    <text>{proc().stat}</text>
                  </box>
                )}
              </VirtualList>

              <row style={{ "justify-content": "space-between", "align-items": "center" }}>
                <text class="muted">
                  {`VirtualList: ${filteredProcesses().length} of ${processes().length} live processes (all mounted lazily)`}
                </text>
                <text class="muted mono">
                  {scrollInfo()
                    ? `scrollY=${Math.round(scrollInfo()!.offset.y)} / contentH=${Math.round(scrollInfo()!.contentSize.height)} / portH=${Math.round(scrollInfo()!.scrollport.height)}`
                    : "scrollMetrics: waiting"}
                </text>
              </row>
            </column>
          </TabsContent>

          <TabsContent value="storage">
            <column class="panel-card">
              <text style={{ "font-weight": "600" }}>Mounted Filesystems</text>
              <column class="disk-list">
                <For each={disks()}>
                  {disk => (
                    <column class="disk-row">
                      <row style={{ "justify-content": "space-between" }}>
                        <text class="mono">{disk.mount}</text>
                        <text class="muted mono">{`${disk.used} / ${disk.size} (${disk.percent}%)`}</text>
                      </row>
                      <Progress value={disk.percent} max={100} label={`Usage for ${disk.mount}`} />
                    </column>
                  )}
                </For>
              </column>
            </column>
          </TabsContent>
        </Tabs>

        <Dialog
          open={selectedProc() !== null}
          onOpenChange={open => {
            if (!open) setSelectedProc(null);
          }}
        >
          <DialogContent>
            <DialogHeader>
              <DialogTitle>
                {selectedProc() ? `Process ${selectedProc()!.command}` : "Process"}
              </DialogTitle>
              <DialogDescription>
                {selectedProc() ? `PID ${selectedProc()!.pid} · State ${selectedProc()!.stat}` : ""}
              </DialogDescription>
            </DialogHeader>
            <Show when={selectedProc()}>
              <column style={{ gap: "8px" }}>
                <row style={{ "justify-content": "space-between" }}>
                  <text class="muted">CPU Utilization</text>
                  <text class="mono">{`${selectedProc()!.cpu.toFixed(1)}%`}</text>
                </row>
                <row style={{ "justify-content": "space-between" }}>
                  <text class="muted">Memory Share</text>
                  <text class="mono">{`${selectedProc()!.mem.toFixed(1)}%`}</text>
                </row>
                <row style={{ "justify-content": "space-between" }}>
                  <text class="muted">Resident Set Size</text>
                  <text class="mono">{`${selectedProc()!.rssMb.toFixed(1)} MiB`}</text>
                </row>
              </column>
            </Show>
            <DialogFooter>
              <DialogClose variant="outline">Close</DialogClose>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </column>
    </ThemeProvider>
  );
}
