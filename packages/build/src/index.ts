import type * as Implementation from "./index.ts";
import type * as Public from "./public-api.d.ts";
import { resolveApplication as resolve } from "./application.ts";
import { buildApplication as build } from "./build.ts";
import { devApplication as dev } from "./dev.ts";
import { prepareNativeHost as prepare, runNativeHost as run } from "./native-process.ts";
import { previewApplication as preview } from "./preview.ts";
import { watchApplication as watch } from "./watch.ts";

export type * from "./public-api.d.ts";

export const buildApplication: typeof Public.buildApplication = build;
export const watchApplication: typeof Public.watchApplication = watch;
export const devApplication: typeof Public.devApplication = dev;
export const previewApplication: typeof Public.previewApplication = preview;
export const resolveApplication: typeof Public.resolveApplication = resolve;
export const prepareNativeHost: typeof Public.prepareNativeHost = prepare;
export const runNativeHost: typeof Public.runNativeHost = run;

type Assert<T extends true> = T;
type ExactKeys<A, B> = [A, B] extends [B, A] ? true : false;
type _ValueExportParity = Assert<ExactKeys<keyof typeof Implementation, keyof typeof Public>>;
