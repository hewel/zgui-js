export interface CpuSample {
  readonly user: number;
  readonly sys: number;
  readonly idle: number;
  readonly wait: number;
  readonly total: number;
  readonly interrupts: number;
  readonly contextSwitches: number;
  readonly runningProcs: number;
  readonly blockedProcs: number;
}

export interface MemoryStats {
  readonly totalMb: number;
  readonly usedMb: number;
  readonly availableMb: number;
  readonly swapTotalMb: number;
  readonly swapUsedMb: number;
}

export interface ProcessRow {
  readonly pid: number;
  readonly cpu: number;
  readonly mem: number;
  readonly rssMb: number;
  readonly stat: string;
  readonly command: string;
}

export interface DiskRow {
  readonly mount: string;
  readonly percent: number;
  readonly used: string;
  readonly size: string;
}

export type ProcessSortKey = "cpu" | "mem" | "rss" | "pid";

export function parseVmstatLine(line: string): CpuSample | null {
  const parts = line.trim().split(/\s+/);
  if (parts.length < 16) return null;
  const nums = parts.slice(0, 16).map(Number);
  if (nums.some(n => !Number.isFinite(n))) return null;
  const user = nums[12]!;
  const sys = nums[13]!;
  const idle = nums[14]!;
  const wait = nums[15]!;
  return {
    runningProcs: nums[0]!,
    blockedProcs: nums[1]!,
    interrupts: nums[10]!,
    contextSwitches: nums[11]!,
    user,
    sys,
    idle,
    wait,
    total: Math.max(0, Math.min(100, 100 - idle)),
  };
}

export function parseFreeOutput(stdout: string): MemoryStats | null {
  let totalMb = 0;
  let usedMb = 0;
  let availableMb = 0;
  let swapTotalMb = 0;
  let swapUsedMb = 0;
  let foundMem = false;
  for (const raw of stdout.split("\n")) {
    const parts = raw.trim().split(/\s+/);
    if (parts[0] === "Mem:" && parts.length >= 7) {
      totalMb = Number(parts[1]) || 0;
      usedMb = Number(parts[2]) || 0;
      availableMb = Number(parts[6]) || 0;
      foundMem = true;
    } else if (parts[0] === "Swap:" && parts.length >= 3) {
      swapTotalMb = Number(parts[1]) || 0;
      swapUsedMb = Number(parts[2]) || 0;
    }
  }
  return foundMem ? { totalMb, usedMb, availableMb, swapTotalMb, swapUsedMb } : null;
}

export function parsePsOutput(stdout: string): ProcessRow[] {
  const rows: ProcessRow[] = [];
  const lines = stdout.split("\n");
  for (let i = 1; i < lines.length; i++) {
    const line = lines[i]?.trim();
    if (!line) continue;
    const match = line.match(/^(\d+)\s+([0-9.]+)\s+([0-9.]+)\s+(\d+)\s+(\S+)\s+(.+)$/);
    if (!match) continue;
    const pid = Number(match[1]);
    const cpu = Number(match[2]);
    const mem = Number(match[3]);
    const rssKb = Number(match[4]);
    if (!Number.isFinite(pid) || !Number.isFinite(cpu) || !Number.isFinite(mem)) continue;
    rows.push({
      pid,
      cpu,
      mem,
      rssMb: Math.round((rssKb / 1024) * 10) / 10,
      stat: match[5]!,
      command: match[6]!,
    });
  }
  return rows;
}

export function parseDfOutput(stdout: string): DiskRow[] {
  const rows: DiskRow[] = [];
  const seen = new Set<string>();
  const lines = stdout.split("\n");
  for (let i = 1; i < lines.length; i++) {
    const parts = lines[i]?.trim().split(/\s+/);
    if (!parts || parts.length < 4) continue;
    const mount = parts[0]!;
    if (seen.has(mount)) continue;
    seen.add(mount);
    const pct = Number(parts[1]!.replace("%", ""));
    rows.push({
      mount,
      percent: Number.isFinite(pct) ? Math.max(0, Math.min(100, pct)) : 0,
      used: parts[2]!,
      size: parts[3]!,
    });
  }
  return rows;
}

export function sortProcesses(
  rows: readonly ProcessRow[],
  sortBy: ProcessSortKey,
  descending: boolean,
): ProcessRow[] {
  const factor = descending ? -1 : 1;
  return [...rows].sort((a, b) => {
    const diff =
      sortBy === "cpu"
        ? a.cpu - b.cpu
        : sortBy === "mem"
          ? a.mem - b.mem
          : sortBy === "rss"
            ? a.rssMb - b.rssMb
            : a.pid - b.pid;
    return diff !== 0 ? diff * factor : a.pid - b.pid;
  });
}

export function formatGiB(mb: number): string {
  return `${(mb / 1024).toFixed(1)} GiB`;
}
