export type ClipboardCapabilities = {
  read?: boolean;
};

export type SqliteCapabilities = {
  databases: string[];
};

export type FileDirectoryGrant = {
  path: string;
  mode: "read" | "readwrite";
};

export type FilesCapabilities = {
  directories?: FileDirectoryGrant[];
  /** Defaults to true for legacy gesture-authorized bookmarks. */
  allowBookmarks?: boolean;
  /** Prevent the persistence or restoration of readwrite workspace grants. */
  readOnlyGrants?: boolean;
};

export type NetworkOriginRule = {
  origin: string;
  allowInsecureHttp?: boolean;
  allowPrivateNetworks?: boolean;
};

export type NetworkCapabilities = {
  origins?: string[];
  originRules?: NetworkOriginRule[];
  allowInsecureHttp?: boolean;
  allowPrivateNetworks?: boolean;
};

export type Capabilities = {
  shellSurfaces?: boolean;
  sessionLock?: boolean;
  imagePrefixes?: string[];
  unixSockets?: string[];
  namedSockets?: string[];
  commands?: string[];
  controlChannel?: boolean;
  /** Base package launch policy; overlays cannot enable this. */
  singleInstance?: boolean;
  clipboard?: ClipboardCapabilities;
  sqlite?: SqliteCapabilities;
  files?: FilesCapabilities;
  network?: NetworkCapabilities;
};

export type FontDeclaration = { family: string; path: string };
export type ChunkDeclaration = { path: string; sourceMap: string };
export type WorkerDeclaration = { name: string; path: string; sourceMap: string };
export type WindowDeclaration = {
  title?: string;
  width?: number;
  height?: number;
  minWidth?: number;
  minHeight?: number;
  resizable?: boolean;
};
export type ResolvedApplication = { app: string; appDirectory: string; root: string };
export type PublishedApplication = { dist: string; manifest: Manifest };

export type Manifest = {
  schemaVersion: 2;
  id: string;
  hostApiVersion: 1;
  entry: string;
  sourceMap: string;
  bytecodeFormat?: "qjs-bytecode-v1";
  quickjsAbi?: string;
  chunks?: ChunkDeclaration[];
  workers?: WorkerDeclaration[];
  styles: string[];
  assets: string[];
  fonts: FontDeclaration[];
  window?: WindowDeclaration;
  capabilities?: Capabilities;
  styleSystem: {
    name: "lightningcss";
    profile: "lightningcss-v1";
    configDigest: string;
  };
  artifactDigests: Record<string, string>;
  buildId: string;
};

export type BuildInfo = {
  schemaVersion: 2;
  hostApiVersion: 1;
  bun: string;
  packageManager: string;
  packages: Record<string, string>;
  lockfile: { path: "bun.lock"; digest: string };
  cargoLock: { path: "Cargo.lock"; digest: string };
  engine: {
    name: "QuickJS-NG";
    revision: string;
    rquickjsVersion: string;
    rquickjsRevision: string;
  };
  conditions: string[];
  target: "es2022";
  solidRuntime: string;
  graph: {
    modules: string[];
    chunks: {
      fileName: string;
      modules: string[];
      imports: string[];
      dynamicImports: string[];
      exports: string[];
    }[];
  };
  styleSystem: Manifest["styleSystem"];
};

export interface BuildApplicationOptions {
  root?: string;
  appDirectory?: string;
  development?: boolean;
  bytecode?: boolean;
  hostRoot?: string;
  hostExecutable?: string;
  hostSearchFrom?: readonly string[] | undefined;
  extraCapabilities?: Capabilities;
  capabilitiesFile?: string;
  cssModuleInclude?: readonly string[] | undefined;
}

export interface SyncCssModuleTypesOptions {
  project?: string | undefined;
  appDirectory?: string | undefined;
  include?: readonly string[] | undefined;
  cwd?: string | undefined;
}

export interface ApplicationWatcher {
  close(): Promise<void>;
}

export type TypecheckEvent =
  | { status: "passed"; attemptId: string }
  | { status: "failed"; attemptId: string; diagnostics: string };

export interface WatchApplicationOptions extends BuildApplicationOptions {
  strictTypecheck?: boolean | undefined;
  onSuccess?: ((result: PublishedApplication) => void) | undefined;
  onError?: ((error: unknown) => void) | undefined;
  onTypecheck?: ((event: TypecheckEvent) => void) | undefined;
  signal?: AbortSignal | undefined;
}

export type DevMode = "desktop" | "preview";

export interface DevApplicationOptions extends Omit<BuildApplicationOptions, "development"> {
  strictTypecheck?: boolean | undefined;
  hostArgs?: string[];
  /** Working directory for the native process, independent of bundle and Cargo paths. */
  hostCwd?: string;
  hostRoot?: string;
  hostExecutable?: string;
  mode?: DevMode;
  signal?: AbortSignal;
}

export interface PreviewApplicationOptions extends Omit<
  DevApplicationOptions,
  "hostArgs" | "mode"
> {
  watch?: boolean;
  profile?: string;
  fixture?: string;
  scenario?: string;
  out?: string;
  onOutput?: (directory: string) => void;
}

export interface NativeHostOptions {
  root?: string | undefined;
  signal?: AbortSignal | undefined;
}

export type PrepareNativeHostOptions = NativeHostOptions;
export type RunNativeHostOptions = NativeHostOptions;

export type NativeHostSource = "explicit" | "env" | "platform-package" | "cargo";

export interface NativeHostAbi {
  name: "zgui-host";
  version: string;
  hostApiVersion: number;
  schemaVersion: number;
  platform: string;
  engine?: string | undefined;
  engineRevision?: string | undefined;
  bytecodeFormat?: string | undefined;
  quickjsAbi?: string | undefined;
  styleProfile?: string | undefined;
}

export interface ResolveNativeHostOptions {
  hostExecutable?: string | undefined;
  hostRoot?: string | undefined;
  root?: string | undefined;
  cwd?: string | undefined;
  env?: Record<string, string | undefined> | undefined;
  platform?: string | undefined;
  arch?: string | undefined;
  searchFrom?: readonly string[] | undefined;
  verifyAbi?: boolean | undefined;
  allowCargoBuild?: boolean | undefined;
  signal?: AbortSignal | undefined;
}

export interface ResolvedNativeHost {
  executable: string;
  source: NativeHostSource;
  /** Absolute working directory selected for native host commands. */
  runRoot: string;
  packageName?: string | undefined;
  cargoRoot?: string | undefined;
  abi?: NativeHostAbi | undefined;
}

export interface PackageApplicationOptions extends Omit<BuildApplicationOptions, "development"> {
  out?: string | undefined;
  bytecode?: boolean | undefined;
  hostRoot?: string | undefined;
  hostExecutable?: string | undefined;
  desktopEntry?: boolean | undefined;
  pureShell?: boolean | undefined;
  signal?: AbortSignal | undefined;
}

export interface PackagedApplication {
  out: string;
  launcher: string;
  executable: string;
  bundle: string;
  desktopEntry?: string | undefined;
  host: ResolvedNativeHost;
  manifest: Manifest;
}

export declare const buildApplication: (
  app: string,
  options?: BuildApplicationOptions,
) => Promise<PublishedApplication>;
export declare const syncCssModuleTypes: (options?: SyncCssModuleTypesOptions) => Promise<string[]>;
export declare const watchApplication: (
  app: string,
  options?: WatchApplicationOptions,
) => ApplicationWatcher;
export declare const devApplication: (
  app: string,
  options?: DevApplicationOptions,
) => Promise<number>;
export declare const previewApplication: (
  app: string,
  options?: PreviewApplicationOptions,
) => Promise<number>;
export declare const packageApplication: (
  app: string,
  options?: PackageApplicationOptions,
) => Promise<PackagedApplication>;
export declare const resolveApplication: (
  selector?: string,
  options?: { cwd?: string; root?: string },
) => Promise<ResolvedApplication>;
export declare const prepareNativeHost: (options?: PrepareNativeHostOptions) => Promise<string>;
export declare const probeNativeHost: (
  executable: string,
  options?: { expectedHostApiVersion?: number; expectedSchemaVersion?: number },
) => NativeHostAbi;
export declare const resolveNativeHost: (
  options?: ResolveNativeHostOptions,
) => Promise<ResolvedNativeHost>;
export declare const runNativeHost: (
  executable: string,
  args: readonly string[],
  options?: RunNativeHostOptions,
) => Promise<number>;
