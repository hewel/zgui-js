import path from "node:path";
import { transformAsync } from "@babel/core";
import typescriptPreset from "@babel/preset-typescript";
import solidPreset from "babel-preset-solid";
import { rolldown, watch as watchCompiler } from "rolldown";
import { validateImport, validateModule, validateGraph } from "./validate.mjs";
import { realpath } from "node:fs/promises";
import { moduleIdentity } from "./identity.mjs";
import { stylexPlugin, stylexOptions, styleSystem, readNativeProfileDigest, validateStylexModule, finalizeStylex } from "./stylex.mjs";

// Babel owns JSX lowering; Rolldown composes this map with its ES2022 output map.
function compilerOptions(entry, { root, appDirectory = path.dirname(entry), plugins = [], babelTransform = transformAsync, stylexDiagnostics = false }) {
  // Rolldown 1.3.2 watch rescans the graph even with incrementalBuild enabled.
  // Keep expensive Babel/Solid work incremental without caching resolution or diagnostics.
  const transforms = new Map();
  let nativeProfileDigest, generationStyleSystem;
  function pruneTransforms() {
    const active = new Set(this.getModuleIds());
    for (const filename of transforms.keys()) if (!active.has(filename)) transforms.delete(filename);
  }
  return {
    cwd: appDirectory, input: path.resolve(entry), platform: "neutral",
    experimental: { attachDebugInfo: "none" },
    resolve: {
      conditionNames: ["browser", "production", "import", "default"],
      mainFields: ["module", "main"],
      modules: ["node_modules", path.join(appDirectory, "node_modules"), path.join(root, "node_modules")],
    },
    transform: { target: "es2022", define: { "process.env.NODE_ENV": '"production"' } },
    onwarn(warning, warn) {
      if (warning.code === "UNRESOLVED_IMPORT") throw new Error(warning.message);
      warn(warning);
    },
    plugins: [{
      name: "stylex-solid-universal-typescript",
      resolveId(source) { validateImport(source); },
      buildStart() {
        this.addWatchFile(path.join(root, "schemas/stylex-profile.json"));
        const digest = readNativeProfileDigest(root);
        if (nativeProfileDigest !== digest) transforms.clear();
        nativeProfileDigest = digest;
        generationStyleSystem = styleSystem(stylexDiagnostics, digest);
      },
      buildEnd: pruneTransforms,
      outputOptions(options) {
        return {
          ...options,
          sourcemapPathTransform(source, sourceMapPath) {
            return moduleIdentity(path.resolve(path.dirname(sourceMapPath), source), { root, appDirectory });
          },
        };
      },
      generateBundle() {
        const active = new Set(this.getModuleIds());
        const modules = [...transforms].filter(([filename]) => active.has(filename)).map(([, record]) => record);
        const { css, origins } = finalizeStylex(modules);
        this.emitFile({ type: "asset", fileName: "app.css", source: css });
        this.emitFile({ type: "asset", fileName: "style-origins.json", source: JSON.stringify({ ...origins, styleSystem: generationStyleSystem }, null, 2) + "\n" });
      },
      closeWatcher() { transforms.clear(); },
      closeBundle() { if (!this.meta.watchMode) transforms.clear(); },
      async transform(source, filename) {
        if (filename.startsWith("\0") || !/\.[cm]?[jt]sx?$/.test(filename)) return null;
        filename = await realpath(filename);
        const cached = transforms.get(filename);
        if (cached?.source === source) return cached.result;
        validateStylexModule(source, { filename });
        // Pass one preserves JSX and TypeScript; its metadata is owned by this generation.
        const first = await transformAsync(source, {
          filename, babelrc: false, configFile: false, sourceMaps: true,
          parserOpts: { plugins: /\.[cm]?ts$/.test(filename) ? ["typescript"] : ["typescript", "jsx"] },
          plugins: [[stylexPlugin, stylexOptions(appDirectory, stylexDiagnostics, filename => moduleIdentity(filename, { root, appDirectory }), nativeProfileDigest)]],
        });
        if (!first?.code || !first.map || !Array.isArray(first.metadata?.stylex)) {
          throw new Error(`StyleX emitted missing or incompatible output for ${filename}`);
        }
        const rules = first.metadata.stylex;
        if (rules.some(rule => !Array.isArray(rule) || typeof rule[0] !== "string" ||
          typeof rule[1]?.ltr !== "string" || !Number.isFinite(rule[2]))) {
          throw new Error(`StyleX emitted incompatible rule metadata for ${filename}`);
        }
        const transformed = await babelTransform(first.code, {
          filename, babelrc: false, configFile: false, sourceMaps: true, inputSourceMap: first.map,
          presets: [
            [solidPreset, { generate: "universal", moduleName: "@hewel/zgui/renderer", hydratable: false }],
            [typescriptPreset, { onlyRemoveTypeImports: true, allExtensions: true, isTSX: /\.tsx$/.test(filename) }],
          ],
        });
        if (!transformed?.code || !transformed.map) throw new Error(`Babel emitted no JavaScript or composed source map for ${filename}`);
        validateModule(transformed.code, { filename });
        validateStylexModule(transformed.code, { filename, compiled: true });
        const result = { code: transformed.code, map: transformed.map, moduleType: "js" };
        transforms.set(filename, { source, result, rules: structuredClone(rules), identity: moduleIdentity(filename, { root, appDirectory }) });
        return result;
      },
    }, ...plugins],
  };
}

function outputOptions(filename = "app.mjs") {
  return { format: "esm", entryFileNames: filename, sourcemap: "hidden", codeSplitting: false, minify: false };
}

// A native graph, not an emulation of another bundler's metadata schema.
function compilationResult(output, moduleIds, { entry = true } = {}) {
  const graph = {
    modules: [...moduleIds].filter(id => !id.startsWith("\0")).sort(),
    chunks: output.filter(item => item.type === "chunk").map(chunk => ({
      fileName: chunk.fileName, modules: [...chunk.moduleIds].filter(id => !id.startsWith("\0")).sort(),
      imports: [...chunk.imports], dynamicImports: [...chunk.dynamicImports], exports: [...chunk.exports].sort(),
    })),
  };
  const solidRuntime = validateGraph(graph);
  const files = new Map();
  for (const item of output) {
    if (item.type === "chunk") {
      if (!item.map) throw new Error(`Missing composed source map: ${item.fileName}`);
      const code = `${item.code.trimEnd()}\n//# sourceMappingURL=${path.basename(item.fileName)}.map\n`;
      validateModule(code, { filename: item.fileName, bundled: true, entry });
      files.set(item.fileName, Buffer.from(code));
      files.set(`${item.fileName}.map`, Buffer.from(item.map.toString()));
    } else files.set(item.fileName, Buffer.from(item.source));
  }
  if (!files.has("app.css") || !files.has("style-origins.json")) throw new Error("Missing StyleX stylesheet or provenance output");
  const { styleSystem } = JSON.parse(files.get("style-origins.json").toString());
  if (styleSystem?.name !== "stylex" || styleSystem.profile !== "zgui-stylex-v1" ||
      !/^sha256:[a-f0-9]{64}$/.test(styleSystem.configDigest) || !/^sha256:[a-f0-9]{64}$/.test(styleSystem.nativeProfileDigest)) {
    throw new Error("Missing or incompatible StyleX profile metadata");
  }
  return { files, graph, solidRuntime, styleSystem };
}

export async function compileBundle(entry, {
  root,
  appDirectory = path.dirname(entry),
  hostEntry = true,
  filename = "app.mjs",
  stylexDiagnostics = false,
  babelTransform = transformAsync,
  plugins = [],
} = {}) {
  let result;
  const compiler = await rolldown(compilerOptions(entry, {
    root, appDirectory, stylexDiagnostics, babelTransform,
    plugins: [{
      name: "capture-native-graph",
      generateBundle(_options, output) {
        result = compilationResult(Object.values(output), this.getModuleIds(), { entry: hostEntry });
      },
    }, ...plugins],
  }));
  try {
    await compiler.generate(outputOptions(filename));
    return result;
  } finally {
    await compiler.close();
  }
}

export function watchBundle(entry, {
  root,
  appDirectory = path.dirname(entry),
  temporaryDirectory,
  filename = "app.mjs",
  buildDelay = 80,
  stylexDiagnostics = false,
  babelTransform = transformAsync,
  watchFiles = [],
  onAttempt,
  onCompile,
  onError,
} = {}) {
  let attemptError;
  const preserve = async operation => {
    try { return await operation(); }
    catch (error) { attemptError = error; throw error; }
  };
  const watcher = watchCompiler({
    ...compilerOptions(entry, {
      root, appDirectory, stylexDiagnostics, babelTransform,
      plugins: [{
        name: "immutable-native-compilation",
        async buildStart() {
          attemptError = undefined;
          for (const file of watchFiles) this.addWatchFile(file);
          if (onAttempt) return preserve(() => onAttempt());
        },
        async generateBundle(_options, output) {
          // Consume output in memory and suppress bundler writes so no partial package lands on disk.
          try {
            const result = await preserve(() => compilationResult(Object.values(output), this.getModuleIds()));
            if (onCompile) await preserve(() => onCompile(result));
          } finally {
            for (const name of Object.keys(output)) delete output[name];
          }
        },
      }],
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
