import type { PreviewApplicationOptions } from "./index.ts";
import { randomUUID } from "node:crypto";
import { tmpdir } from "node:os";
import path from "node:path";
import { buildWithResolvedHost } from "./build.ts";
import { devApplication } from "./dev.ts";
import { runNativeHost } from "./host.ts";
import { packagedSdkRoot, workspaceRoot } from "./sdk.ts";

export async function previewApplication(
  app: string,
  {
    root = workspaceRoot,
    appDirectory = path.join(root, "apps/examples", app),
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
    bytecode = false,
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
      appDirectory,
      ...(explicitHostRoot === undefined ? {} : { hostRoot: explicitHostRoot }),
      hostExecutable,
      mode: "preview",
      hostArgs: args,
      signal,
    });
  const {
    application: { dist },
    resolveHost,
  } = await buildWithResolvedHost(
    app,
    { ...buildOptions, root, appDirectory, bytecode },
    { hostRoot: explicitHostRoot, hostExecutable, signal },
  );
  const host = await resolveHost();
  return runNativeHost(host.executable, ["preview", "run", "--bundle", dist, ...args], {
    root: host.runRoot,
    signal,
  });
}
