import type { CanvasCommand } from "@hewel/zgui";
import type { CpuSample } from "./telemetry.js";

const FULL_ARC = Math.PI * 1.5;
const START_ARC = Math.PI * 0.75;

export function buildGaugeCommands(
  percent: number,
  accentHex: string,
  trackHex: string,
): CanvasCommand[] {
  const clamped = Math.max(0, Math.min(100, percent));
  const sweep = (clamped / 100) * FULL_ARC;
  const commands: CanvasCommand[] = [
    {
      kind: "arc",
      x: 52,
      y: 52,
      radius: 38,
      startAngle: START_ARC,
      sweepAngle: FULL_ARC,
      stroke: { color: trackHex, width: 9, lineCap: "round" },
    },
  ];
  if (sweep > 0.02) {
    commands.push({
      kind: "arc",
      x: 52,
      y: 52,
      radius: 38,
      startAngle: START_ARC,
      sweepAngle: sweep,
      stroke: { color: accentHex, width: 9, lineCap: "round" },
    });
  }
  return commands;
}

export function buildTimelineCommands(
  samples: readonly CpuSample[],
  dark: boolean,
): CanvasCommand[] {
  const width = 620;
  const height = 160;
  const padX = 8;
  const padY = 12;
  const plotW = width - padX * 2;
  const plotH = height - padY * 2;
  const gridColor = dark ? "#2a2f3a" : "#e2e8f0";
  const bgColor = dark ? "#141821" : "#f8fafc";

  const commands: CanvasCommand[] = [
    { kind: "roundedRect", x: 0, y: 0, width, height, radius: 10, fill: bgColor },
  ];

  for (let row = 0; row <= 4; row++) {
    const y = padY + (plotH * row) / 4;
    commands.push({
      kind: "path",
      segments: [
        { kind: "moveTo", x: padX, y },
        { kind: "lineTo", x: width - padX, y },
      ],
      stroke: { color: gridColor, width: 1 },
    });
  }

  if (samples.length < 2) return commands;

  const maxPoints = 60;
  const slice = samples.slice(-maxPoints);
  const stepX = plotW / (maxPoints - 1);
  const offsetX = padX + (maxPoints - slice.length) * stepX;

  const yFor = (pct: number) => padY + plotH - (Math.max(0, Math.min(100, pct)) / 100) * plotH;

  const totalLine: { kind: "moveTo" | "lineTo"; x: number; y: number }[] = [];
  const userSysLine: { kind: "moveTo" | "lineTo"; x: number; y: number }[] = [];
  const sysLine: { kind: "moveTo" | "lineTo"; x: number; y: number }[] = [];

  for (let i = 0; i < slice.length; i++) {
    const s = slice[i]!;
    const x = offsetX + i * stepX;
    const kind = i === 0 ? "moveTo" : "lineTo";
    totalLine.push({ kind, x, y: yFor(s.total) });
    userSysLine.push({ kind, x, y: yFor(s.user + s.sys) });
    sysLine.push({ kind, x, y: yFor(s.sys) });
  }

  const lastX = offsetX + (slice.length - 1) * stepX;
  const baseY = padY + plotH;

  commands.push({
    kind: "path",
    segments: [
      ...totalLine,
      { kind: "lineTo", x: lastX, y: baseY },
      { kind: "lineTo", x: offsetX, y: baseY },
      { kind: "close" },
    ],
    fill: dark ? "#1e293b" : "#dbeafe",
  });

  commands.push({
    kind: "path",
    segments: totalLine,
    stroke: { color: "#f59e0b", width: 2, lineCap: "round", lineJoin: "round" },
  });

  commands.push({
    kind: "path",
    segments: userSysLine,
    stroke: { color: "#3b82f6", width: 2.5, lineCap: "round", lineJoin: "round" },
  });

  commands.push({
    kind: "path",
    segments: sysLine,
    stroke: { color: "#ef4444", width: 2, lineCap: "round", lineJoin: "round" },
  });

  const last = slice[slice.length - 1]!;
  commands.push({
    kind: "circle",
    x: lastX,
    y: yFor(last.user + last.sys),
    radius: 4,
    fill: "#3b82f6",
  });

  return commands;
}
