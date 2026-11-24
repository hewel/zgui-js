import type { PreviewApplicationOptions } from "./index.ts";
import { randomUUID } from "node:crypto";
import { tmpdir } from "node:os";
import path from "node:path";
import { buildApplication } from "./build.ts";
import { devApplication } from "./dev.ts";
import { resolveNativeHost } from "./host.ts";
import { runNativeHost } from "./native-process.ts";
import { packagedSdkRoot, workspaceRoot } from "./sdk.ts";

export async function previewApplication(
  app: string,
  {
    root = workspaceRoot,
    hostRoot,
    hostExecutable,
    watch = false,
    profile = path.join(root, "preview/profiles/default.json"),
    fixture = path.join(root, "preview/fixtures/empty.json"),
    scenario = path.join(root, "preview/scenarios/counter-capture.json"),
    out = path.join(
      path.resolve(root) === packagedSdkRoot ? tmpdir() : path.join(root, "target"),
      `preview-${app}-${randomUUID()}`,
    ),
    signal,
    onOutput,
    ...buildOptions
  }: PreviewApplicationOptions = {},
): Promise<number> {
  signal?.throwIfAborted();
  const explicitHostRoot = hostRoot === undefined ? undefined : path.resolve(hostRoot);
  const output = path.resolve(out);
  const args = [
    "--profile",
    path.resolve(profile),
    "--fixture",
    path.resolve(fixture),
    "--scenario",
    path.resolve(scenario),
    "--out",
    output,
  ];
  onOutput?.(output);
  if (watch)
    return devApplication(app, {
      ...buildOptions,
      root,
      ...(explicitHostRoot === undefined ? {} : { hostRoot: explicitHostRoot }),
      hostExecutable,
      mode: "preview",
      hostArgs: args,
      signal,
    });
  const { dist } = await buildApplication(app, { ...buildOptions, root });
  signal?.throwIfAborted();
  const resolvedHost = await resolveNativeHost({
    hostExecutable,
    hostRoot: explicitHostRoot,
    root,
    cwd: buildOptions.appDirectory ?? process.cwd(),
    searchFrom: buildOptions.hostSearchFrom,
    signal,
  });
  const runRoot = explicitHostRoot ?? resolvedHost.cargoRoot ?? buildOptions.appDirectory ?? root;
  return runNativeHost(resolvedHost.executable, ["preview", "run", "--bundle", dist, ...args], {
    root: runRoot,
    signal,
  });
}
