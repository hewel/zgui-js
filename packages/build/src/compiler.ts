import path from "node:path";
import { realpathSync } from "node:fs";
import {
  transformAsync,
  type BabelFileResult,
  type TransformOptions as BabelTransformOptions,
} from "@babel/core";
import { parse } from "@babel/parser";
import typescriptPresetModule from "@babel/preset-typescript";
import solidBabelPlugin from "@solidjs/babel-plugin";
import {
  rolldown,
  watch as watchCompiler,
  type InputOptions,
  type OutputAsset,
  type OutputChunk,
  type OutputOptions,
  type Plugin,
  type PluginContext,
  type SourceDescription,
} from "rolldown";
import type { Rule } from "@stylexjs/babel-plugin";
import {
  validateImport,
  validateWorkerImport,
  validateModule,
  validateGraph,
  type CompilationGraph,
} from "./validate.ts";
import { realpath } from "node:fs/promises";
import { moduleIdentity } from "./identity.ts";
import {
  stylexPlugin,
  stylexOptions,
  styleSystem,
  readNativeProfileDigest,
  validateStylexModule,
  validateStylesheet,
  finalizeStylex,
  type StyleSystem,
  type StylexModule,
} from "./stylex.ts";

const typescriptPreset =
  (
    typescriptPresetModule as typeof typescriptPresetModule & {
      default?: typeof typescriptPresetModule;
    }
  ).default ?? typescriptPresetModule;

export interface CompilationResult {
  files: Map<string, Buffer>;
  graph: CompilationGraph;
  solidRuntime: string;
  styleSystem: StyleSystem;
}

export type BabelTransform = (
  source: string,
  options: BabelTransformOptions,
) => Promise<Pick<BabelFileResult, "code" | "map"> | null>;

interface CompilerOptions {
  root: string;
  appDirectory?: string;
  plugins?: Plugin[];
  babelTransform?: BabelTransform;
  stylexDiagnostics?: boolean;
  development?: boolean;
  hoistComponentStyles?: boolean;
}

export interface CompileBundleOptions extends CompilerOptions {
  hostEntry?: boolean;
  filename?: string;
}

export interface WatchBundleOptions extends Omit<CompilerOptions, "plugins"> {
  temporaryDirectory?: string;
  filename?: string;
  buildDelay?: number;
  watchFiles?: string[];
  onAttempt?: () => void | Promise<void>;
  onCompile?: (result: CompilationResult) => void | Promise<void>;
  onError?: (error: unknown) => void | Promise<void>;
}

export interface BundleWatcher {
  close(): Promise<void>;
}

interface StaticComponentSheet {
  readonly name: string;
  readonly className: string;
  readonly css: string;
}

const GENERATED_STYLE_FILE =
  /(?:^|[\\/])components[\\/]generated[\\/][A-Za-z0-9]+Style\.[cm]?[jt]s$/;

function extractStaticComponentSheet(
  source: string,
  filename: string,
): StaticComponentSheet | undefined {
  if (!GENERATED_STYLE_FILE.test(filename)) return undefined;
  const ast = parse(source, { sourceType: "module", plugins: ["typescript"] });
  if (ast.program.body.length !== 1) return undefined;
  const statement = ast.program.body[0];
  if (
    statement?.type !== "ExportNamedDeclaration" ||
    statement.declaration?.type !== "VariableDeclaration"
  ) {
    return undefined;
  }
  const decl = statement.declaration.declarations[0];
  if (
    statement.declaration.declarations.length !== 1 ||
    decl?.id.type !== "Identifier" ||
    decl.id.name !== "sheet" ||
    !decl.init
  ) {
    return undefined;
  }
  const expr = decl.init.type === "TSAsExpression" ? decl.init.expression : decl.init;
  if (expr.type !== "ObjectExpression") return undefined;
  let name: string | undefined;
  let className: string | undefined;
  let css: string | undefined;
  for (const prop of expr.properties) {
    if (prop.type !== "ObjectProperty" || prop.computed || prop.key.type !== "Identifier")
      return undefined;
    if (prop.value.type !== "StringLiteral") return undefined;
    if (prop.key.name === "name") name = prop.value.value;
    else if (prop.key.name === "className") className = prop.value.value;
    else if (prop.key.name === "css") css = prop.value.value;
    else return undefined;
  }
  if (name === undefined || className === undefined || css === undefined) return undefined;
  return { name, className, css };
}

interface CachedTransform extends StylexModule {
  source: string;
  result: SourceDescription;
  componentSheet?: StaticComponentSheet;
}

function isStylexRule(rule: unknown): rule is Rule {
  return (
    Array.isArray(rule) &&
    typeof rule[0] === "string" &&
    typeof rule[1]?.ltr === "string" &&
    Number.isFinite(rule[2])
  );
}

function isStyleSystem(value: unknown): value is StyleSystem {
  return (
    typeof value === "object" &&
    value !== null &&
    "name" in value &&
    value.name === "stylex" &&
    "profile" in value &&
    value.profile === "zgui-stylex-v1" &&
    "configDigest" in value &&
    typeof value.configDigest === "string" &&
    /^sha256:[a-f0-9]{64}$/.test(value.configDigest) &&
    "nativeProfileDigest" in value &&
    typeof value.nativeProfileDigest === "string" &&
    /^sha256:[a-f0-9]{64}$/.test(value.nativeProfileDigest)
  );
}

// Babel owns JSX lowering; Rolldown composes this map with its ES2022 output map.
function compilerOptions(
  entry: string,
  {
    root,
    appDirectory = path.dirname(entry),
    plugins = [],
    babelTransform = transformAsync,
    stylexDiagnostics = false,
    development = false,
    hoistComponentStyles = true,
  }: CompilerOptions,
): InputOptions {
  // Rolldown 1.3.2 watch rescans the graph even with incrementalBuild enabled.
  // Keep expensive Babel/Solid work incremental without caching resolution or diagnostics.
  const transforms = new Map<string, CachedTransform>();
  let nativeProfileDigest: string;
  let publicationStyleSystem: StyleSystem;
  function pruneTransforms(this: PluginContext): void {
    const active = new Set(this.getModuleIds());
    for (const filename of transforms.keys())
      if (!active.has(filename)) transforms.delete(filename);
  }
  return {
    cwd: appDirectory,
    input: path.resolve(entry),
    platform: "neutral",
    experimental: { attachDebugInfo: "none" },
    resolve: {
      conditionNames: [development ? "development" : "production", "browser", "import", "default"],
      mainFields: ["module", "main"],
      modules: [
        "node_modules",
        path.join(appDirectory, "node_modules"),
        path.join(root, "node_modules"),
      ],
    },
    transform: {
      target: "es2022",
      define: { "process.env.NODE_ENV": development ? '"development"' : '"production"' },
    },
    onwarn(warning, warn) {
      if (warning.code === "UNRESOLVED_IMPORT") throw new Error(warning.message);
      warn(warning);
    },
    plugins: [
      {
        name: "stylex-solid-universal-typescript",
        resolveId(source) {
          validateImport(source);
        },
        buildStart() {
          this.addWatchFile(path.join(root, "schemas/stylex-profile.json"));
          const digest = readNativeProfileDigest(root);
          if (nativeProfileDigest !== digest) transforms.clear();
          nativeProfileDigest = digest;
          publicationStyleSystem = styleSystem(stylexDiagnostics, digest);
        },
        buildEnd: pruneTransforms,
        outputOptions(options) {
          return {
            ...options,
            sourcemapPathTransform(source, sourceMapPath) {
              return moduleIdentity(path.resolve(path.dirname(sourceMapPath), source), {
                root,
                appDirectory,
              });
            },
          };
        },
        generateBundle(_options, output) {
          const active = new Set(this.getModuleIds());
          const modules = [...transforms]
            .filter(([filename]) => active.has(filename))
            .map(([, record]) => record);
          const { css, origins } = finalizeStylex(modules);
          this.emitFile({ type: "asset", fileName: "app.css", source: css });
          this.emitFile({
            type: "asset",
            fileName: "style-origins.json",
            source:
              JSON.stringify({ ...origins, styleSystem: publicationStyleSystem }, null, 2) + "\n",
          });
          if (hoistComponentStyles) {
            const reachable = new Set<string>();
            for (const item of Object.values(output)) {
              if (item.type !== "chunk") continue;
              for (const id of item.moduleIds) {
                if (id.startsWith("\0")) continue;
                reachable.add(id);
                try {
                  reachable.add(realpathSync(id));
                } catch {
                  // Ignore virtual or non-filesystem module IDs.
                }
              }
            }
            const hoisted = new Map<string, string>();
            for (const [filename, record] of transforms) {
              if (!record.componentSheet || !reachable.has(filename)) continue;
              if (record.componentSheet.css.trim().length === 0) continue;
              hoisted.set(record.componentSheet.name, record.componentSheet.css);
            }
            if (hoisted.size > 0) {
              const body = [...hoisted.entries()]
                .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
                .map(([name, sheetCss]) => `/* ${name} */\n${sheetCss}`)
                .join("\n");
              const componentsCss = `@layer zgui.components {\n${body}\n}\n`;
              validateStylesheet(componentsCss, "components.css");
              this.emitFile({ type: "asset", fileName: "components.css", source: componentsCss });
            }
          }
        },
        closeWatcher() {
          transforms.clear();
        },
        closeBundle() {
          if (!this.meta.watchMode) transforms.clear();
        },
        async transform(source, filename) {
          if (filename.startsWith("\0") || !/\.[cm]?[jt]sx?$/.test(filename)) return null;
          filename = await realpath(filename);
          const cached = transforms.get(filename);
          if (cached?.source === source) return cached.result;
          if (hoistComponentStyles) {
            const componentSheet = extractStaticComponentSheet(source, filename);
            if (componentSheet) {
              const code = `export const sheet = { name: ${JSON.stringify(componentSheet.name)}, className: ${JSON.stringify(componentSheet.className)}, css: "" };\n`;
              const result: SourceDescription = {
                code,
                map: { version: 3, sources: [filename], names: [], mappings: "AAAA" },
                moduleType: "js",
              };
              transforms.set(filename, {
                source,
                result,
                rules: [],
                identity: moduleIdentity(filename, { root, appDirectory }),
                componentSheet,
              });
              return result;
            }
          }
          validateStylexModule(source, { filename });
          // Pass one preserves JSX and TypeScript; its metadata is owned by this build pass.
          const first = await transformAsync(source, {
            filename,
            babelrc: false,
            configFile: false,
            sourceMaps: true,
            parserOpts: {
              plugins: /\.[cm]?ts$/.test(filename) ? ["typescript"] : ["typescript", "jsx"],
            },
            plugins: [
              [
                stylexPlugin,
                stylexOptions(
                  appDirectory,
                  stylexDiagnostics,
                  filename => moduleIdentity(filename, { root, appDirectory }),
                  nativeProfileDigest,
                ),
              ],
            ],
          });
          const metadata: unknown = first?.metadata;
          if (
            !first?.code ||
            !first.map ||
            typeof metadata !== "object" ||
            metadata === null ||
            !("stylex" in metadata) ||
            !Array.isArray(metadata.stylex)
          ) {
            throw new Error(`StyleX emitted missing or incompatible output for ${filename}`);
          }
          const rules: unknown[] = metadata.stylex;
          if (!rules.every(isStylexRule)) {
            throw new Error(`StyleX emitted incompatible rule metadata for ${filename}`);
          }
          const transformed = await babelTransform(first.code, {
            filename,
            babelrc: false,
            configFile: false,
            sourceMaps: true,
            inputSourceMap: first.map,
            presets: [
              {
                plugins: [
                  [
                    solidBabelPlugin,
                    { generate: "universal", moduleName: "@hewel/zgui/renderer", dev: development },
                  ],
                ],
              },
              [
                typescriptPreset,
                {
                  onlyRemoveTypeImports: true,
                  allExtensions: true,
                  isTSX: /\.tsx$/.test(filename),
                },
              ],
            ],
          });
          if (!transformed?.code || !transformed.map)
            throw new Error(`Babel emitted no JavaScript or composed source map for ${filename}`);
          validateModule(transformed.code, { filename });
          validateStylexModule(transformed.code, { filename, compiled: true });
          const result: SourceDescription = {
            code: transformed.code,
            map: transformed.map,
            moduleType: "js",
          };
          transforms.set(filename, {
            source,
            result,
            rules: structuredClone(rules),
            identity: moduleIdentity(filename, { root, appDirectory }),
          });
          return result;
        },
      } satisfies Plugin,
      ...plugins,
    ],
  };
}

function outputOptions(filename = "app.mjs", { codeSplitting = true } = {}): OutputOptions {
  return {
    format: "esm",
    entryFileNames: filename,
    chunkFileNames: "chunks/[name]-[hash].mjs",
    sourcemap: "hidden",
    codeSplitting,
    minify: false,
  };
}

// A native graph, not an emulation of another bundler's metadata schema.
function compilationResult(
  output: (OutputAsset | OutputChunk)[],
  moduleIds: Iterable<string>,
  { entry = true }: { entry?: boolean } = {},
): CompilationResult {
  const chunks = output.filter((item): item is OutputChunk => item.type === "chunk");
  const graph: CompilationGraph = {
    modules: [...moduleIds].filter(id => !id.startsWith("\0")).sort(),
    chunks: chunks.map(chunk => ({
      fileName: chunk.fileName,
      modules: [...chunk.moduleIds].filter(id => !id.startsWith("\0")).sort(),
      imports: [...chunk.imports],
      dynamicImports: [...chunk.dynamicImports],
      exports: [...chunk.exports].sort(),
    })),
  };
  const solidRuntime = validateGraph(graph, { allowMultipleChunks: entry });
  const allowedChunks = new Set(chunks.map(chunk => chunk.fileName));
  const files = new Map<string, Buffer>();
  for (const item of output) {
    if (item.type === "chunk") {
      if (!item.map) throw new Error(`Missing composed source map: ${item.fileName}`);
      const code = `${item.code.trimEnd()}\n//# sourceMappingURL=${path.basename(item.fileName)}.map\n`;
      validateModule(code, {
        filename: item.fileName,
        bundled: true,
        entry: entry && item.isEntry,
        allowedChunks,
      });
      files.set(item.fileName, Buffer.from(code));
      files.set(`${item.fileName}.map`, Buffer.from(item.map.toString()));
    } else files.set(item.fileName, Buffer.from(item.source));
  }
  const origins = files.get("style-origins.json");
  if (!files.has("app.css") || !origins)
    throw new Error("Missing StyleX stylesheet or provenance output");
  const { styleSystem }: { styleSystem?: unknown } = JSON.parse(origins.toString());
  if (!isStyleSystem(styleSystem)) {
    throw new Error("Missing or incompatible StyleX profile metadata");
  }
  return { files, graph, solidRuntime, styleSystem };
}

export async function compileBundle(
  entry: string,
  {
    root,
    appDirectory = path.dirname(entry),
    hostEntry = true,
    filename = "app.mjs",
    stylexDiagnostics = false,
    development = false,
    babelTransform = transformAsync,
    plugins = [],
  }: CompileBundleOptions,
): Promise<CompilationResult> {
  let result!: CompilationResult;
  const compiler = await rolldown(
    compilerOptions(entry, {
      root,
      appDirectory,
      stylexDiagnostics,
      development,
      hoistComponentStyles: hostEntry,
      babelTransform,
      plugins: [
        {
          name: "capture-native-graph",
          generateBundle(_options, output) {
            result = compilationResult(Object.values(output), this.getModuleIds(), {
              entry: hostEntry,
            });
          },
        } satisfies Plugin,
        ...plugins,
      ],
    }),
  );
  try {
    await compiler.generate(outputOptions(filename, { codeSplitting: hostEntry }));
    return result;
  } finally {
    await compiler.close();
  }
}

export interface CompiledWorkerBundle {
  name: string;
  path: string;
  sourceMap: string;
  files: Map<string, Buffer>;
}

export async function compileWorkerBundle(
  entry: string,
  {
    root,
    appDirectory = path.dirname(entry),
    name,
    development = false,
  }: {
    root: string;
    appDirectory?: string;
    name: string;
    development?: boolean;
  },
): Promise<CompiledWorkerBundle> {
  const filename = `workers/${name}.mjs`;
  const files = new Map<string, Buffer>();
  const compiler = await rolldown({
    cwd: appDirectory,
    input: path.resolve(entry),
    platform: "neutral",
    experimental: { attachDebugInfo: "none" },
    resolve: {
      conditionNames: [development ? "development" : "production", "import", "default"],
      mainFields: ["module", "main"],
      modules: [
        "node_modules",
        path.join(appDirectory, "node_modules"),
        path.join(root, "node_modules"),
      ],
    },
    transform: {
      target: "es2022",
      define: { "process.env.NODE_ENV": development ? '"development"' : '"production"' },
    },
    onwarn(warning, warn) {
      if (warning.code === "UNRESOLVED_IMPORT") throw new Error(warning.message);
      warn(warning);
    },
    plugins: [
      {
        name: "zgui-compute-worker",
        resolveId(source) {
          validateWorkerImport(source);
        },
        outputOptions(options) {
          return {
            ...options,
            sourcemapPathTransform(source, sourceMapPath) {
              return moduleIdentity(path.resolve(path.dirname(sourceMapPath), source), {
                root,
                appDirectory,
              });
            },
          };
        },
        generateBundle(_options, output) {
          const chunks = Object.values(output).filter(
            (item): item is OutputChunk => item.type === "chunk",
          );
          if (chunks.length !== 1) {
            throw new Error(`Compute worker ${name} must emit a single self-contained ESM chunk`);
          }
          const chunk = chunks[0];
          if (chunk.imports.length > 0 || chunk.dynamicImports.length > 0) {
            throw new Error(
              `Compute worker ${name} must have a closed module graph with no external imports`,
            );
          }
          if (!chunk.map) {
            throw new Error(`Missing source map for compute worker ${name}`);
          }
          validateModule(chunk.code, {
            filename: chunk.fileName,
            bundled: true,
            entry: false,
            worker: true,
          });
          files.set(chunk.fileName, Buffer.from(chunk.code));
          files.set(`${chunk.fileName}.map`, Buffer.from(chunk.map.toString()));
        },
      } satisfies Plugin,
    ],
  });
  try {
    await compiler.generate({
      format: "es",
      codeSplitting: false,
      sourcemap: true,
      entryFileNames: filename,
    });
    return { name, path: filename, sourceMap: `${filename}.map`, files };
  } finally {
    await compiler.close();
  }
}

export function watchBundle(
  entry: string,
  {
    root,
    appDirectory = path.dirname(entry),
    temporaryDirectory,
    filename = "app.mjs",
    buildDelay = 80,
    stylexDiagnostics = false,
    development = false,
    babelTransform = transformAsync,
    watchFiles = [],
    onAttempt,
    onCompile,
    onError,
  }: WatchBundleOptions,
): BundleWatcher {
  let attemptError: unknown;
  const preserve = async <T>(operation: () => T | Promise<T>): Promise<T> => {
    try {
      return await operation();
    } catch (error) {
      attemptError = error;
      throw error;
    }
  };
  const watcher = watchCompiler({
    ...compilerOptions(entry, {
      root,
      appDirectory,
      stylexDiagnostics,
      development,
      hoistComponentStyles: true,
      babelTransform,
      plugins: [
        {
          name: "immutable-native-compilation",
          async buildStart() {
            attemptError = undefined;
            for (const file of watchFiles) this.addWatchFile(file);
            if (onAttempt) return preserve(() => onAttempt());
          },
          async generateBundle(_options, output) {
            // Consume output in memory and suppress bundler writes so no partial package lands on disk.
            try {
              const result = await preserve(() =>
                compilationResult(Object.values(output), this.getModuleIds()),
              );
              if (onCompile) await preserve(() => onCompile(result));
            } finally {
              for (const name of Object.keys(output)) delete output[name];
            }
          },
        } satisfies Plugin,
      ],
    }),
    output: { ...outputOptions(filename), dir: temporaryDirectory },
    watch: { buildDelay },
  });
  watcher.on("event", async event => {
    if (event.code !== "BUNDLE_END" && event.code !== "ERROR") return;
    try {
      if (event.code === "ERROR") await onError?.(attemptError ?? event.error);
    } finally {
      await event.result?.close();
    }
  });
  return { close: () => watcher.close() };
}
