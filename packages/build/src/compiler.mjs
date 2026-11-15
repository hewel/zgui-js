import path from "node:path";
import { transformAsync } from "@babel/core";
import typescriptPreset from "@babel/preset-typescript";
import solidPreset from "babel-preset-solid";
import { validateImport, validateModule, validateGraph } from "./validate.mjs";

// Babel owns JSX lowering; Rolldown composes this map with its ES2022 output map.
export function compilerOptions(entry, { root, appDirectory = path.dirname(entry), plugins = [], babelTransform = transformAsync }) {
  // Rolldown 1.3.2 watch rescans the graph even with incrementalBuild enabled.
  // Keep expensive Babel/Solid work incremental without caching resolution or diagnostics.
  const transforms = new Map();
  function pruneTransforms() {
    const active = new Set(this.getModuleIds());
    for (const filename of transforms.keys()) if (!active.has(filename)) transforms.delete(filename);
  }
  return {
    cwd: appDirectory, input: path.resolve(entry), platform: "neutral",
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
      name: "solid-universal-typescript",
      resolveId(source) { validateImport(source); },
      buildEnd: pruneTransforms,
      closeWatcher() { transforms.clear(); },
      closeBundle() { if (!this.meta.watchMode) transforms.clear(); },
      async transform(source, filename) {
        if (!/\.[cm]?[jt]sx?$/.test(filename)) return null;
        const cached = transforms.get(filename);
        if (cached?.source === source) return cached.result;
        const transformed = await babelTransform(source, {
          filename, babelrc: false, configFile: false, sourceMaps: true,
          presets: [
            [solidPreset, { generate: "universal", moduleName: "@hewel/zgui/renderer", hydratable: false }],
            [typescriptPreset, { onlyRemoveTypeImports: true, allExtensions: true, isTSX: /\.tsx$/.test(filename) }],
          ],
        });
        if (!transformed?.code) throw new Error(`Babel emitted no JavaScript for ${filename}`);
        validateModule(transformed.code, { filename });
        const result = { code: transformed.code, map: transformed.map, moduleType: "js" };
        transforms.set(filename, { source, result });
        return result;
      },
    }, ...plugins],
  };
}

export function outputOptions(filename = "app.mjs") {
  return { format: "esm", entryFileNames: filename, sourcemap: "hidden", codeSplitting: false, minify: false };
}

// A native graph, not an emulation of another bundler's metadata schema.
export function compilationResult(output, moduleIds, { entry = true } = {}) {
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
  return { files, graph, solidRuntime };
}
