import {
  buildApplication,
  devApplication,
  packageApplication,
  prepareNativeHost,
  previewApplication,
  probeNativeHost,
  resolveApplication,
  resolveNativeHost,
  runNativeHost,
  syncCssModuleTypes,
  watchApplication,
  type ApplicationWatcher,
  type BuildApplicationOptions,
  type BuildInfo,
  type Capabilities,
  type DevApplicationOptions,
  type DevMode,
  type FontDeclaration,
  type Manifest,
  type NativeHostAbi,
  type NativeHostOptions,
  type PackageApplicationOptions,
  type PackagedApplication,
  type PrepareNativeHostOptions,
  type PreviewApplicationOptions,
  type PublishedApplication,
  type ResolvedApplication,
  type ResolvedNativeHost,
  type ResolveNativeHostOptions,
  type RunNativeHostOptions,
  type SyncCssModuleTypesOptions,
  type WatchApplicationOptions,
} from "@hewel/zgui-build";

export async function buildAndPreview(directory: string, signal: AbortSignal) {
  const selection: ResolvedApplication = await resolveApplication(directory);
  const syncOptions: SyncCssModuleTypesOptions = {
    appDirectory: selection.appDirectory,
    project: `${selection.appDirectory}/tsconfig.json`,
    include: ["src"],
    cwd: selection.root,
  };
  const syncedCssModules: string[] = await syncCssModuleTypes(syncOptions);
  const extraCapabilities: Capabilities = { namedSockets: ["status"] };
  const build: BuildApplicationOptions = {
    ...selection,
    extraCapabilities,
    cssModuleInclude: ["src"],
  };
  const publication: PublishedApplication = await buildApplication(selection.app, build);
  const manifest: Manifest = publication.manifest;
  const fonts: FontDeclaration[] = manifest.fonts;
  const watch: WatchApplicationOptions = {
    ...build,
    signal,
    onSuccess: result => result.manifest.id,
  };
  const watcher: ApplicationWatcher = watchApplication(selection.app, watch);
  await watcher.close();

  const host: NativeHostOptions = { root: selection.root, signal };
  const prepare: PrepareNativeHostOptions = host;
  const executable: string = await prepareNativeHost(prepare);
  const abi: NativeHostAbi = probeNativeHost(executable);
  const resolveOpts: ResolveNativeHostOptions = { hostExecutable: executable, signal };
  const resolvedHost: ResolvedNativeHost = await resolveNativeHost(resolveOpts);
  const packOpts: PackageApplicationOptions = { ...build, hostExecutable: resolvedHost.executable };
  const packaged: PackagedApplication = await packageApplication(selection.app, packOpts);
  const run: RunNativeHostOptions = { signal };
  const mode: DevMode = "desktop";
  const dev: DevApplicationOptions = { ...build, hostExecutable: executable, mode, signal };
  const preview: PreviewApplicationOptions = { ...build, hostExecutable: executable, signal };
  const statuses: number[] = [
    await runNativeHost(executable, ["--bundle", publication.dist], run),
    await devApplication(selection.app, dev),
    await previewApplication(selection.app, preview),
  ];
  return { abi, fonts, packaged, statuses, syncedCssModules };
}

export type BuildProvenance = Pick<BuildInfo, "lockfile" | "cargoLock">;

// Lifecycle and compiler injection points are not consumer API.
export function checkPrivateParameters() {
  // @ts-expect-error The public watch API has no dependency-injection parameter.
  watchApplication("fixture", {}, {});
  // @ts-expect-error The public dev API has no dependency-injection parameter.
  void devApplication("fixture", {}, {});
  // @ts-expect-error The public native host API has no process-injection parameter.
  void prepareNativeHost({}, {});
}
