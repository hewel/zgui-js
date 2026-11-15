#!/usr/bin/env node
import path from "node:path";
import { randomUUID } from "node:crypto";
import { buildApplication, workspaceRoot } from "./build.mjs";
import { resolveApplication } from "./application.mjs";
import { devApplication } from "./dev.mjs";
import { prepareNativeHost, runNativeHost } from "./native-process.mjs";
import { buildSmoke } from "./smoke.mjs";
import { typecheckProject } from "./typecheck.mjs";
import { watchApplication } from "./watch.mjs";

const usage = `Usage: zgui-js <dev|build|preview> [DIRECTORY | --app NAME_OR_DIRECTORY] [options]

  dev       Build the native host, open the app, and reload on changes
  build     Type-check and publish an immutable application package
  preview   Build the app and host, then capture an offscreen native frame

Application defaults to the current directory (counter at the repository root).
  build --watch          Watch packages without starting a host
  dev --exit-after-ms N  Close the native app after N milliseconds
  dev --layer-config F  Use an upstream layer-shell configuration
  preview --watch       Re-run offscreen scenarios on changes, without a desktop host
  preview --profile F --fixture F --scenario F --out NEW_DIRECTORY

Preview defaults to the repository profile, empty fixture, and initial-frame scenario.
Output defaults to a fresh directory under target/. Graphics support is required.
  zgui-js smoke         Build the compiler/runtime smoke fixture`;

function parse(args) {
  const command = args.shift();
  if (!["dev", "build", "preview", "smoke"].includes(command)) throw new Error(usage);
  const flags = new Map();
  const allowed = new Set(["--app",
    ...(command === "build" || command === "preview" ? ["--watch"] : []),
    ...(command === "dev" ? ["--exit-after-ms", "--layer-config"] : []),
    ...(command === "preview" ? ["--profile", "--fixture", "--scenario", "--out"] : []),
  ]);
  let directory;
  while (args.length) {
    const arg = args.shift();
    if (arg === "--") continue;
    if (!arg.startsWith("-")) {
      if (directory !== undefined) throw new Error("Specify only one application directory");
      directory = arg;
      continue;
    }
    if (!allowed.has(arg) || flags.has(arg)) throw new Error(`Unknown or duplicate option: ${arg}`);
    if (arg === "--watch") { flags.set(arg, true); continue; }
    const value = args.shift();
    if (!value || value.startsWith("--")) throw new Error(`${arg} requires a value`);
    flags.set(arg, value);
  }
  if (directory !== undefined && flags.has("--app")) throw new Error("Use DIRECTORY or --app, not both");
  if (command === "smoke" && (directory !== undefined || flags.size)) throw new Error("zgui-js smoke takes no options");
  if (flags.has("--exit-after-ms") && !/^[1-9][0-9]*$/.test(flags.get("--exit-after-ms"))) {
    throw new Error("--exit-after-ms requires a positive integer");
  }
  return { command, selector: directory ?? flags.get("--app"), flags };
}

async function watchBuild(application) {
  const watcher = watchApplication(application.app, application);
  let closing;
  const stop = () => {
    closing ??= watcher.close().finally(() => {
      process.removeListener("SIGINT", stop);
      process.removeListener("SIGTERM", stop);
    });
    closing.catch(error => { console.error(error.message); process.exitCode = 1; });
  };
  process.on("SIGINT", stop);
  process.on("SIGTERM", stop);
}

async function preview(application, flags) {
  const output = path.resolve(flags.get("--out") ?? path.join(workspaceRoot, "target", `preview-${application.app}-${randomUUID()}`));
  const args = [
    "--profile", path.resolve(flags.get("--profile") ?? path.join(workspaceRoot, "preview/profiles/default.json")),
    "--fixture", path.resolve(flags.get("--fixture") ?? path.join(workspaceRoot, "preview/fixtures/empty.json")),
    "--scenario", path.resolve(flags.get("--scenario") ?? path.join(workspaceRoot, "preview/scenarios/counter-capture.json")),
    "--out", output,
  ];
  console.error(`[preview] Artifacts: ${output}`);
  if (flags.has("--watch")) return devApplication(application.app, { ...application, mode: "preview", hostArgs: args });
  const controller = new AbortController();
  let interrupted;
  const interrupt = signal => {
    if (interrupted !== undefined) return;
    interrupted = signal === "SIGINT" ? 130 : 143;
    controller.abort(signal);
  };
  const onInt = () => interrupt("SIGINT");
  const onTerm = () => interrupt("SIGTERM");
  process.on("SIGINT", onInt);
  process.on("SIGTERM", onTerm);
  try {
    const { dist } = await buildApplication(application.app, application);
    controller.signal.throwIfAborted();
    const executable = await prepareNativeHost({ root: application.root, signal: controller.signal });
    return await runNativeHost(executable, ["preview", "run", "--bundle", dist, ...args], {
      root: application.root, signal: controller.signal,
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
    const { command, selector, flags } = parse(args);
    if (command === "smoke") {
      typecheckProject(path.join(workspaceRoot, "packages/build/tsconfig.smoke.json"));
      await buildSmoke();
    } else {
      const application = await resolveApplication(selector);
      if (command === "dev") {
        const hostArgs = [];
        for (const flag of ["--exit-after-ms", "--layer-config"]) {
          if (flags.has(flag)) hostArgs.push(flag, flag === "--layer-config" ? path.resolve(flags.get(flag)) : flags.get(flag));
        }
        process.exitCode = await devApplication(application.app, { ...application, hostArgs });
      } else if (command === "preview") {
        process.exitCode = await preview(application, flags);
      } else if (flags.has("--watch")) {
        await watchBuild(application);
      } else {
        const { dist, manifest } = await buildApplication(application.app, application);
        console.log(`Published ${dist} ${manifest.buildId}`);
      }
    }
  }
} catch (error) {
  console.error(error.message);
  process.exitCode = error.exitCode ?? 1;
}
