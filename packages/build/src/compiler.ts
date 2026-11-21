import path from "node:path";
import { realpathSync } from "node:fs";
import { realpath } from "node:fs/promises";
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
import { styleSystem, transformStylesheet, validateStylesheet, type StyleSystem } from "./css.ts";
import { moduleIdentity } from "./identity.ts";
import {
  validateImport,
  validateWorkerImport,
  validateModule,
  validateGraph,
  type CompilationGraph,
} from "./validate.ts";

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

interface CachedTransform {
  source: string;
  identity: string;
  result: SourceDescription;
  componentSheet?: StaticComponentSheet;
  cssModuleCss?: string;
}

// Single-pass Solid Universal + TypeScript lowering paired with Lightning CSS Modules.
function compilerOptions(
  entry: string,
  {
    root,
    appDirectory = path.dirname(entry),
    plugins = [],
    babelTransform = transformAsync,
    development = false,
    hoistComponentStyles = true,
  }: CompilerOptions,
): InputOptions {
  const transforms = new Map<string, CachedTransform>();
  function pruneTransforms(this: PluginContext): void {
    const active = new Set<string>();
    for (const id of this.getModuleIds()) {
      active.add(id);
      if (!id.startsWith("\0")) {
        try {
          active.add(realpathSync(id));
        } catch {
          // Ignore virtual or deleted module paths.
        }
      }
    }
    for (const filename of transforms.keys()) {
      if (!active.has(filename)) transforms.delete(filename);
    }
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
        name: "lightningcss-solid-universal-typescript",
        resolveId(source) {
          validateImport(source);
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
          const cssModuleParts = [...transforms.entries()]
            .filter(
              ([filename, record]) => record.cssModuleCss !== undefined && reachable.has(filename),
            )
            .map(([, record]) => record)
            .sort((a, b) => (a.identity < b.identity ? -1 : a.identity > b.identity ? 1 : 0))
            .map(record => record.cssModuleCss!)
            .filter(part => part.length > 0);
          const mergedModulesCss =
            cssModuleParts.length > 0 ? `${cssModuleParts.join("\n\n")}\n` : "";
          this.emitFile({ type: "asset", fileName: "app.css", source: mergedModulesCss });

          if (hoistComponentStyles) {
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
              const componentsCss = `${validateStylesheet(`@layer zgui.components {\n${body}\n}`, "components.css")}\n`;
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
          if (filename.startsWith("\0")) return null;
          if (/\.module\.css$/i.test(filename)) {
            const real = await realpath(filename);
            const cached = transforms.get(real);
            if (cached?.source === source) return cached.result;
            const identity = moduleIdentity(real, { root, appDirectory });
            const { css, exports: exportsMap } = transformStylesheet(source, {
              filename: identity,
              cssModules: true,
            });
            const code = `export default Object.freeze(${JSON.stringify(exportsMap, null, 2)});\n`;
            const result: SourceDescription = {
              code,
              map: { version: 3, sources: [real], names: [], mappings: "AAAA" },
              moduleType: "js",
            };
            transforms.set(real, {
              source,
              identity,
              result,
              cssModuleCss: css,
            });
            return result;
          }
          if (!/\.[cm]?[jt]sx?$/.test(filename)) return null;
          const real = await realpath(filename);
          const cached = transforms.get(real);
          if (cached?.source === source) return cached.result;
          const identity = moduleIdentity(real, { root, appDirectory });
          if (hoistComponentStyles) {
            const componentSheet = extractStaticComponentSheet(source, real);
            if (componentSheet) {
              const code = `export const sheet = { name: ${JSON.stringify(componentSheet.name)}, className: ${JSON.stringify(componentSheet.className)}, css: "" };\n`;
              const result: SourceDescription = {
                code,
                map: { version: 3, sources: [real], names: [], mappings: "AAAA" },
                moduleType: "js",
              };
              transforms.set(real, {
                source,
                identity,
                result,
                componentSheet,
              });
              return result;
            }
          }
          const transformed = await babelTransform(source, {
            filename: real,
            babelrc: false,
            configFile: false,
            sourceMaps: true,
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
                  isTSX: /\.tsx$/.test(real),
                },
              ],
            ],
          });
          if (!transformed?.code || !transformed.map)
            throw new Error(`Babel emitted no JavaScript or source map for ${real}`);
          validateModule(transformed.code, { filename: real });
          const result: SourceDescription = {
            code: transformed.code,
            map: transformed.map,
            moduleType: "js",
          };
          transforms.set(real, {
            source,
            identity,
            result,
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
  if (!files.has("app.css")) {
    throw new Error("Missing Lightning CSS stylesheet output");
  }
  return { files, graph, solidRuntime, styleSystem: styleSystem() };
}

export async function compileBundle(
  entry: string,
  {
    root,
    appDirectory = path.dirname(entry),
    hostEntry = true,
    filename = "app.mjs",
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
