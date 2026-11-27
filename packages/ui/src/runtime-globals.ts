/** Host-owned registrations, not browser or Node compatibility. */
export {};

declare global {
  const console: {
    log(...values: unknown[]): void;
    info(...values: unknown[]): void;
    warn(...values: unknown[]): void;
    error(...values: unknown[]): void;
    debug(...values: unknown[]): void;
  };
  function setTimeout(callback: () => void, delay?: number): number;
  function clearTimeout(handle: number): void;
  function requestAnimationFrame(callback: (timestamp: number) => void): number;
  function cancelAnimationFrame(handle: number): void;
  function queueMicrotask(callback: () => void): void;

  class URL {
    constructor(url: string | URL, base?: string | URL);
    static canParse(url: string | URL, base?: string | URL): boolean;
    static parse(url: string | URL, base?: string | URL): URL | null;
    href: string;
    readonly origin: string;
    protocol: string;
    username: string;
    password: string;
    host: string;
    hostname: string;
    port: string;
    pathname: string;
    search: string;
    hash: string;
    readonly searchParams: URLSearchParams;
    readonly toString: () => string;
    readonly toJSON: () => string;
  }

  class URLSearchParams {
    constructor(
      init?:
        | string
        | URLSearchParams
        | Iterable<readonly [string, string]>
        | Readonly<Record<string, string>>,
    );
    readonly size: number;
    readonly append: (name: string, value: string) => void;
    readonly delete: (name: string, value?: string) => void;
    readonly get: (name: string) => string | null;
    readonly getAll: (name: string) => string[];
    readonly has: (name: string, value?: string) => boolean;
    readonly set: (name: string, value: string) => void;
    readonly sort: () => void;
    readonly entries: () => IterableIterator<[string, string]>;
    readonly keys: () => IterableIterator<string>;
    readonly values: () => IterableIterator<string>;
    readonly [Symbol.iterator]: () => IterableIterator<[string, string]>;
    readonly forEach: (
      callback: (value: string, key: string, parent: URLSearchParams) => void,
      thisArg?: unknown,
    ) => void;
    readonly toString: () => string;
  }

  interface TextEncoderEncodeIntoResult {
    readonly read: number;
    readonly written: number;
  }

  class TextEncoder {
    readonly encoding: "utf-8";
    readonly encode: (input?: string) => Uint8Array<ArrayBuffer>;
    readonly encodeInto: (source: string, destination: Uint8Array) => TextEncoderEncodeIntoResult;
  }

  interface TextDecoderOptions {
    readonly fatal?: boolean | undefined;
    readonly ignoreBOM?: boolean | undefined;
  }

  interface TextDecodeOptions {
    readonly stream?: boolean | undefined;
  }

  class TextDecoder {
    constructor(label?: string, options?: TextDecoderOptions);
    readonly encoding: "utf-8";
    readonly fatal: boolean;
    readonly ignoreBOM: boolean;
    readonly decode: (
      input?: ArrayBufferLike | ArrayBufferView,
      options?: TextDecodeOptions,
    ) => string;
  }
}
