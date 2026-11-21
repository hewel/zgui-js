#!/usr/bin/env bun
import type { PreviewApplicationOptions, ResolvedApplication } from "./index.ts";
import path from "node:path";
import { buildApplication, workspaceRoot } from "./build.ts";
import { resolveApplication } from "./application.ts";
import { devApplication } from "./dev.ts";
import { previewApplication } from "./preview.ts";
import { buildSmoke } from "./smoke.ts";
import { typecheckProject } from "./typecheck.ts";
import { watchApplication } from "./watch.ts";

const usage = `Usage: zgui-js <dev|build|preview> [DIRECTORY | --app NAME_OR_DIRECTORY] [options]

  dev       Build the native host, open the app, and reload on changes
  build     Type-check and publish an immutable application package
  preview   Build the app and host, then capture an offscreen native frame

Application defaults to the current directory (counter at the repository root).
  build --watch          Watch packages without starting a host
  --capabilities-file F   Merge capability declarations into build, dev, or preview
  dev, preview --host-root DIR          Build the host from this checkout and run it there
  dev, preview --host-executable PATH   Run this prebuilt host instead of invoking Cargo
  dev --exit-after-ms N  Close the native app after N milliseconds
  dev --width N          Override initial window width in logical pixels
  dev --height N         Override initial window height in logical pixels
  dev --title TEXT       Override initial window title
  dev --max-js-stack-bytes N  Override the QuickJS stack size limit in bytes
  dev --layer-config F  Use an upstream layer-shell configuration
  dev --pure-shell       Start without a primary window (requires shellSurfaces or sessionLock)
  preview --watch       Re-run offscreen scenarios on changes, without a desktop host
  preview --profile F --fixture F --scenario F --out NEW_DIRECTORY

Preview defaults to the repository profile, empty fixture, and initial-frame scenario.
Output defaults to a fresh directory under target/. Graphics support is required.
  zgui-js smoke         Build the compiler/runtime smoke fixture`;

type PreviewOptions = Pick<
  PreviewApplicationOptions,
  "watch" | "profile" | "fixture" | "scenario" | "out"
>;

type HostOptions = Pick<PreviewApplicationOptions, "hostRoot" | "hostExecutable">;

type Command =
  | { command: "smoke" }
  | {
      command: "build";
      selector?: string;
      watch: boolean;
      capabilitiesFile?: string;
    }
  | ({
      command: "dev";
      selector?: string;
      capabilitiesFile?: string;
      exitAfterMs?: string;
      width?: string;
      height?: string;
      title?: string;
      maxJsStackBytes?: string;
      layerConfig?: string;
      pureShell: boolean;
    } & HostOptions)
  | ({
      command: "preview";
      selector?: string;
      capabilitiesFile?: string;
    } & PreviewOptions &
      HostOptions);

function parse(args: string[]): Command {
  const command = args.shift();
  if (command !== "dev" && command !== "build" && command !== "preview" && command !== "smoke")
    throw new Error(usage);
  const flags = new Map<string, string | true>();
  const allowed = new Set([
    "--app",
    "--capabilities-file",
    ...(command === "build" || command === "preview" ? ["--watch"] : []),
    ...(command === "dev" || command === "preview" ? ["--host-root", "--host-executable"] : []),
    ...(command === "dev"
      ? [
          "--exit-after-ms",
          "--width",
          "--height",
          "--title",
          "--max-js-stack-bytes",
          "--layer-config",
          "--pure-shell",
        ]
      : []),
    ...(command === "preview" ? ["--profile", "--fixture", "--scenario", "--out"] : []),
  ]);
  let directory: string | undefined;
  while (args.length) {
    const arg = args.shift()!;
    if (arg === "--") continue;
    if (!arg.startsWith("-")) {
      if (directory !== undefined) throw new Error("Specify only one application directory");
      directory = arg;
      continue;
    }
    if (!allowed.has(arg) || flags.has(arg)) throw new Error(`Unknown or duplicate option: ${arg}`);
    if (arg === "--watch" || arg === "--pure-shell") {
      flags.set(arg, true);
      continue;
    }
    const value = args.shift();
    if (!value || value.startsWith("--")) throw new Error(`${arg} requires a value`);
    flags.set(arg, value);
  }
  if (directory !== undefined && flags.has("--app"))
    throw new Error("Use DIRECTORY or --app, not both");
  if (command === "smoke" && (directory !== undefined || flags.size))
    throw new Error("zgui-js smoke takes no options");
  const value = (flag: string): string | undefined => {
    const selected = flags.get(flag);
    return typeof selected === "string" ? selected : undefined;
  };
  const exitAfterMs = value("--exit-after-ms");
  if (exitAfterMs !== undefined && !/^[1-9][0-9]*$/.test(exitAfterMs)) {
    throw new Error("--exit-after-ms requires a positive integer");
  }
  const maxJsStackBytes = value("--max-js-stack-bytes");
  if (maxJsStackBytes !== undefined && !/^[1-9][0-9]*$/.test(maxJsStackBytes)) {
    throw new Error("--max-js-stack-bytes requires a positive integer");
  }
  for (const dim of ["--width", "--height"] as const) {
    const raw = value(dim);
    if (raw !== undefined) {
      const parsed = Number(raw);
      if (!Number.isFinite(parsed) || parsed <= 0 || parsed > 16384) {
        throw new Error(`${dim} requires a positive finite number <= 16384`);
      }
    }
  }
  const selector = directory ?? value("--app");
  const capabilitiesFile = value("--capabilities-file");
  const host = {
    hostRoot: value("--host-root"),
    hostExecutable: value("--host-executable"),
  };
  switch (command) {
    case "smoke":
      return { command };
    case "build":
      return {
        command,
        selector,
        capabilitiesFile,
        watch: flags.has("--watch"),
      };
    case "dev":
      return {
        command,
        selector,
        capabilitiesFile,
        ...host,
        exitAfterMs,
        width: value("--width"),
        height: value("--height"),
        title: value("--title"),
        maxJsStackBytes,
        layerConfig: value("--layer-config"),
        pureShell: flags.has("--pure-shell"),
      };
    case "preview":
      return {
        command,
        selector,
        capabilitiesFile,
        ...host,
        watch: flags.has("--watch"),
        profile: value("--profile"),
        fixture: value("--fixture"),
        scenario: value("--scenario"),
        out: value("--out"),
      };
  }
}

async function watchBuild(application: ResolvedApplication & { capabilitiesFile?: string }) {
  const watcher = watchApplication(application.app, application);
  let closing: Promise<void> | undefined;
  const stop = () => {
    closing ??= watcher.close().finally(() => {
      process.removeListener("SIGINT", stop);
      process.removeListener("SIGTERM", stop);
    });
    closing.catch((error: unknown) => {
      console.error(error instanceof Error ? error.message : String(error));
      process.exitCode = 1;
    });
  };
  process.on("SIGINT", stop);
  process.on("SIGTERM", stop);
}

async function withInterrupt(run: (signal: AbortSignal) => Promise<number>): Promise<number> {
  const controller = new AbortController();
  let interrupted: number | undefined;
  const interrupt = (signal: "SIGINT" | "SIGTERM") => {
    if (interrupted !== undefined) return;
    interrupted = signal === "SIGINT" ? 130 : 143;
    controller.abort(signal);
  };
  const onInt = () => interrupt("SIGINT");
  const onTerm = () => interrupt("SIGTERM");
  process.on("SIGINT", onInt);
  process.on("SIGTERM", onTerm);
  try {
    return await run(controller.signal);
  } catch (error) {
    if (!controller.signal.aborted) throw error;
    return interrupted ?? 1;
  } finally {
    process.removeListener("SIGINT", onInt);
    process.removeListener("SIGTERM", onTerm);
  }
}

try {
  const args = process.argv.slice(2);
  if (!args.length || args.includes("--help") || args.includes("-h")) {
    console.log(usage);
  } else {
    const selected = parse(args);
    if (selected.command === "smoke") {
      typecheckProject(path.join(workspaceRoot, "packages/build/tsconfig.smoke.json"));
      await buildSmoke();
    } else {
      const application = {
        ...(await resolveApplication(selected.selector)),
        capabilitiesFile: selected.capabilitiesFile,
      };
      if (selected.command === "dev") {
        const hostArgs: string[] = [];
        if (selected.exitAfterMs !== undefined)
          hostArgs.push("--exit-after-ms", selected.exitAfterMs);
        if (selected.width !== undefined) hostArgs.push("--width", selected.width);
        if (selected.height !== undefined) hostArgs.push("--height", selected.height);
        if (selected.title !== undefined) hostArgs.push("--title", selected.title);
        if (selected.maxJsStackBytes !== undefined)
          hostArgs.push("--max-js-stack-bytes", selected.maxJsStackBytes);
        if (selected.layerConfig !== undefined)
          hostArgs.push("--layer-config", path.resolve(selected.layerConfig));
        if (selected.pureShell) hostArgs.push("--pure-shell");
        process.exitCode = await withInterrupt(signal =>
          devApplication(application.app, {
            ...application,
            hostRoot: selected.hostRoot,
            hostExecutable: selected.hostExecutable,
            hostArgs,
            signal,
          }),
        );
      } else if (selected.command === "preview") {
        process.exitCode = await withInterrupt(signal =>
          previewApplication(application.app, {
            ...application,
            ...selected,
            signal,
            onOutput: output => console.error(`[preview] Artifacts: ${output}`),
          }),
        );
      } else {
        if (selected.watch) {
          await watchBuild(application);
        } else {
          const { dist, manifest } = await buildApplication(application.app, application);
          console.log(`Published ${dist} ${manifest.buildId}`);
        }
      }
    }
  }
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode =
    typeof error === "object" &&
    error !== null &&
    "exitCode" in error &&
    typeof error.exitCode === "number"
      ? error.exitCode
      : 1;
}
