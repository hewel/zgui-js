import {
  buildApplication,
  devApplication,
  prepareNativeHost,
  previewApplication,
  resolveApplication,
  runNativeHost,
  watchApplication,
  type ApplicationWatcher,
  type BuildApplicationOptions,
  type BuildInfo,
  type Capabilities,
  type DevApplicationOptions,
  type DevMode,
  type FontDeclaration,
  type Manifest,
  type NativeHostOptions,
  type PrepareNativeHostOptions,
  type PreviewApplicationOptions,
  type PublishedApplication,
  type ResolvedApplication,
  type RunNativeHostOptions,
  type WatchApplicationOptions,
} from "@hewel/zgui-build";

export async function buildAndPreview(directory: string, signal: AbortSignal) {
  const selection: ResolvedApplication = await resolveApplication(directory);
  const extraCapabilities: Capabilities = { namedSockets: ["status"] };
  const build: BuildApplicationOptions = { ...selection, extraCapabilities };
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
  const run: RunNativeHostOptions = { signal };
  const mode: DevMode = "desktop";
  const dev: DevApplicationOptions = { ...build, hostExecutable: executable, mode, signal };
  const preview: PreviewApplicationOptions = { ...build, hostExecutable: executable, signal };
  const statuses: number[] = [
    await runNativeHost(executable, ["--bundle", publication.dist], run),
    await devApplication(selection.app, dev),
    await previewApplication(selection.app, preview),
  ];
  return { fonts, statuses };
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
