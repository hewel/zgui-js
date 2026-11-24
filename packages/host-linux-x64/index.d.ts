export interface StagedHostManifest {
  readonly schemaVersion: 1;
  readonly package: string;
  readonly platform: string;
  readonly version: string;
  readonly hostApiVersion: number;
  readonly bundleSchemaVersion: number;
  readonly sha256: string;
  readonly bytes: number;
  readonly libc?: "glibc" | undefined;
  readonly glibcBaseline?: string | undefined;
}

export declare const platform: "linux-x64";
export declare const binaryPath: string;
export declare const manifestPath: string;
export declare function resolveHostBinary(): string | null;
export declare function readHostManifest(): StagedHostManifest | null;
