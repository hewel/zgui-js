export type ClipboardCapabilities = {
  read?: boolean;
};

export type SqliteCapabilities = {
  databases: string[];
};

export type Capabilities = {
  shellSurfaces?: boolean;
  sessionLock?: boolean;
  imagePrefixes?: string[];
  unixSockets?: string[];
  namedSockets?: string[];
  commands?: string[];
  controlChannel?: boolean;
  clipboard?: ClipboardCapabilities;
  sqlite?: SqliteCapabilities;
};

export type FontDeclaration = { family: string; path: string };
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
  styles: string[];
  assets: string[];
  fonts: FontDeclaration[];
  window?: WindowDeclaration;
  capabilities?: Capabilities;
  styleSystem: {
    name: "stylex";
    profile: "zgui-stylex-v1";
    nativeProfileDigest: string;
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
  stylexDiagnostics?: boolean;
  development?: boolean;
  extraCapabilities?: Capabilities;
  capabilitiesFile?: string;
}

export interface ApplicationWatcher {
  close(): Promise<void>;
}

export interface WatchApplicationOptions extends BuildApplicationOptions {
  onSuccess?: (result: PublishedApplication) => void;
  onError?: (error: unknown) => void;
  signal?: AbortSignal;
}

export type DevMode = "desktop" | "preview";

export interface DevApplicationOptions extends Omit<BuildApplicationOptions, "development"> {
  hostArgs?: string[];
  hostRoot?: string;
  hostExecutable?: string;
  mode?: DevMode;
  signal?: AbortSignal;
}

export interface PreviewApplicationOptions extends Omit<
  DevApplicationOptions,
  "hostArgs" | "mode" | "stylexDiagnostics"
> {
  watch?: boolean;
  profile?: string;
  fixture?: string;
  scenario?: string;
  out?: string;
  onOutput?: (directory: string) => void;
}

export interface NativeHostOptions {
  root?: string;
  signal?: AbortSignal;
}

export type PrepareNativeHostOptions = NativeHostOptions;
export type RunNativeHostOptions = NativeHostOptions;

export declare const buildApplication: (
  app: string,
  options?: BuildApplicationOptions,
) => Promise<PublishedApplication>;
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
export declare const resolveApplication: (
  selector?: string,
  options?: { cwd?: string; root?: string },
) => Promise<ResolvedApplication>;
export declare const prepareNativeHost: (options?: PrepareNativeHostOptions) => Promise<string>;
export declare const runNativeHost: (
  executable: string,
  args: readonly string[],
  options?: RunNativeHostOptions,
) => Promise<number>;
