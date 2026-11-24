import { existsSync } from "node:fs";
import path from "node:path";
import {
  buildApplication,
  resolveApplication,
  resolveNativeHost,
  runNativeHost,
} from "@hewel/zgui-build";

export interface RunInspectOptions {
  args: readonly string[];
  cwd?: string;
  hostRoot?: string;
  hostExecutable?: string;
  ui?: boolean;
  signal?: AbortSignal;
}

export type InspectorSource =
  | { readonly mode: "prebuilt"; readonly bundleDir: string }
  | { readonly mode: "checkout"; readonly appDir: string };

export function resolveInspectorSource({
  cliDir = import.meta.dir,
}: {
  cliDir?: string;
} = {}): InspectorSource {
  const resolvedCliDir = path.resolve(cliDir);
  const packagedBundleDir = path.resolve(resolvedCliDir, "../inspector-bundle");
  if (
    existsSync(path.join(packagedBundleDir, "manifest.json")) &&
    existsSync(path.join(packagedBundleDir, "app.mjs"))
  ) {
    return { mode: "prebuilt", bundleDir: packagedBundleDir };
  }

  const checkoutRoot = path.resolve(resolvedCliDir, "../../..");
  const checkoutInspectorDir = path.join(checkoutRoot, "apps/devtools/inspector");
  if (
    existsSync(path.join(checkoutRoot, "Cargo.toml")) &&
    existsSync(path.join(checkoutInspectorDir, "package.json"))
  ) {
    return { mode: "checkout", appDir: checkoutInspectorDir };
  }

  throw new Error(
    `Missing prebuilt Inspector bundle at ${packagedBundleDir}. Reinstall @hewel/zgui-cli or run from a zgui-js checkout.`,
  );
}

function parseUiFlags(
  args: readonly string[],
  cwd: string,
): { socket: string; exitAfterMs?: string; width?: string; height?: string } {
  let socket: string | undefined;
  let exitAfterMs: string | undefined;
  let width: string | undefined;
  let height: string | undefined;
  const remaining = [...args];
  while (remaining.length > 0) {
    const arg = remaining.shift()!;
    if (arg === "--socket") {
      const val = remaining.shift();
      if (!val || val.startsWith("--")) throw new Error("--socket requires a Unix socket path");
      socket = val;
      continue;
    }
    if (arg === "--exit-after-ms") {
      const val = remaining.shift();
      if (!val || !/^[1-9][0-9]*$/.test(val)) {
        throw new Error("--exit-after-ms requires a positive integer");
      }
      exitAfterMs = val;
      continue;
    }
    if (arg === "--width") {
      const val = remaining.shift();
      if (!val || val.startsWith("--")) throw new Error("--width requires a value");
      width = val;
      continue;
    }
    if (arg === "--height") {
      const val = remaining.shift();
      if (!val || val.startsWith("--")) throw new Error("--height requires a value");
      height = val;
      continue;
    }
    if (!arg.startsWith("-") && socket === undefined) {
      socket = arg;
      continue;
    }
    throw new Error(`Unknown option for inspect --ui: ${arg}`);
  }
  if (!socket) {
    throw new Error(
      "zgui-js inspect --ui requires --socket <PATH> pointing to a --dev-inspect socket",
    );
  }
  return {
    socket: path.resolve(cwd, socket),
    exitAfterMs,
    width,
    height,
  };
}

export async function runInspect({
  args,
  cwd = process.cwd(),
  hostRoot,
  hostExecutable,
  ui = false,
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
  const runRoot = hostRoot ? path.resolve(resolvedCwd, hostRoot) : (host.cargoRoot ?? resolvedCwd);
  if (ui) {
    const options = parseUiFlags(args, resolvedCwd);
    const source = resolveInspectorSource();
    let bundleDir: string;
    if (source.mode === "prebuilt") {
      bundleDir = source.bundleDir;
    } else {
      const resolvedApp = await resolveApplication(source.appDir);
      const built = await buildApplication(resolvedApp.app, {
        ...resolvedApp,
        development: false,
      });
      bundleDir = built.dist;
    }
    const hostArgs = [
      "--bundle",
      bundleDir,
      "--title",
      "zgui-js Inspector",
      "--width",
      options.width ?? "980",
      "--height",
      options.height ?? "680",
      "--bind-socket",
      `devInspect=${options.socket}`,
      ...(options.exitAfterMs ? ["--exit-after-ms", options.exitAfterMs] : []),
    ];
    return runNativeHost(host.executable, hostArgs, {
      root: runRoot,
      signal,
    });
  }
  return runNativeHost(host.executable, ["inspect", ...args], {
    root: runRoot,
    signal,
  });
}
