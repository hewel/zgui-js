#!/usr/bin/env bun
import {
  buildApplication,
  devApplication,
  packageApplication,
  previewApplication,
  resolveApplication,
  syncCssModuleTypes,
  watchApplication,
  type PreviewApplicationOptions,
  type ResolvedApplication,
} from "@hewel/zgui-build";
import { buildSmoke, resolveSmokeTsconfigPath, typecheckProject } from "@hewel/zgui-build/internal";
import { existsSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { formatDoctorReport, runDoctor } from "./doctor.ts";
import { runInitCli } from "./init.ts";
import { runInspect } from "./inspect.ts";

const usage = `Usage: zgui-js <command> [options]

Commands:
  init       Scaffold a new zgui-js app (desktop-app or wayland-shell)
  dev        Resolve/build zgui-host, open the app, and reload on changes
  build      Type-check and publish an immutable application package
  typecheck  Sync *.module.css.d.ts declarations and run tsrs --noEmit
  preview    Build the app and host, then capture an offscreen native frame
  package    Assemble a self-contained runnable Linux app directory
  doctor     Verify Bun, zgui-host ABI, Wayland, Vulkan, portal, tsrs, and oxlint
  inspect    Query --dev-inspect socket, toggle visual picker, or open --ui Inspector window
  smoke      Build the compiler/runtime smoke fixture

Application defaults to the current directory (counter at the repository root).
Relative file and directory flags are resolved from the current working directory.
  build --watch          Watch packages without starting a host
  build --bytecode       Compile entry, chunks, and workers to QuickJS bytecode (.qjsb)
  typecheck [DIR] [--project FILE] [--include GLOB] [--types-only]
  --capabilities-file F   Merge capability declarations into build, dev, preview, or package
  --host-root DIR        Build the host from this Cargo checkout and run it there
  --host-executable PATH Run this prebuilt host instead of resolving packages/Cargo
  dev --exit-after-ms N  Close the native app after N milliseconds
  dev --width N          Override initial window width in logical pixels
  dev --height N         Override initial window height in logical pixels
  dev --title TEXT       Override initial window title
  dev --max-js-stack-bytes N  Override the QuickJS stack size limit in bytes
  dev --layer-config F   Use an upstream layer-shell configuration
  dev --pure-shell       Start without a primary window (requires shellSurfaces or sessionLock)
  dev --dev-inspect PATH Enable dev-only read-only inspector Unix socket
  dev --strict-typecheck Gate dev publication on tsrs before reloading
  preview --watch        Re-run offscreen scenarios on changes, without a desktop host
  preview --profile F --fixture F --scenario F --out NEW_DIRECTORY
  package --out DIR [--bytecode] [--desktop-entry] [--pure-shell]
  doctor [--json] [--host-root DIR] [--host-executable PATH]
  inspect [--ui] [--host-root DIR] [--host-executable PATH] [--] <zgui-host inspect args...>
  init [DIR] [--template desktop-app|wayland-shell] [--name NAME] [--force]`;

type PreviewOptions = Pick<
  PreviewApplicationOptions,
  "watch" | "profile" | "fixture" | "scenario" | "out"
>;

type HostOptions = Pick<PreviewApplicationOptions, "hostRoot" | "hostExecutable">;

type Command =
  | { command: "smoke" }
  | { command: "init"; args: string[] }
  | {
      command: "typecheck";
      help: boolean;
      selector?: string;
      project?: string;
      include: string[];
      typesOnly: boolean;
    }
  | ({ command: "doctor"; json: boolean } & HostOptions)
  | ({ command: "inspect"; args: string[]; ui: boolean } & HostOptions)
  | ({
      command: "build";
      selector?: string;
      watch: boolean;
      bytecode: boolean;
      capabilitiesFile?: string;
    } & HostOptions)
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
      devInspect?: string;
      pureShell: boolean;
      strictTypecheck: boolean;
    } & HostOptions)
  | ({
      command: "preview";
      selector?: string;
      capabilitiesFile?: string;
    } & PreviewOptions &
      HostOptions)
  | ({
      command: "package";
      selector?: string;
      capabilitiesFile?: string;
      out?: string;
      bytecode: boolean;
      desktopEntry: boolean;
      pureShell: boolean;
    } & HostOptions);

function parseInspect(args: string[]): Extract<Command, { command: "inspect" }> {
  let hostRoot: string | undefined;
  let hostExecutable: string | undefined;
  let ui = false;
  const forwarded: string[] = [];
  let passthroughAll = false;
  while (args.length > 0) {
    const arg = args.shift()!;
    if (passthroughAll) {
      forwarded.push(arg);
      continue;
    }
    if (arg === "--") {
      passthroughAll = true;
      continue;
    }
    if (arg === "--ui") {
      ui = true;
      continue;
    }
    if (arg === "--host-root") {
      const val = args.shift();
      if (!val || val.startsWith("--")) throw new Error("--host-root requires a value");
      hostRoot = path.resolve(val);
      continue;
    }
    if (arg === "--host-executable") {
      const val = args.shift();
      if (!val || val.startsWith("--")) throw new Error("--host-executable requires a value");
      hostExecutable = path.resolve(val);
      continue;
    }
    forwarded.push(arg);
  }
  return { command: "inspect", args: forwarded, ui, hostRoot, hostExecutable };
}

const typecheckUsage = `Usage: zgui-js typecheck [DIR] [options]

Options:
  --app <NAME>         Select a workspace example or app directory
  -p, --project <FILE> Path to tsconfig.json (or directory containing one)
  --include <GLOB>     Additional CSS Modules directory or glob (repeatable)
  --types-only         Sync *.module.css.d.ts declarations without running tsrs
  -h, --help           Show this help message`;

function parseTypecheck(args: string[]): Extract<Command, { command: "typecheck" }> {
  if (args.includes("-h") || args.includes("--help")) {
    return { command: "typecheck", help: true, include: [], typesOnly: false };
  }
  let directory: string | undefined;
  let app: string | undefined;
  let project: string | undefined;
  const include: string[] = [];
  let typesOnly = false;
  while (args.length > 0) {
    const arg = args.shift()!;
    if (arg === "--") continue;
    if (!arg.startsWith("-")) {
      if (directory !== undefined) throw new Error("Specify only one application directory");
      directory = arg;
      continue;
    }
    if (arg === "--types-only") {
      if (typesOnly) throw new Error(`Unknown or duplicate option: ${arg}`);
      typesOnly = true;
      continue;
    }
    if (arg === "--app") {
      if (app !== undefined) throw new Error(`Unknown or duplicate option: ${arg}`);
      const val = args.shift();
      if (!val || val.startsWith("-")) throw new Error("--app requires a value");
      app = val;
      continue;
    }
    if (arg === "--project" || arg === "-p") {
      if (project !== undefined) throw new Error(`Unknown or duplicate option: ${arg}`);
      const val = args.shift();
      if (!val || val.startsWith("-")) throw new Error(`${arg} requires a value`);
      project = val;
      continue;
    }
    if (arg === "--include") {
      const val = args.shift();
      if (!val || val.startsWith("-")) throw new Error("--include requires a value");
      include.push(val);
      continue;
    }
    throw new Error(`Unknown or duplicate option: ${arg}`);
  }
  if (directory !== undefined && app !== undefined) {
    throw new Error("Use DIRECTORY or --app, not both");
  }
  const selector = directory ?? app;
  if (selector !== undefined && project !== undefined) {
    throw new Error("Use --project or an application selector, not both");
  }
  return {
    command: "typecheck",
    help: false,
    ...(selector !== undefined ? { selector } : {}),
    ...(project !== undefined ? { project } : {}),
    include,
    typesOnly,
  };
}

function parse(args: string[]): Command {
  const command = args.shift();
  if (command === "init") {
    return { command: "init", args };
  }
  if (command === "typecheck") {
    return parseTypecheck(args);
  }
  if (command === "inspect") {
    return parseInspect(args);
  }
  if (
    command !== "dev" &&
    command !== "build" &&
    command !== "preview" &&
    command !== "package" &&
    command !== "doctor" &&
    command !== "smoke"
  ) {
    throw new Error(usage);
  }
  const flags = new Map<string, string | true>();
  const allowed = new Set([
    ...(command === "doctor" || command === "smoke" ? [] : ["--app", "--capabilities-file"]),
    ...(command === "build" || command === "preview" ? ["--watch"] : []),
    ...(command === "build" || command === "package" ? ["--bytecode"] : []),
    ...(command === "build" ||
    command === "dev" ||
    command === "preview" ||
    command === "package" ||
    command === "doctor"
      ? ["--host-root", "--host-executable"]
      : []),
    ...(command === "doctor" ? ["--json"] : []),
    ...(command === "dev"
      ? [
          "--exit-after-ms",
          "--width",
          "--height",
          "--title",
          "--max-js-stack-bytes",
          "--layer-config",
          "--dev-inspect",
          "--pure-shell",
          "--strict-typecheck",
        ]
      : []),
    ...(command === "preview" ? ["--profile", "--fixture", "--scenario", "--out"] : []),
    ...(command === "package" ? ["--out", "--desktop-entry", "--pure-shell"] : []),
  ]);
  const booleanFlags = new Set([
    "--watch",
    "--bytecode",
    "--pure-shell",
    "--desktop-entry",
    "--json",
    "--strict-typecheck",
  ]);
  let directory: string | undefined;
  while (args.length) {
    const arg = args.shift()!;
    if (arg === "--") continue;
    if (!arg.startsWith("-")) {
      if (command === "doctor" || command === "smoke") {
        throw new Error(`zgui-js ${command} does not accept positional arguments`);
      }
      if (directory !== undefined) throw new Error("Specify only one application directory");
      directory = arg;
      continue;
    }
    if (!allowed.has(arg) || flags.has(arg)) throw new Error(`Unknown or duplicate option: ${arg}`);
    if (booleanFlags.has(arg)) {
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
  const pathValue = (flag: string): string | undefined => {
    const selected = value(flag);
    return selected === undefined ? undefined : path.resolve(selected);
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
  const capabilitiesFile = pathValue("--capabilities-file");
  const host = {
    hostRoot: pathValue("--host-root"),
    hostExecutable: pathValue("--host-executable"),
  };
  switch (command) {
    case "smoke":
      return { command };
    case "doctor":
      return { command, ...host, json: flags.has("--json") };
    case "build":
      return {
        command,
        selector,
        capabilitiesFile,
        ...host,
        watch: flags.has("--watch"),
        bytecode: flags.has("--bytecode"),
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
        layerConfig: pathValue("--layer-config"),
        devInspect: pathValue("--dev-inspect"),
        pureShell: flags.has("--pure-shell"),
        strictTypecheck: flags.has("--strict-typecheck"),
      };
    case "preview":
      return {
        command,
        selector,
        capabilitiesFile,
        ...host,
        watch: flags.has("--watch"),
        profile: pathValue("--profile"),
        fixture: pathValue("--fixture"),
        scenario: pathValue("--scenario"),
        out: pathValue("--out"),
      };
    case "package":
      return {
        command,
        selector,
        capabilitiesFile,
        ...host,
        out: pathValue("--out"),
        bytecode: flags.has("--bytecode"),
        desktopEntry: flags.has("--desktop-entry"),
        pureShell: flags.has("--pure-shell"),
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

function cliVersion(): string {
  const pkg = JSON.parse(
    readFileSync(path.resolve(import.meta.dir, "../package.json"), "utf8"),
  ) as { version?: string };
  return pkg.version ?? "0.1.0";
}

export async function runCli(rawArgs: readonly string[]): Promise<number> {
  const args = [...rawArgs];
  if (!args.length || args[0] === "--help" || args[0] === "-h") {
    console.log(usage);
    return 0;
  }
  if (args[0] === "--version" || args[0] === "-V") {
    console.log(`zgui-js ${cliVersion()}`);
    return 0;
  }
  const selected = parse(args);
  if (selected.command === "init") {
    return runInitCli(selected.args);
  }
  if (selected.command === "doctor") {
    const report = await runDoctor({
      hostRoot: selected.hostRoot,
      hostExecutable: selected.hostExecutable,
    });
    if (selected.json) {
      console.log(JSON.stringify(report, null, 2));
    } else {
      console.log(formatDoctorReport(report));
    }
    return report.ok ? 0 : 1;
  }
  if (selected.command === "inspect") {
    return withInterrupt(signal =>
      runInspect({
        args: selected.args,
        ui: selected.ui,
        hostRoot: selected.hostRoot,
        hostExecutable: selected.hostExecutable,
        signal,
      }),
    );
  }
  if (selected.command === "smoke") {
    typecheckProject(resolveSmokeTsconfigPath());
    await buildSmoke();
    return 0;
  }
  if (selected.command === "typecheck") {
    if (selected.help) {
      console.log(typecheckUsage);
      return 0;
    }
    if (
      selected.typesOnly &&
      selected.project === undefined &&
      selected.selector === undefined &&
      !existsSync(path.resolve("tsconfig.json")) &&
      selected.include.length > 0
    ) {
      await syncCssModuleTypes({ include: selected.include, cwd: process.cwd() });
      return 0;
    }
    let projectPath: string;
    let appDirectory: string | undefined;
    if (selected.project !== undefined) {
      let candidate = path.resolve(selected.project);
      if (existsSync(candidate) && statSync(candidate).isDirectory()) {
        candidate = path.join(candidate, "tsconfig.json");
      }
      if (!existsSync(candidate)) {
        throw new Error(`tsconfig.json not found: ${candidate}`);
      }
      projectPath = candidate;
    } else if (selected.selector !== undefined) {
      const direct = path.resolve(selected.selector);
      const directTsconfig = path.join(direct, "tsconfig.json");
      const isExplicitDir =
        path.isAbsolute(selected.selector) ||
        selected.selector.includes("/") ||
        selected.selector.includes("\\") ||
        selected.selector === "." ||
        selected.selector === "..";
      if ((isExplicitDir || existsSync(direct)) && existsSync(directTsconfig)) {
        projectPath = directTsconfig;
        if (existsSync(path.join(direct, "src"))) {
          appDirectory = direct;
        }
      } else {
        const resolved = await resolveApplication(selected.selector);
        appDirectory = resolved.appDirectory;
        projectPath = path.join(resolved.appDirectory, "tsconfig.json");
      }
    } else if (existsSync(path.resolve("tsconfig.json"))) {
      projectPath = path.resolve("tsconfig.json");
      if (existsSync(path.resolve("src"))) {
        appDirectory = process.cwd();
      }
    } else {
      const resolved = await resolveApplication(undefined);
      appDirectory = resolved.appDirectory;
      projectPath = path.join(resolved.appDirectory, "tsconfig.json");
    }

    await syncCssModuleTypes({
      project: projectPath,
      ...(appDirectory !== undefined ? { appDirectory } : {}),
      ...(selected.include.length > 0 ? { include: selected.include } : {}),
      cwd: process.cwd(),
    });
    if (!selected.typesOnly) {
      typecheckProject(projectPath);
    }
    return 0;
  }

  const hostSearchFrom = [import.meta.url];
  const host = {
    hostRoot: selected.hostRoot,
    hostExecutable: selected.hostExecutable,
    hostSearchFrom,
  };
  const application = {
    ...(await resolveApplication(selected.selector)),
    capabilitiesFile: selected.capabilitiesFile,
  };
  if (selected.command === "dev") {
    const hostArgs: string[] = [];
    if (selected.exitAfterMs !== undefined) hostArgs.push("--exit-after-ms", selected.exitAfterMs);
    if (selected.width !== undefined) hostArgs.push("--width", selected.width);
    if (selected.height !== undefined) hostArgs.push("--height", selected.height);
    if (selected.title !== undefined) hostArgs.push("--title", selected.title);
    if (selected.maxJsStackBytes !== undefined)
      hostArgs.push("--max-js-stack-bytes", selected.maxJsStackBytes);
    if (selected.layerConfig !== undefined) hostArgs.push("--layer-config", selected.layerConfig);
    if (selected.devInspect !== undefined) hostArgs.push("--dev-inspect", selected.devInspect);
    if (selected.pureShell) hostArgs.push("--pure-shell");
    return withInterrupt(signal =>
      devApplication(application.app, {
        ...application,
        strictTypecheck: selected.strictTypecheck,
        ...host,
        hostArgs,
        signal,
      }),
    );
  }
  if (selected.command === "preview") {
    return withInterrupt(signal =>
      previewApplication(application.app, {
        ...application,
        ...selected,
        ...host,
        signal,
        onOutput: output => console.error(`[preview] Artifacts: ${output}`),
      }),
    );
  }
  if (selected.command === "package") {
    const packaged = await withInterrupt(async signal => {
      const result = await packageApplication(application.app, {
        ...application,
        out: selected.out,
        bytecode: selected.bytecode,
        ...host,
        desktopEntry: selected.desktopEntry,
        pureShell: selected.pureShell,
        signal,
      });
      console.log(
        `Packaged ${result.out} (${result.manifest.buildId}, host=${result.host.source})`,
      );
      return 0;
    });
    return packaged;
  }
  if (selected.watch) {
    if (selected.bytecode) {
      throw new Error("zgui-js build --watch cannot be combined with --bytecode");
    }
    await watchBuild(application);
    return 0;
  }
  const { dist, manifest } = await buildApplication(application.app, {
    ...application,
    bytecode: selected.bytecode,
    ...host,
  });
  console.log(`Published ${dist} ${manifest.buildId}`);
  return 0;
}

if (import.meta.main) {
  try {
    const code = await runCli(process.argv.slice(2));
    if (code !== 0) process.exitCode = code;
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
}
