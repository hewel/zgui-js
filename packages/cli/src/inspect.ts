import path from "node:path";
import { resolveNativeHost, runNativeHost } from "@hewel/zgui-build";

export interface RunInspectOptions {
  args: readonly string[];
  cwd?: string;
  hostRoot?: string;
  hostExecutable?: string;
  signal?: AbortSignal;
}

export async function runInspect({
  args,
  cwd = process.cwd(),
  hostRoot,
  hostExecutable,
  signal,
}: RunInspectOptions): Promise<number> {
  const resolvedCwd = path.resolve(cwd);
  const host = await resolveNativeHost({
    hostExecutable,
    hostRoot,
    cwd: resolvedCwd,
    searchFrom: [import.meta.url],
    signal,
  });
  return runNativeHost(host.executable, ["inspect", ...args], {
    root: hostRoot ? path.resolve(resolvedCwd, hostRoot) : (host.cargoRoot ?? resolvedCwd),
    signal,
  });
}
