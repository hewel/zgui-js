import type { CanvasCommand, CanvasLineCap, CanvasLineJoin } from "@hewel/zgui";

export interface ArenaState {
  readonly targetX: number;
  readonly targetY: number;
  readonly puckX: number;
  readonly puckY: number;
  readonly trailX: number;
  readonly trailY: number;
  readonly pulse: number;
  readonly dark: boolean;
}

export function buildArenaCommands(s: ArenaState): CanvasCommand[] {
  const width = 560;
  const height = 300;
  const bg = s.dark ? "#111622" : "#f1f5f9";
  const grid = s.dark ? "#1e2738" : "#dce4ef";
  const cmds: CanvasCommand[] = [
    { kind: "roundedRect", x: 0, y: 0, width, height, radius: 14, fill: bg },
  ];

  for (let x = 40; x < width; x += 40) {
    cmds.push({
      kind: "path",
      segments: [
        { kind: "moveTo", x, y: 8 },
        { kind: "lineTo", x, y: height - 8 },
      ],
      stroke: { color: grid, width: 1 },
    });
  }
  for (let y = 40; y < height; y += 40) {
    cmds.push({
      kind: "path",
      segments: [
        { kind: "moveTo", x: 8, y },
        { kind: "lineTo", x: width - 8, y },
      ],
      stroke: { color: grid, width: 1 },
    });
  }

  // Elastic tether from target to primary puck and trailing ghost
  cmds.push({
    kind: "path",
    segments: [
      { kind: "moveTo", x: s.trailX, y: s.trailY },
      { kind: "quadTo", cx: s.puckX, cy: s.puckY, x: s.targetX, y: s.targetY },
    ],
    stroke: { color: "#60a5fa", width: 2.5, lineCap: "round", lineJoin: "round" },
  });

  // Target crosshair
  cmds.push({
    kind: "circle",
    x: s.targetX,
    y: s.targetY,
    radius: 10,
    stroke: { color: "#f59e0b", width: 2 },
  });
  cmds.push({
    kind: "circle",
    x: s.targetX,
    y: s.targetY,
    radius: 3,
    fill: "#f59e0b",
  });

  // Trailing follower ring
  cmds.push({
    kind: "circle",
    x: s.trailX,
    y: s.trailY,
    radius: 18,
    fill: s.dark ? "#1d4ed8" : "#93c5fd",
    stroke: { color: "#60a5fa", width: 2 },
  });

  // Expanding impulse ring
  if (s.pulse > 1) {
    cmds.push({
      kind: "circle",
      x: s.puckX,
      y: s.puckY,
      radius: 18 + s.pulse,
      stroke: { color: "#a855f7", width: 3 },
    });
  }

  // Primary spring puck
  cmds.push({
    kind: "circle",
    x: s.puckX,
    y: s.puckY,
    radius: 16,
    fill: "#3b82f6",
    stroke: { color: "#ffffff", width: 3 },
  });

  return cmds;
}

export function buildStrokeMatrixCommands(dark: boolean): CanvasCommand[] {
  const bg = dark ? "#141925" : "#f8fafc";
  const cmds: CanvasCommand[] = [
    { kind: "roundedRect", x: 0, y: 0, width: 560, height: 190, radius: 12, fill: bg },
  ];

  const caps: readonly CanvasLineCap[] = ["butt", "round", "square"];
  const joins: readonly CanvasLineJoin[] = ["miter", "round", "bevel"];
  const colors = ["#3b82f6", "#10b981", "#f59e0b"] as const;

  for (let i = 0; i < caps.length; i++) {
    const x0 = 28 + i * 175;
    cmds.push({
      kind: "path",
      segments: [
        { kind: "moveTo", x: x0, y: 78 },
        { kind: "lineTo", x: x0 + 55, y: 26 },
        { kind: "lineTo", x: x0 + 110, y: 78 },
      ],
      stroke: { color: colors[i]!, width: 14, lineCap: caps[i]!, lineJoin: joins[i]! },
    });
  }

  // Cubic Bezier wave with round cap
  cmds.push({
    kind: "path",
    segments: [
      { kind: "moveTo", x: 28, y: 145 },
      { kind: "cubicTo", c1x: 130, c1y: 95, c2x: 220, c2y: 185, x: 320, y: 140 },
      { kind: "cubicTo", c1x: 400, c1y: 105, c2x: 465, c2y: 175, x: 532, y: 132 },
    ],
    stroke: { color: "#ec4899", width: 6, lineCap: "round", lineJoin: "round" },
  });

  return cmds;
}
