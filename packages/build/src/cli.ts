#!/usr/bin/env bun
import path from "node:path";
import { randomUUID } from "node:crypto";
import { buildApplication, workspaceRoot } from "./build.ts";
import { resolveApplication, type ResolvedApplication } from "./application.ts";
import { devApplication } from "./dev.ts";
import { prepareNativeHost, runNativeHost } from "./native-process.ts";
import { buildSmoke } from "./smoke.ts";
import { typecheckProject } from "./typecheck.ts";
import { watchApplication } from "./watch.ts";

const usage = `Usage: zgui-js <dev|build|preview> [DIRECTORY | --app NAME_OR_DIRECTORY] [options]

  dev       Build the native host, open the app, and reload on changes
  build     Type-check and publish an immutable application package
  preview   Build the app and host, then capture an offscreen native frame

Application defaults to the current directory (counter at the repository root).
  build --watch          Watch packages without starting a host
  build --stylex-diagnostics  Include StyleX source metadata (Preview always includes it)
  dev --exit-after-ms N  Close the native app after N milliseconds
  dev --layer-config F  Use an upstream layer-shell configuration
  preview --watch       Re-run offscreen scenarios on changes, without a desktop host
  preview --profile F --fixture F --scenario F --out NEW_DIRECTORY

Preview defaults to the repository profile, empty fixture, and initial-frame scenario.
Output defaults to a fresh directory under target/. Graphics support is required.
  zgui-js smoke         Build the compiler/runtime smoke fixture`;

type PreviewOptions = {
  watch: boolean;
  profile?: string;
  fixture?: string;
  scenario?: string;
  out?: string;
};

type Command =
  | { command: "smoke" }
  | { command: "build"; selector?: string; watch: boolean; stylexDiagnostics: boolean }
  | { command: "dev"; selector?: string; exitAfterMs?: string; layerConfig?: string }
  | ({ command: "preview"; selector?: string } & PreviewOptions);

function parse(args: string[]): Command {
  const command = args.shift();
  if (command !== "dev" && command !== "build" && command !== "preview" && command !== "smoke")
    throw new Error(usage);
  const flags = new Map<string, string | true>();
  const allowed = new Set([
    "--app",
    ...(command === "build" ? ["--stylex-diagnostics"] : []),
    ...(command === "build" || command === "preview" ? ["--watch"] : []),
    ...(command === "dev" ? ["--exit-after-ms", "--layer-config"] : []),
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
    if (arg === "--watch" || arg === "--stylex-diagnostics") {
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
  const selector = directory ?? value("--app");
  switch (command) {
    case "smoke":
      return { command };
    case "build":
      return {
        command,
        selector,
        watch: flags.has("--watch"),
        stylexDiagnostics: flags.has("--stylex-diagnostics"),
      };
    case "dev":
      return { command, selector, exitAfterMs, layerConfig: value("--layer-config") };
    case "preview":
      return {
        command,
        selector,
        watch: flags.has("--watch"),
        profile: value("--profile"),
        fixture: value("--fixture"),
        scenario: value("--scenario"),
        out: value("--out"),
      };
  }
}

async function watchBuild(application: ResolvedApplication & { stylexDiagnostics: boolean }) {
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

async function preview(
  application: ResolvedApplication,
  options: PreviewOptions,
): Promise<number | undefined> {
  const output = path.resolve(
    options.out ?? path.join(workspaceRoot, "target", `preview-${application.app}-${randomUUID()}`),
  );
  const args = [
    "--profile",
    path.resolve(options.profile ?? path.join(workspaceRoot, "preview/profiles/default.json")),
    "--fixture",
    path.resolve(options.fixture ?? path.join(workspaceRoot, "preview/fixtures/empty.json")),
    "--scenario",
    path.resolve(
      options.scenario ?? path.join(workspaceRoot, "preview/scenarios/counter-capture.json"),
    ),
    "--out",
    output,
  ];
  console.error(`[preview] Artifacts: ${output}`);
  if (options.watch)
    return devApplication(application.app, {
      ...application,
      mode: "preview",
      stylexDiagnostics: true,
      hostArgs: args,
    });
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
    const { dist } = await buildApplication(application.app, {
      ...application,
      stylexDiagnostics: true,
    });
    controller.signal.throwIfAborted();
    const executable = await prepareNativeHost({
      root: application.root,
      signal: controller.signal,
    });
    return await runNativeHost(executable, ["preview", "run", "--bundle", dist, ...args], {
      root: application.root,
      signal: controller.signal,
    });
  } catch (error) {
    if (!controller.signal.aborted) throw error;
    return interrupted;
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
        stylexDiagnostics: selected.command === "build" && selected.stylexDiagnostics,
      };
      if (selected.command === "dev") {
        const hostArgs: string[] = [];
        if (selected.exitAfterMs !== undefined)
          hostArgs.push("--exit-after-ms", selected.exitAfterMs);
        if (selected.layerConfig !== undefined)
          hostArgs.push("--layer-config", path.resolve(selected.layerConfig));
        process.exitCode = await devApplication(application.app, { ...application, hostArgs });
      } else if (selected.command === "preview") {
        process.exitCode = await preview(application, selected);
      } else if (selected.watch) {
        await watchBuild(application);
      } else {
        const { dist, manifest } = await buildApplication(application.app, application);
        console.log(`Published ${dist} ${manifest.buildId}`);
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
