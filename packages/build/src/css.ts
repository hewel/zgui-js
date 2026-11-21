import { createRequire } from "node:module";
import { readdir, readFile, unlink, writeFile } from "node:fs/promises";
import path from "node:path";
import { Features, transform, type MediaCondition, type SelectorComponent } from "lightningcss";
import { canonicalJson, moduleIdentity, sha256 } from "./identity.ts";

const require = createRequire(import.meta.url);
const lightningcssPkg = require("lightningcss/package.json") as { version: string };

export const CSS_MODULES_PATTERN = "[hash]_[local]";

export interface StyleSystem {
  name: "lightningcss";
  profile: "lightningcss-v1";
  configDigest: string;
}

export interface TransformedCss {
  css: string;
  exports: Record<string, string>;
}

const STYLE_CONFIG = {
  name: "lightningcss",
  profile: "lightningcss-v1",
  lightningcssVersion: lightningcssPkg.version,
  cssModules: {
    pattern: CSS_MODULES_PATTERN,
    dashedIdents: false,
  },
  nesting: true,
  targets: null,
} as const;

const STYLE_CONFIG_DIGEST = sha256(canonicalJson(STYLE_CONFIG));

export function styleSystem(): StyleSystem {
  return {
    name: "lightningcss",
    profile: "lightningcss-v1",
    configDigest: STYLE_CONFIG_DIGEST,
  };
}

function hasPrefersReducedMotion(condition: MediaCondition | null | undefined): boolean {
  if (!condition) return false;
  switch (condition.type) {
    case "feature": {
      const feat = condition.value;
      if (feat.type === "plain" || feat.type === "range" || feat.type === "boolean") {
        return feat.name.toLowerCase() === "prefers-reduced-motion";
      }
      if (feat.type === "interval") {
        return feat.name.toLowerCase() === "prefers-reduced-motion";
      }
      return false;
    }
    case "not":
      return hasPrefersReducedMotion(condition.value);
    case "operation":
      return condition.conditions.some(item => hasPrefersReducedMotion(item));
    default:
      return false;
  }
}

function selectorContainsDirAttr(components: readonly SelectorComponent[]): boolean {
  for (const part of components) {
    if (part.type === "attribute" && part.name.toLowerCase() === "dir") return true;
    if (part.type === "pseudo-class" && "selectors" in part && Array.isArray(part.selectors)) {
      for (const nested of part.selectors) {
        if (Array.isArray(nested) && selectorContainsDirAttr(nested as SelectorComponent[])) {
          return true;
        }
      }
    }
  }
  return false;
}

function checkSelector(
  components: readonly SelectorComponent[],
  filename: string,
  cssModules: boolean,
): void {
  let hasHtmlType = false;
  let hasDirAttr = false;

  const flushCompound = (): void => {
    if (hasHtmlType && hasDirAttr) {
      throw new Error(`Unsupported native CSS feature in ${filename}: html[dir]`);
    }
    hasHtmlType = false;
    hasDirAttr = false;
  };

  for (const part of components) {
    if (part.type === "combinator") {
      flushCompound();
      continue;
    }
    if (part.type === "type" && part.name.toLowerCase() === "html") {
      hasHtmlType = true;
    }
    if (part.type === "attribute" && part.name.toLowerCase() === "dir") {
      hasDirAttr = true;
    }
    if (cssModules && part.type === "pseudo-class") {
      if (part.kind === "custom" && part.name === "export") {
        throw new Error(`Unsupported CSS Modules ICSS :export rule in ${filename}`);
      }
      if (part.kind === "custom-function" && part.name === "import") {
        throw new Error(`Unsupported CSS Modules ICSS :import rule in ${filename}`);
      }
    }
    if (part.type === "pseudo-class" && "selectors" in part && Array.isArray(part.selectors)) {
      for (const nested of part.selectors) {
        if (Array.isArray(nested)) {
          const list = nested as SelectorComponent[];
          if (selectorContainsDirAttr(list)) hasDirAttr = true;
          checkSelector(list, filename, cssModules);
        }
      }
    }
  }
  flushCompound();
}

/**
 * Validates and transforms native CSS or a locally scoped `*.module.css` file via Lightning CSS.
 * Runs with no Browserslist targets so Stylo keeps native tokens (`oklch`, `color-mix`, `--zui-*`),
 * flattens CSS nesting (`&`), and rejects unsupported Stylo/ICSS features at the AST level.
 */
export function transformStylesheet(
  css: string,
  {
    filename,
    cssModules = false,
  }: {
    filename: string;
    cssModules?: boolean;
  },
): TransformedCss {
  let result;
  try {
    result = transform({
      filename,
      code: Buffer.from(css, "utf8"),
      include: Features.Nesting,
      cssModules: cssModules
        ? {
            pattern: CSS_MODULES_PATTERN,
            dashedIdents: false,
          }
        : false,
      visitor: {
        Rule(rule) {
          switch (rule.type) {
            case "import":
              throw new Error(`Unsupported native CSS feature in ${filename}: @import`);
            case "font-face":
              throw new Error(`Unsupported native CSS feature in ${filename}: @font-face`);
            case "container":
              throw new Error(`Unsupported native CSS feature in ${filename}: @container`);
            case "property":
              throw new Error(`Unsupported native CSS feature in ${filename}: @property`);
            case "unknown":
              if (rule.value.name.toLowerCase() === "value") {
                throw new Error(`Unsupported CSS Modules @value rule in ${filename}`);
              }
              break;
            default:
              break;
          }
        },
        Url(u) {
          throw new Error(`Unsupported native CSS feature in ${filename}: url(${u.url})`);
        },
        Image(img) {
          if (img.type === "image-set") {
            throw new Error(`Unsupported native CSS feature in ${filename}: image-set()`);
          }
        },
        Function(fn) {
          const lower = fn.name.toLowerCase();
          if (lower === "image-set" || lower === "-webkit-image-set") {
            throw new Error(`Unsupported native CSS feature in ${filename}: ${fn.name}()`);
          }
        },
        MediaQuery(mq) {
          if (hasPrefersReducedMotion(mq.condition)) {
            throw new Error(
              `Unsupported native CSS feature in ${filename}: prefers-reduced-motion`,
            );
          }
        },
        Selector(sel) {
          checkSelector(sel, filename, cssModules);
        },
      },
    });
  } catch (error) {
    if (error instanceof Error && error.message.startsWith("Unsupported ")) {
      throw error;
    }
    const message = error instanceof Error ? error.message : String(error);
    throw new Error(`CSS compilation failed for ${filename}: ${message}`);
  }

  const exportsMap: Record<string, string> = {};
  if (cssModules && result.exports) {
    for (const key of Object.keys(result.exports).sort()) {
      const entry = result.exports[key];
      if (!entry) continue;
      if (entry.composes.length > 0) {
        throw new Error(
          `Unsupported CSS Modules feature in ${filename}: 'composes' on '.${key}' is not supported in v1`,
        );
      }
      exportsMap[key] = entry.name;
    }
  }

  return {
    css: Buffer.from(result.code).toString("utf8").trim(),
    exports: exportsMap,
  };
}

export function validateStylesheet(css: string, filename = "app.css"): string {
  return transformStylesheet(css, { filename, cssModules: false }).css;
}

const VALID_TS_PROP = /^[A-Za-z_$][A-Za-z0-9_$]*$/;

export function formatCssModuleDts(exportsMap: Readonly<Record<string, string>>): string {
  const keys = Object.keys(exportsMap).sort();
  if (keys.length === 0) {
    return [
      "// Generated by @hewel/zgui-build. Do not edit manually.",
      "declare const styles: Readonly<Record<never, never>>;",
      "export default styles;",
      "",
    ].join("\n");
  }
  const lines = [
    "// Generated by @hewel/zgui-build. Do not edit manually.",
    "declare const styles: {",
    ...keys.map(key => {
      const prop = VALID_TS_PROP.test(key) ? key : JSON.stringify(key);
      return `  readonly ${prop}: string;`;
    }),
    "};",
    "export default styles;",
    "",
  ];
  return lines.join("\n");
}

async function scanCssModules(
  directory: string,
  modules: string[],
  declarations: string[],
): Promise<void> {
  let entries;
  try {
    entries = await readdir(directory, { withFileTypes: true });
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return;
    throw error;
  }
  entries.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
  for (const entry of entries) {
    if (
      entry.name === "node_modules" ||
      entry.name === "dist" ||
      entry.name === ".zgui-generations" ||
      entry.name === ".git"
    ) {
      continue;
    }
    const full = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      await scanCssModules(full, modules, declarations);
    } else if (entry.isFile()) {
      if (entry.name.endsWith(".module.css")) {
        modules.push(full);
      } else if (entry.name.endsWith(".module.css.d.ts")) {
        declarations.push(full);
      }
    }
  }
}

/**
 * Generates exact `*.module.css.d.ts` declarations for every `*.module.css` file under `src/`,
 * skipping writes when unchanged (watch-safe) and deleting stale declarations on rename/delete.
 */
export async function syncCssModuleTypes(
  appDirectory: string,
  { root }: { root: string },
): Promise<string[]> {
  const srcDir = path.join(appDirectory, "src");
  const modules: string[] = [];
  const existingDts: string[] = [];
  await scanCssModules(srcDir, modules, existingDts);

  const expectedDts = new Set<string>();
  for (const cssPath of modules) {
    const source = await readFile(cssPath, "utf8");
    const idKey = moduleIdentity(cssPath, { root, appDirectory });
    const { exports: exportsMap } = transformStylesheet(source, {
      filename: idKey,
      cssModules: true,
    });
    const dtsPath = `${cssPath}.d.ts`;
    expectedDts.add(dtsPath);
    const nextContent = formatCssModuleDts(exportsMap);
    let currentContent: string | undefined;
    try {
      currentContent = await readFile(dtsPath, "utf8");
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
    if (currentContent !== nextContent) {
      await writeFile(dtsPath, nextContent, "utf8");
    }
  }

  for (const dtsPath of existingDts) {
    if (!expectedDts.has(dtsPath)) {
      await unlink(dtsPath);
    }
  }

  return modules;
}
