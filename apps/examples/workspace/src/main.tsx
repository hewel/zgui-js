import { For, Image, Show, createEffect, createMemo, createSignal, renderApp } from "@hewel/zgui";
import type { NativeNode } from "@hewel/zgui";
import {
  AspectRatio,
  Avatar,
  AvatarFallback,
  Badge,
  Button,
  Checkbox,
  ContextMenu,
  ContextMenuContent,
  ContextMenuTrigger,
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
  HoverCard,
  HoverCardContent,
  HoverCardTrigger,
  Icon,
  Input,
  InputGroup,
  InputGroupAddon,
  InputGroupInput,
  Kbd,
  KbdGroup,
  Label,
  MenuItem,
  MenuLabel,
  MenuSeparator,
  MenuSub,
  MenuSubContent,
  MenuSubTrigger,
  Menubar,
  MenubarCheckboxItem,
  MenubarContent,
  MenubarItem,
  MenubarMenu,
  MenubarRadioGroup,
  MenubarRadioItem,
  MenubarSeparator,
  MenubarShortcut,
  MenubarTrigger,
  Popover,
  PopoverClose,
  PopoverContent,
  PopoverDescription,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
  Progress,
  RadioGroup,
  RadioGroupItem,
  Separator,
  Slider,
  Switch,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
  Textarea,
  ThemeProvider,
  ToggleGroup,
  ToggleGroupItem,
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@hewel/zgui/components";
import type { CheckedState, ColorScheme, ThemePreset } from "@hewel/zgui/components";
import { ELLIPSIS, PLUS, SEARCH } from "@hewel/zgui/icons";
import { registerImageBytes } from "@hewel/zgui-services";
import { INITIAL_NOTES, INITIAL_TASKS, buildProceduralSvg, encodeAsciiBytes } from "./data.js";
import type { KanbanTask, NoteDoc, ProceduralSceneId, TaskColumn, TaskPriority } from "./data.js";

type WorkspaceTab = "notes" | "kanban" | "assets";
type ObjectFitMode = "contain" | "cover" | "fill" | "none" | "scale-down";
type ClipPreset = "none" | "circle" | "rounded" | "pentagon";

const CLIP_CSS: Record<ClipPreset, string | undefined> = {
  none: undefined,
  circle: "circle(44% at 50% 50%)",
  rounded: "inset(8% 8% 8% 8% round 28px)",
  pentagon: "polygon(50% 4%, 96% 36%, 79% 94%, 21% 94%, 4% 36%)",
};

const COLUMNS: readonly { id: TaskColumn; label: string }[] = [
  { id: "backlog", label: "Backlog" },
  { id: "active", label: "In Progress" },
  { id: "done", label: "Done" },
];

export const hostApiVersion = 1;

export function mount(root: NativeNode): () => void {
  return renderApp(() => <WorkspaceApp />, root);
}

function WorkspaceApp() {
  const [scheme, setScheme] = createSignal<ColorScheme>("dark");
  const [preset, setPreset] = createSignal<ThemePreset>("base");
  const [tab, setTab] = createSignal<WorkspaceTab>("notes");
  const [showInspector, setShowInspector] = createSignal(true);
  const [showTags, setShowTags] = createSignal(true);
  const [density, setDensity] = createSignal("cosy");
  const [status, setStatus] = createSignal(
    "Ready — try Menubar, right-clicking Kanban cards, or editing notes.",
  );

  const [notes, setNotes] = createSignal<readonly NoteDoc[]>(INITIAL_NOTES);
  const [activeNoteId, setActiveNoteId] = createSignal(INITIAL_NOTES[0]!.id);
  const [docSearch, setDocSearch] = createSignal("");
  const [newNoteOpen, setNewNoteOpen] = createSignal(false);
  const [draftTitle, setDraftTitle] = createSignal("");
  const [deleteDialogOpen, setDeleteDialogOpen] = createSignal(false);
  const [wordWrap, setWordWrap] = createSignal<CheckedState>(true);

  const [tasks, setTasks] = createSignal<readonly KanbanTask[]>(INITIAL_TASKS);
  const [onlyHigh, setOnlyHigh] = createSignal(false);

  const [sceneId, setSceneId] = createSignal<ProceduralSceneId>("mesh");
  const [hueShift, setHueShift] = createSignal(0);
  const [fitMode, setFitMode] = createSignal<ObjectFitMode>("cover");
  const [clipMode, setClipMode] = createSignal<ClipPreset>("none");
  const [frameWidth, setFrameWidth] = createSignal(420);
  const [assetOpacity, setAssetOpacity] = createSignal(100);
  const [blobSrc, setBlobSrc] = createSignal<string | null>(null);

  createEffect(
    () => ({ scene: sceneId(), hue: hueShift() }),
    ({ scene, hue }) => {
      const svg = buildProceduralSvg(scene, hue);
      const handle = registerImageBytes(encodeAsciiBytes(svg));
      setBlobSrc(handle.src);
      return () => {
        handle.release();
      };
    },
  );

  const activeNote = createMemo(
    () => notes().find(n => n.id === activeNoteId()) ?? notes()[0] ?? null,
  );

  const visibleNotes = createMemo(() => {
    const q = docSearch().trim().toLowerCase();
    if (!q) return notes();
    return notes().filter(
      n => n.title.toLowerCase().includes(q) || n.body.toLowerCase().includes(q),
    );
  });

  const noteStats = createMemo(() => {
    const body = activeNote()?.body ?? "";
    const trimmed = body.trim();
    const words = trimmed ? trimmed.split(/\s+/).length : 0;
    const lines = body ? body.split("\n").length : 0;
    return { chars: body.length, words, lines };
  });

  const updateActiveNote = (patch: Partial<NoteDoc>) => {
    const current = activeNote();
    if (!current) return;
    setNotes(prev => prev.map(n => (n.id === current.id ? { ...n, ...patch } : n)));
  };

  const createNote = () => {
    const title = draftTitle().trim() || `Untitled Note ${notes().length + 1}`;
    const id = `n-${Date.now()}`;
    const next: NoteDoc = {
      id,
      title,
      category: "Field Notes",
      tags: ["draft"],
      pinned: false,
      body: "New document created inside native Dialog.",
    };
    setNotes(prev => [next, ...prev]);
    setActiveNoteId(id);
    setDraftTitle("");
    setNewNoteOpen(false);
    setTab("notes");
    setStatus(`Created note "${title}"`);
  };

  const deleteActiveNote = () => {
    const current = activeNote();
    if (!current || notes().length <= 1) return;
    const remaining = notes().filter(n => n.id !== current.id);
    setNotes(remaining);
    setActiveNoteId(remaining[0]!.id);
    setDeleteDialogOpen(false);
    setStatus(`Deleted "${current.title}"`);
  };

  const moveTask = (id: string, column: TaskColumn) => {
    setTasks(prev => prev.map(t => (t.id === id ? { ...t, column } : t)));
    setStatus(`Moved task ${id} -> ${column}`);
  };

  const setTaskPriority = (id: string, priority: TaskPriority) => {
    setTasks(prev => prev.map(t => (t.id === id ? { ...t, priority } : t)));
    setStatus(`Set task ${id} priority -> ${priority}`);
  };

  const duplicateTask = (task: KanbanTask) => {
    const copy: KanbanTask = {
      ...task,
      id: `t-${Date.now()}`,
      title: `${task.title} (copy)`,
    };
    setTasks(prev => [...prev, copy]);
    setStatus(`Duplicated "${task.title}"`);
  };

  const removeTask = (id: string) => {
    setTasks(prev => prev.filter(t => t.id !== id));
    setStatus(`Removed task ${id}`);
  };

  const completionPct = createMemo(() => {
    const all = tasks();
    if (!all.length) return 0;
    const done = all.filter(t => t.column === "done").length;
    return Math.round((done / all.length) * 100);
  });

  const handleGlobalKey = (event: { key: string | null; modifiers: { control: boolean } }) => {
    if (!event.modifiers.control || !event.key) return;
    const lower = event.key.toLowerCase();
    if (lower === "n") {
      setNewNoteOpen(true);
      setStatus("Shortcut Ctrl+N -> New Note dialog");
    } else if (lower === "b") {
      setShowInspector(v => !v);
      setStatus("Shortcut Ctrl+B -> Toggled Inspector");
    }
  };

  return (
    <ThemeProvider scheme={scheme()} light={{ preset: preset() }} dark={{ preset: preset() }}>
      <column class="ws-root" testId="workspace-root" onKeyDown={handleGlobalKey}>
        <row class="ws-topbar">
          <row style={{ gap: "12px", "align-items": "center" }}>
            <text style={{ "font-weight": "700" }}>zgui Studio</text>
            <Menubar label="Workspace menu">
              <MenubarMenu value="file">
                <MenubarTrigger>File</MenubarTrigger>
                <MenubarContent style={{ width: "210px" }}>
                  <MenubarItem onSelect={() => setNewNoteOpen(true)}>
                    <text>New Note</text>
                    <MenubarShortcut>Ctrl+N</MenubarShortcut>
                  </MenubarItem>
                  <MenubarItem
                    onSelect={() => {
                      const next: KanbanTask = {
                        id: `t-${Date.now()}`,
                        title: `Investigate runtime case #${tasks().length + 1}`,
                        assignee: "Ada",
                        role: "QA",
                        column: "backlog",
                        priority: "medium",
                        notes: "Added from Menubar File -> New Task.",
                      };
                      setTasks(prev => [...prev, next]);
                      setTab("kanban");
                      setStatus(`Added "${next.title}"`);
                    }}
                  >
                    New Kanban Task
                  </MenubarItem>
                  <MenubarSeparator />
                  <MenubarItem
                    disabled={notes().length <= 1}
                    onSelect={() => setDeleteDialogOpen(true)}
                  >
                    Delete Current Note
                  </MenubarItem>
                </MenubarContent>
              </MenubarMenu>

              <MenubarMenu value="view">
                <MenubarTrigger>View</MenubarTrigger>
                <MenubarContent style={{ width: "220px" }}>
                  <MenubarCheckboxItem
                    checked={showInspector()}
                    closeOnSelect={false}
                    onCheckedChange={v => {
                      setShowInspector(v);
                      setStatus(`Inspector ${v ? "shown" : "hidden"}`);
                    }}
                  >
                    Show Inspector Panel
                  </MenubarCheckboxItem>
                  <MenubarCheckboxItem
                    checked={showTags()}
                    closeOnSelect={false}
                    onCheckedChange={setShowTags}
                  >
                    Show Document Tags
                  </MenubarCheckboxItem>
                  <MenubarSeparator />
                  <MenubarRadioGroup value={density()} onValueChange={setDensity}>
                    <MenubarRadioItem value="compact">Compact</MenubarRadioItem>
                    <MenubarRadioItem value="cosy">Cosy</MenubarRadioItem>
                    <MenubarRadioItem value="spacious">Spacious</MenubarRadioItem>
                  </MenubarRadioGroup>
                  <MenubarSeparator />
                  <MenuSub>
                    <MenuSubTrigger>Theme Preset</MenuSubTrigger>
                    <MenuSubContent style={{ width: "150px" }}>
                      <MenuItem onSelect={() => setPreset("base")}>Base</MenuItem>
                      <MenuItem onSelect={() => setPreset("ocean")}>Ocean</MenuItem>
                      <MenuItem onSelect={() => setPreset("midnight")}>Midnight</MenuItem>
                      <MenuItem onSelect={() => setPreset("ember")}>Ember</MenuItem>
                    </MenuSubContent>
                  </MenuSub>
                </MenubarContent>
              </MenubarMenu>
            </Menubar>
          </row>

          <row style={{ gap: "8px", "align-items": "center" }}>
            <Badge variant="outline">{`Preset: ${preset()} · ${density()}`}</Badge>
            <Button size="xs" variant="outline" onClick={() => setNewNoteOpen(true)}>
              <Icon icon={PLUS} size="sm" />
              <text>New Note</text>
            </Button>
            <Button
              size="xs"
              variant="outline"
              onClick={() => setScheme(s => (s === "dark" ? "light" : "dark"))}
            >
              {scheme() === "dark" ? "Light" : "Dark"}
            </Button>
          </row>
        </row>

        <row class="ws-body">
          <column class="ws-sidebar">
            <InputGroup>
              <InputGroupAddon>
                <Icon icon={SEARCH} size="sm" />
              </InputGroupAddon>
              <InputGroupInput
                value={docSearch()}
                onValueChange={setDocSearch}
                placeholder="Search docs..."
                label="Search documents"
              />
            </InputGroup>

            <column style={{ gap: "6px" }}>
              <text
                class="muted"
                style={{ "font-weight": "600" }}
              >{`Documents (${visibleNotes().length})`}</text>
              <For each={visibleNotes()}>
                {doc => (
                  <box
                    class="doc-item"
                    role="button"
                    tabIndex={0}
                    data-active={activeNote()?.id === doc.id ? "true" : "false"}
                    onClick={() => {
                      setActiveNoteId(doc.id);
                      setTab("notes");
                      setStatus(`Opened "${doc.title}"`);
                    }}
                  >
                    <row style={{ "justify-content": "space-between", "align-items": "center" }}>
                      <text class="doc-title">{doc.title}</text>
                      <Show when={doc.pinned}>
                        <Badge variant="secondary">Pin</Badge>
                      </Show>
                    </row>
                    <text class="muted">{doc.category}</text>
                  </box>
                )}
              </For>
            </column>

            <Separator />

            <column style={{ gap: "8px" }}>
              <text class="muted" style={{ "font-weight": "600" }}>
                Board Summary
              </text>
              <row style={{ "justify-content": "space-between" }}>
                <text class="muted">Completion</text>
                <text class="mono muted">{`${completionPct()}%`}</text>
              </row>
              <Progress value={completionPct()} max={100} label="Board completion" />
              <row style={{ "justify-content": "space-between", "align-items": "center" }}>
                <text class="muted">High priority only</text>
                <Switch
                  size="sm"
                  checked={onlyHigh()}
                  onCheckedChange={setOnlyHigh}
                  ariaLabel="Filter high priority tasks"
                />
              </row>
            </column>
          </column>

          <column class="ws-main">
            <Tabs
              value={tab()}
              onValueChange={v => setTab(v as WorkspaceTab)}
              label="Workspace views"
            >
              <TabsList label="Workspace views">
                <TabsTrigger testId="ws-tab-notes" value="notes">
                  Notes Editor
                </TabsTrigger>
                <TabsTrigger
                  testId="ws-tab-kanban"
                  value="kanban"
                >{`Kanban Board (${tasks().length})`}</TabsTrigger>
                <TabsTrigger testId="ws-tab-assets" value="assets">
                  Asset &amp; Clip Inspector
                </TabsTrigger>
              </TabsList>

              <TabsContent value="notes">
                <Show when={activeNote()}>
                  <column class="editor-shell">
                    <row
                      style={{
                        "justify-content": "space-between",
                        "align-items": "center",
                        gap: "8px",
                      }}
                    >
                      <box style={{ flex: "1" }}>
                        <Input
                          label="Document title"
                          value={activeNote()!.title}
                          onValueChange={title => updateActiveNote({ title })}
                        />
                      </box>

                      <Popover>
                        <PopoverTrigger size="sm" variant="outline">
                          Doc Info
                        </PopoverTrigger>
                        <PopoverContent style={{ width: "260px" }}>
                          <PopoverHeader>
                            <PopoverTitle>{activeNote()!.title}</PopoverTitle>
                            <PopoverDescription>{`Category: ${activeNote()!.category}`}</PopoverDescription>
                          </PopoverHeader>
                          <column style={{ gap: "4px" }}>
                            <text class="muted mono">{`Words: ${noteStats().words}`}</text>
                            <text class="muted mono">{`Lines: ${noteStats().lines}`}</text>
                            <text class="muted mono">{`Characters: ${noteStats().chars}`}</text>
                          </column>
                          <PopoverClose size="xs" variant="outline">
                            Done
                          </PopoverClose>
                        </PopoverContent>
                      </Popover>

                      <TooltipProvider>
                        <Tooltip>
                          <TooltipTrigger>
                            <Button
                              size="sm"
                              variant={activeNote()!.pinned ? "default" : "outline"}
                              onClick={() => updateActiveNote({ pinned: !activeNote()!.pinned })}
                            >
                              {activeNote()!.pinned ? "Pinned" : "Pin"}
                            </Button>
                          </TooltipTrigger>
                          <TooltipContent>Pin document to top of sidebar</TooltipContent>
                        </Tooltip>
                      </TooltipProvider>
                    </row>

                    <Show when={showTags()}>
                      <row style={{ gap: "8px", "align-items": "center", "flex-wrap": "wrap" }}>
                        <text class="muted">Tags:</text>
                        <ToggleGroup
                          selection="multiple"
                          size="sm"
                          variant="outline"
                          value={activeNote()!.tags}
                          onValueChange={tags => updateActiveNote({ tags })}
                          label="Document tags"
                        >
                          <ToggleGroupItem value="layout">layout</ToggleGroupItem>
                          <ToggleGroupItem value="editing">editing</ToggleGroupItem>
                          <ToggleGroupItem value="overlays">overlays</ToggleGroupItem>
                          <ToggleGroupItem value="paint">paint</ToggleGroupItem>
                          <ToggleGroupItem value="a11y">a11y</ToggleGroupItem>
                        </ToggleGroup>
                      </row>
                    </Show>

                    <Textarea
                      class="editor-textarea"
                      label="Document body"
                      placeholder="Write notes..."
                      value={activeNote()!.body}
                      onValueChange={body => updateActiveNote({ body })}
                    />

                    <row style={{ "justify-content": "space-between", "align-items": "center" }}>
                      <text class="muted mono">
                        {`${noteStats().words} words · ${noteStats().lines} lines · ${noteStats().chars} chars`}
                      </text>
                      <Button
                        size="xs"
                        variant="destructive"
                        disabled={notes().length <= 1}
                        onClick={() => setDeleteDialogOpen(true)}
                      >
                        Delete note
                      </Button>
                    </row>
                  </column>
                </Show>
              </TabsContent>

              <TabsContent value="kanban">
                <column style={{ gap: "12px" }}>
                  <row style={{ "justify-content": "space-between", "align-items": "center" }}>
                    <text class="muted">
                      Right-click any card for ContextMenu actions (move column, priority,
                      duplicate, delete).
                    </text>
                    <Badge variant="outline">{`${completionPct()}% done`}</Badge>
                  </row>

                  <box class="kanban-grid">
                    <For each={COLUMNS}>
                      {col => {
                        const colTasks = createMemo(() =>
                          tasks().filter(
                            t => t.column === col.id && (!onlyHigh() || t.priority === "high"),
                          ),
                        );
                        return (
                          <column class="kanban-col">
                            <row class="kanban-col-header">
                              <text style={{ "font-weight": "600" }}>{col.label}</text>
                              <Badge variant="secondary">{String(colTasks().length)}</Badge>
                            </row>
                            <For each={colTasks()}>
                              {task => (
                                <ContextMenu>
                                  <ContextMenuTrigger>
                                    <column class="kanban-card">
                                      <row
                                        style={{
                                          "justify-content": "space-between",
                                          "align-items": "center",
                                          gap: "6px",
                                        }}
                                      >
                                        <Badge
                                          variant={
                                            task.priority === "high"
                                              ? "destructive"
                                              : task.priority === "medium"
                                                ? "default"
                                                : "secondary"
                                          }
                                        >
                                          {task.priority}
                                        </Badge>
                                        <DropdownMenu>
                                          <DropdownMenuTrigger size="xs" variant="ghost">
                                            <Icon icon={ELLIPSIS} size="sm" />
                                          </DropdownMenuTrigger>
                                          <DropdownMenuContent style={{ width: "170px" }}>
                                            <MenuLabel>Move to</MenuLabel>
                                            <MenuItem onSelect={() => moveTask(task.id, "backlog")}>
                                              Backlog
                                            </MenuItem>
                                            <MenuItem onSelect={() => moveTask(task.id, "active")}>
                                              In Progress
                                            </MenuItem>
                                            <MenuItem onSelect={() => moveTask(task.id, "done")}>
                                              Done
                                            </MenuItem>
                                            <MenuSeparator />
                                            <MenuItem onSelect={() => duplicateTask(task)}>
                                              Duplicate
                                            </MenuItem>
                                            <MenuItem
                                              destructive
                                              onSelect={() => removeTask(task.id)}
                                            >
                                              Delete
                                            </MenuItem>
                                          </DropdownMenuContent>
                                        </DropdownMenu>
                                      </row>

                                      <text style={{ "font-weight": "500" }}>{task.title}</text>
                                      <text class="muted">{task.notes}</text>

                                      <row
                                        style={{
                                          "justify-content": "space-between",
                                          "align-items": "center",
                                        }}
                                      >
                                        <HoverCard>
                                          <HoverCardTrigger>
                                            <row style={{ gap: "6px", "align-items": "center" }}>
                                              <Avatar size="sm">
                                                <AvatarFallback>
                                                  {task.assignee.slice(0, 2).toUpperCase()}
                                                </AvatarFallback>
                                              </Avatar>
                                              <text class="muted">{task.assignee}</text>
                                            </row>
                                          </HoverCardTrigger>
                                          <HoverCardContent style={{ width: "210px" }}>
                                            <column style={{ gap: "4px" }}>
                                              <text style={{ "font-weight": "600" }}>
                                                {task.assignee}
                                              </text>
                                              <text class="muted">{`Owner · ${task.role}`}</text>
                                            </column>
                                          </HoverCardContent>
                                        </HoverCard>
                                        <text class="muted mono">{task.id}</text>
                                      </row>
                                    </column>
                                  </ContextMenuTrigger>
                                  <ContextMenuContent style={{ width: "190px" }}>
                                    <MenuLabel>{task.title}</MenuLabel>
                                    <MenuSeparator />
                                    <MenuSub>
                                      <MenuSubTrigger>Move column</MenuSubTrigger>
                                      <MenuSubContent style={{ width: "150px" }}>
                                        <MenuItem onSelect={() => moveTask(task.id, "backlog")}>
                                          Backlog
                                        </MenuItem>
                                        <MenuItem onSelect={() => moveTask(task.id, "active")}>
                                          In Progress
                                        </MenuItem>
                                        <MenuItem onSelect={() => moveTask(task.id, "done")}>
                                          Done
                                        </MenuItem>
                                      </MenuSubContent>
                                    </MenuSub>
                                    <MenuSub>
                                      <MenuSubTrigger>Priority</MenuSubTrigger>
                                      <MenuSubContent style={{ width: "140px" }}>
                                        <MenuItem onSelect={() => setTaskPriority(task.id, "high")}>
                                          High
                                        </MenuItem>
                                        <MenuItem
                                          onSelect={() => setTaskPriority(task.id, "medium")}
                                        >
                                          Medium
                                        </MenuItem>
                                        <MenuItem onSelect={() => setTaskPriority(task.id, "low")}>
                                          Low
                                        </MenuItem>
                                      </MenuSubContent>
                                    </MenuSub>
                                    <MenuSeparator />
                                    <MenuItem onSelect={() => duplicateTask(task)}>
                                      Duplicate card
                                    </MenuItem>
                                    <MenuItem destructive onSelect={() => removeTask(task.id)}>
                                      Delete card
                                    </MenuItem>
                                  </ContextMenuContent>
                                </ContextMenu>
                              )}
                            </For>
                          </column>
                        );
                      }}
                    </For>
                  </box>
                </column>
              </TabsContent>

              <TabsContent value="assets">
                <column style={{ gap: "12px" }}>
                  <row style={{ gap: "8px", "align-items": "center", "flex-wrap": "wrap" }}>
                    <text class="muted">Scene:</text>
                    <Button
                      size="xs"
                      variant={sceneId() === "mesh" ? "default" : "outline"}
                      onClick={() => setSceneId("mesh")}
                    >
                      Mesh
                    </Button>
                    <Button
                      size="xs"
                      variant={sceneId() === "radar" ? "default" : "outline"}
                      onClick={() => setSceneId("radar")}
                    >
                      Radar
                    </Button>
                    <Button
                      size="xs"
                      variant={sceneId() === "poster" ? "default" : "outline"}
                      onClick={() => setSceneId("poster")}
                    >
                      Poster
                    </Button>
                    <Separator orientation="vertical" />
                    <text class="muted">object-fit:</text>
                    <For each={["cover", "contain", "fill", "none", "scale-down"] as const}>
                      {mode => (
                        <Button
                          size="xs"
                          variant={fitMode() === mode ? "default" : "outline"}
                          onClick={() => setFitMode(mode)}
                        >
                          {mode}
                        </Button>
                      )}
                    </For>
                  </row>

                  <row style={{ gap: "8px", "align-items": "center", "flex-wrap": "wrap" }}>
                    <text class="muted">clip-path:</text>
                    <For each={["none", "circle", "rounded", "pentagon"] as const}>
                      {mode => (
                        <Button
                          size="xs"
                          variant={clipMode() === mode ? "default" : "outline"}
                          onClick={() => setClipMode(mode)}
                        >
                          {mode}
                        </Button>
                      )}
                    </For>
                  </row>

                  <box class="asset-stage">
                    <box class="asset-frame" style={{ width: `${frameWidth()}px` }}>
                      <AspectRatio ratio={16 / 10}>
                        <Image
                          src={blobSrc()}
                          style={{
                            width: "100%",
                            height: "100%",
                            "object-fit": fitMode(),
                            "object-position": "center",
                            opacity: assetOpacity() / 100,
                            "clip-path": CLIP_CSS[clipMode()],
                          }}
                        />
                      </AspectRatio>
                    </box>
                  </box>

                  <column style={{ gap: "10px" }}>
                    <row style={{ gap: "12px", "align-items": "center" }}>
                      <text
                        class="muted mono"
                        style={{ width: "120px" }}
                      >{`Hue: ${hueShift()}°`}</text>
                      <box style={{ flex: "1" }}>
                        <Slider
                          value={hueShift()}
                          onValueChange={setHueShift}
                          min={0}
                          max={360}
                          step={15}
                          label="Hue shift"
                        />
                      </box>
                    </row>
                    <row style={{ gap: "12px", "align-items": "center" }}>
                      <text
                        class="muted mono"
                        style={{ width: "120px" }}
                      >{`Width: ${frameWidth()}px`}</text>
                      <box style={{ flex: "1" }}>
                        <Slider
                          value={frameWidth()}
                          onValueChange={setFrameWidth}
                          min={220}
                          max={560}
                          step={20}
                          label="Frame width"
                        />
                      </box>
                    </row>
                    <row style={{ gap: "12px", "align-items": "center" }}>
                      <text
                        class="muted mono"
                        style={{ width: "120px" }}
                      >{`Opacity: ${assetOpacity()}%`}</text>
                      <box style={{ flex: "1" }}>
                        <Slider
                          value={assetOpacity()}
                          onValueChange={setAssetOpacity}
                          min={20}
                          max={100}
                          step={5}
                          label="Asset opacity"
                        />
                      </box>
                    </row>
                  </column>
                </column>
              </TabsContent>
            </Tabs>
          </column>

          <Show when={showInspector()}>
            <column class="ws-inspector">
              <text style={{ "font-weight": "600" }}>Inspector</text>
              <Separator />

              <column style={{ gap: "8px" }}>
                <text class="muted">Color Scheme</text>
                <RadioGroup
                  value={scheme()}
                  onValueChange={v => setScheme(v as ColorScheme)}
                  label="Color scheme"
                >
                  <column style={{ gap: "6px" }}>
                    <row style={{ gap: "8px", "align-items": "center" }}>
                      <RadioGroupItem value="dark" label="Dark" />
                      <Label>Dark</Label>
                    </row>
                    <row style={{ gap: "8px", "align-items": "center" }}>
                      <RadioGroupItem value="light" label="Light" />
                      <Label>Light</Label>
                    </row>
                    <row style={{ gap: "8px", "align-items": "center" }}>
                      <RadioGroupItem value="system" label="System" />
                      <Label>System</Label>
                    </row>
                  </column>
                </RadioGroup>
              </column>

              <Separator />

              <row style={{ gap: "8px", "align-items": "center" }}>
                <Checkbox
                  checked={wordWrap()}
                  onCheckedChange={setWordWrap}
                  ariaLabel="Soft wrap indicator"
                />
                <text class="muted">{`Soft wrap: ${String(wordWrap())}`}</text>
              </row>

              <Separator />

              <column style={{ gap: "6px" }}>
                <text class="muted">Keyboard Shortcuts</text>
                <row style={{ "justify-content": "space-between", "align-items": "center" }}>
                  <text class="muted">New Note</text>
                  <KbdGroup>
                    <Kbd>Ctrl</Kbd>
                    <Kbd>N</Kbd>
                  </KbdGroup>
                </row>
                <row style={{ "justify-content": "space-between", "align-items": "center" }}>
                  <text class="muted">Toggle Inspector</text>
                  <KbdGroup>
                    <Kbd>Ctrl</Kbd>
                    <Kbd>B</Kbd>
                  </KbdGroup>
                </row>
                <row style={{ "justify-content": "space-between", "align-items": "center" }}>
                  <text class="muted">Close Overlay</text>
                  <Kbd>Esc</Kbd>
                </row>
              </column>
            </column>
          </Show>
        </row>

        <row class="ws-statusbar">
          <text>{status()}</text>
          <text class="mono">{`zgui-blob: ${blobSrc() ? "active" : "none"} · docs: ${notes().length}`}</text>
        </row>

        <Dialog open={newNoteOpen()} onOpenChange={setNewNoteOpen}>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Create Note</DialogTitle>
              <DialogDescription>Add a new document to the studio sidebar.</DialogDescription>
            </DialogHeader>
            <Input
              label="New note title"
              placeholder="e.g. Wayland Input Notes"
              value={draftTitle()}
              onValueChange={setDraftTitle}
            />
            <DialogFooter>
              <DialogClose variant="outline">Cancel</DialogClose>
              <Button onClick={createNote}>Create</Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>

        <Dialog open={deleteDialogOpen()} onOpenChange={setDeleteDialogOpen}>
          <DialogContent>
            <DialogHeader>
              <DialogTitle>Delete Note?</DialogTitle>
              <DialogDescription>
                {activeNote() ? `Remove "${activeNote()!.title}" from this session?` : ""}
              </DialogDescription>
            </DialogHeader>
            <DialogFooter>
              <DialogClose variant="outline">Cancel</DialogClose>
              <Button variant="destructive" onClick={deleteActiveNote}>
                Delete
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </column>
    </ThemeProvider>
  );
}
