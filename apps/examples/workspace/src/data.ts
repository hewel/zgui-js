export interface NoteDoc {
  readonly id: string;
  readonly title: string;
  readonly category: "Architecture" | "Field Notes" | "Specs";
  readonly tags: readonly string[];
  readonly body: string;
  readonly pinned: boolean;
}

export type TaskColumn = "backlog" | "active" | "done";
export type TaskPriority = "low" | "medium" | "high";

export interface KanbanTask {
  readonly id: string;
  readonly title: string;
  readonly assignee: string;
  readonly role: string;
  readonly column: TaskColumn;
  readonly priority: TaskPriority;
  readonly notes: string;
}

export const INITIAL_NOTES: readonly NoteDoc[] = [
  {
    id: "n-1",
    title: "Native Layout & Editing Notes",
    category: "Architecture",
    tags: ["layout", "editing"],
    pinned: true,
    body: [
      "zgui-js renders Solid 2.0.1 components directly to native nodes over QuickJS-NG.",
      "",
      "Things to probe in this editor:",
      "1. Multiline typing, cursor navigation, word selection, and copy/paste.",
      "2. Scrolling behavior when text exceeds the Textarea height.",
      "3. Focus restoration after opening Menubar, Popover, ContextMenu, and Dialog.",
      "4. Keyboard shortcuts (Ctrl+N, Ctrl+B) across focused controls.",
    ].join("\n"),
  },
  {
    id: "n-2",
    title: "Overlay Layering Checklist",
    category: "Specs",
    tags: ["overlays", "a11y"],
    pinned: false,
    body: [
      "Verify escape-order dismissal when stacking:",
      "- Menubar -> Submenu",
      "- ContextMenu on Kanban card -> Move / Duplicate",
      "- Dialog modal with focus trap and backdrop scrim",
      "- HoverCard and Tooltip coexistence inside scrolling panels",
    ].join("\n"),
  },
  {
    id: "n-3",
    title: "Image & Clip-Path Observations",
    category: "Field Notes",
    tags: ["paint", "images"],
    pinned: false,
    body: [
      "Dynamic procedural SVG assets are encoded to Uint8Array and registered",
      "through registerImageBytes() -> zgui-blob:// handles.",
      "Switch object-fit, object-position, and clip-path masks in the Asset tab.",
    ].join("\n"),
  },
];

export const INITIAL_TASKS: readonly KanbanTask[] = [
  {
    id: "t-1",
    title: "Audit multiline Textarea scroll & IME composition",
    assignee: "Ada",
    role: "Text & Editing",
    column: "backlog",
    priority: "high",
    notes: "Verify scroll-to-caret and CJK IME candidate positioning on Wayland.",
  },
  {
    id: "t-2",
    title: "Evaluate drag-and-drop ergonomics without HTML5 DnD",
    assignee: "Linus",
    role: "Input & Pointer",
    column: "backlog",
    priority: "medium",
    notes: "Kanban currently uses ContextMenu + buttons; test pointer capture drag.",
  },
  {
    id: "t-3",
    title: "Stress-test nested overlays & focus restoration",
    assignee: "Grace",
    role: "Overlay Runtime",
    column: "active",
    priority: "high",
    notes: "Right-click cards, open Menubar submenus, then trigger modal Dialog.",
  },
  {
    id: "t-4",
    title: "Verify zgui-blob:// image release on rapid switching",
    assignee: "Ken",
    role: "Paint & Cache",
    column: "active",
    priority: "medium",
    notes: "Ensure registerImageBytes handles dispose cleanly with zero leaks.",
  },
  {
    id: "t-5",
    title: "Ship clip-path mask clipping & per-side borders",
    assignee: "Margaret",
    role: "Renderer",
    column: "done",
    priority: "low",
    notes: "Landed in 9ad1f06; validate circle, inset, and polygon masks live.",
  },
];

export type ProceduralSceneId = "mesh" | "radar" | "poster";

export function encodeAsciiBytes(text: string): Uint8Array {
  const bytes = new Uint8Array(text.length);
  for (let i = 0; i < text.length; i++) {
    bytes[i] = text.charCodeAt(i) & 0x7f;
  }
  return bytes;
}

export function buildProceduralSvg(scene: ProceduralSceneId, hueShift: number): string {
  const h1 = (210 + hueShift) % 360;
  const h2 = (320 + hueShift) % 360;
  const h3 = (155 + hueShift) % 360;
  if (scene === "radar") {
    return [
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 480 300">`,
      `<rect width="480" height="300" fill="hsl(${h1}, 36%, 14%)"/>`,
      `<circle cx="240" cy="150" r="120" fill="none" stroke="hsl(${h3}, 75%, 55%)" stroke-width="2" opacity="0.45"/>`,
      `<circle cx="240" cy="150" r="80" fill="none" stroke="hsl(${h3}, 75%, 55%)" stroke-width="2" opacity="0.65"/>`,
      `<circle cx="240" cy="150" r="40" fill="none" stroke="hsl(${h3}, 75%, 55%)" stroke-width="2" opacity="0.85"/>`,
      `<line x1="100" y1="150" x2="380" y2="150" stroke="hsl(${h3}, 70%, 60%)" stroke-width="1.5" opacity="0.5"/>`,
      `<line x1="240" y1="20" x2="240" y2="280" stroke="hsl(${h3}, 70%, 60%)" stroke-width="1.5" opacity="0.5"/>`,
      `<path d="M240 150 L335 78 A120 120 0 0 1 360 150 Z" fill="hsl(${h3}, 85%, 55%)" opacity="0.35"/>`,
      `<circle cx="304" cy="108" r="7" fill="hsl(${h2}, 90%, 65%)"/>`,
      `<circle cx="192" cy="186" r="5" fill="hsl(${h2}, 90%, 65%)"/>`,
      `</svg>`,
    ].join("");
  }
  if (scene === "poster") {
    return [
      `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 480 300">`,
      `<rect width="480" height="300" fill="hsl(${h2}, 42%, 18%)"/>`,
      `<rect x="28" y="28" width="424" height="244" rx="20" fill="hsl(${h1}, 48%, 24%)" stroke="hsl(${h3}, 80%, 68%)" stroke-width="4"/>`,
      `<circle cx="145" cy="150" r="74" fill="hsl(${h3}, 78%, 54%)" opacity="0.88"/>`,
      `<polygon points="275,70 405,220 215,220" fill="hsl(${h2}, 82%, 62%)" opacity="0.82"/>`,
      `<rect x="190" y="116" width="160" height="24" rx="12" fill="#ffffff" opacity="0.9"/>`,
      `<rect x="190" y="156" width="110" height="16" rx="8" fill="#ffffff" opacity="0.55"/>`,
      `</svg>`,
    ].join("");
  }
  return [
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 480 300">`,
    `<rect width="480" height="300" fill="hsl(${h1}, 40%, 15%)"/>`,
    `<circle cx="120" cy="95" r="115" fill="hsl(${h1}, 82%, 52%)" opacity="0.65"/>`,
    `<circle cx="350" cy="210" r="135" fill="hsl(${h2}, 82%, 56%)" opacity="0.62"/>`,
    `<circle cx="265" cy="85" r="88" fill="hsl(${h3}, 78%, 50%)" opacity="0.58"/>`,
    `<rect x="44" y="44" width="392" height="212" rx="18" fill="none" stroke="#ffffff" stroke-width="2" opacity="0.35"/>`,
    `<path d="M44 210 Q160 110 260 180 T436 125" fill="none" stroke="#ffffff" stroke-width="5" stroke-linecap="round" opacity="0.85"/>`,
    `</svg>`,
  ].join("");
}
