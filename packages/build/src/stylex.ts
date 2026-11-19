import type { Manifest } from "./index.ts";
import stylexPlugin, { type Options, type Rule } from "@stylexjs/babel-plugin";
import { parse } from "@babel/parser";
import traverseModule from "@babel/traverse";
import { canonicalJson, sha256 } from "./identity.ts";
import { readFileSync } from "node:fs";
import path from "node:path";

const traverse =
  (traverseModule as typeof traverseModule & { default?: typeof traverseModule }).default ??
  traverseModule;
const admitted = new Set(["create", "props", "defineVars", "createTheme", "keyframes"]);

export type StyleSystem = Manifest["styleSystem"];

export interface StylexModule {
  rules: Rule[];
  identity: string;
}

export interface GeneratedStyleLocation {
  offset: number;
  line: number;
  column: number;
}

export interface StylexOrigins {
  schemaVersion: 1;
  stylesheet: "app.css";
  rules: {
    generated: { start: GeneratedStyleLocation; end: GeneratedStyleLocation };
    origins: { module: string }[];
  }[];
}

export interface FinalizedStylex {
  css: string;
  origins: StylexOrigins;
}

export type StylexConfiguration = {
  nativeProfileDigest: string;
  versions: Record<string, string>;
  dev: false;
  debug: boolean;
  enableDebugDataProp: boolean;
  treeshakeCompensation: true;
  dynamicPropertyRegistration: "universal-noninheriting";
  runtimeInjection: false;
  test: false;
  styleResolution: "property-specificity";
  sxPropName: false;
  moduleResolution: "package-relative-commonJS";
  themeFileExtension: ".stylex";
  finalizer: Readonly<{ useLayers: false; legacyDisableLayers: false }>;
};
export const finalizerOptions = Object.freeze({ useLayers: false, legacyDisableLayers: false });
export function readNativeProfileDigest(root: string): string {
  return sha256(
    canonicalJson(JSON.parse(readFileSync(path.join(root, "schemas/stylex-profile.json"), "utf8"))),
  );
}
export function stylexConfiguration(
  diagnostics: boolean,
  nativeProfileDigest: string,
): StylexConfiguration {
  return {
    nativeProfileDigest,
    versions: { "@stylexjs/stylex": "0.19.2", "@stylexjs/babel-plugin": "0.19.2", styleq: "0.2.1" },
    dev: false,
    debug: diagnostics,
    enableDebugDataProp: diagnostics,
    treeshakeCompensation: true,
    dynamicPropertyRegistration: "universal-noninheriting",
    runtimeInjection: false,
    test: false,
    styleResolution: "property-specificity",
    sxPropName: false,
    moduleResolution: "package-relative-commonJS",
    themeFileExtension: ".stylex",
    finalizer: finalizerOptions,
  };
}
export function styleSystem(diagnostics: boolean, nativeProfileDigest: string): StyleSystem {
  return {
    name: "stylex",
    profile: "zgui-stylex-v1",
    nativeProfileDigest,
    configDigest: sha256(canonicalJson(stylexConfiguration(diagnostics, nativeProfileDigest))),
  };
}
export function stylexOptions(
  appDirectory: string,
  diagnostics: boolean,
  debugFilePath: (filename: string) => string,
  nativeProfileDigest: string,
): Partial<Options> {
  const {
    versions: _versions,
    moduleResolution: _resolution,
    themeFileExtension,
    finalizer: _finalizer,
    dynamicPropertyRegistration: _registration,
    nativeProfileDigest: _nativeProfileDigest,
    ...options
  } = stylexConfiguration(diagnostics, nativeProfileDigest);
  return {
    ...options,
    debugFilePath,
    unstable_moduleResolution: { type: "commonJS", rootDir: appDirectory, themeFileExtension },
  };
}
export { stylexPlugin };

// Inspect import bindings, not words in strings/comments or unrelated objects named stylex.
export function validateStylexModule(
  code: string,
  { filename, compiled = false }: { filename: string; compiled?: boolean },
): void {
  const ast = parse(code, {
    sourceType: "module",
    plugins: /\.[cm]?ts$/.test(filename) ? ["typescript"] : ["typescript", "jsx"],
  });
  traverse(ast, {
    ImportDeclaration(p) {
      if (p.node.source.value !== "@stylexjs/stylex" || p.node.importKind === "type") return;
      for (const specifier of p.node.specifiers) {
        if (specifier.type === "ImportSpecifier" && specifier.importKind === "type") continue;
        const imported =
          specifier.type === "ImportSpecifier"
            ? specifier.imported.type === "Identifier"
              ? specifier.imported.name
              : specifier.imported.value
            : null;
        const binding = p.scope.getBinding(specifier.local.name);
        for (const reference of binding?.referencePaths ?? []) {
          if (reference.findParent(parent => parent.isTSType())) continue;
          let api: unknown = imported;
          if (api === null) {
            const member = reference.parentPath;
            if (!member?.isMemberExpression() || member.node.object !== reference.node) {
              throw new Error(`StyleX namespace escapes static compilation in ${filename}`);
            }
            const property = member.node.property;
            api = member.node.computed
              ? "value" in property
                ? property.value
                : undefined
              : "name" in property
                ? property.name
                : undefined;
          }
          if (typeof api !== "string" || !admitted.has(api))
            throw new Error(`Unsupported native StyleX API: ${String(api)}`);
          if (compiled && api !== "props")
            throw new Error(`Untransformed StyleX compile-time API: ${api}`);
        }
      }
    },
    ExportNamedDeclaration(p) {
      if (p.node.source?.value === "@stylexjs/stylex" && p.node.exportKind !== "type") {
        for (const specifier of p.node.specifiers) {
          if (specifier.type !== "ExportSpecifier" || specifier.local.name !== "props") {
            throw new Error("StyleX compile-time APIs must be imported directly, not re-exported");
          }
        }
      }
    },
    ExportAllDeclaration(p) {
      if (p.node.source.value === "@stylexjs/stylex")
        throw new Error("StyleX compile-time APIs must be imported directly, not re-exported");
    },
  });
}

// Native preflight remains authoritative. Reject browser-only output before publication as well.
export function validateStylesheet(
  css: string,
  filename: string,
  { allowDynamicProperties = false }: { allowDynamicProperties?: boolean } = {},
): void {
  let source = css.replace(/\/\*[\s\S]*?\*\//g, "");
  if (allowDynamicProperties) {
    // This exact registration is emitted unconditionally for dynamic functions in 0.19.2.
    source = source.replace(
      /@property\s+--[-_a-zA-Z0-9]+\s*\{\s*syntax\s*:\s*(["'])\*\1\s*;\s*inherits\s*:\s*false\s*;?\s*\}/g,
      "",
    );
  }
  const tokens = source.replace(/"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'/g, '""');
  if (
    /@(?:property|font-face|import)\b/i.test(tokens) ||
    /\b(?:url|(?:-webkit-)?image-set)\s*\(/i.test(tokens)
  ) {
    throw new Error(`Unsupported native CSS resource or typed variable feature in ${filename}`);
  }
  if (/(?:^|[{}\s,>+~(])html\s*(?:\[\s*dir\b|:not\(\s*\[\s*dir\b)/i.test(tokens)) {
    throw new Error(`Unsupported native StyleX directional html[dir] output in ${filename}`);
  }
}

function generatedRules(css: string): { start: number; end: number; text: string }[] {
  const rules: { start: number; end: number; text: string }[] = [];
  let start = 0,
    depth = 0,
    quote: string | null = null,
    comment = false;
  for (let i = 0; i < css.length; i++) {
    const character = css[i];
    if (comment) {
      if (character === "*" && css[i + 1] === "/") {
        comment = false;
        i++;
      }
      continue;
    }
    if (quote) {
      if (character === "\\") i++;
      else if (character === quote) quote = null;
      continue;
    }
    if (character === "/" && css[i + 1] === "*") {
      comment = true;
      i++;
      continue;
    }
    if (character === '"' || character === "'") {
      quote = character;
      continue;
    }
    if (character === "{") depth++;
    else if (character === "}" && --depth === 0) {
      while (/\s/.test(css[start] ?? "") && start < i) start++;
      rules.push({ start, end: i + 1, text: css.slice(start, i + 1) });
      start = i + 1;
    }
  }
  return rules;
}

export function finalizeStylex(modules: readonly StylexModule[]): FinalizedStylex {
  const rules = modules.flatMap(module => module.rules);
  const css = stylexPlugin.processStylexRules(rules, finalizerOptions);
  if (typeof css !== "string" || (rules.length > 0 && !css.trim()))
    throw new Error("StyleX finalizer emitted no stylesheet");
  validateStylesheet(css, "app.css", { allowDynamicProperties: true });
  const owners = new Map<string, Set<string>>();
  for (const module of modules) {
    for (const [key] of module.rules) {
      let identities = owners.get(key);
      if (!identities) owners.set(key, (identities = new Set()));
      identities.add(module.identity);
    }
  }
  const location = (offset: number): GeneratedStyleLocation => {
    const before = css.slice(0, offset);
    return {
      offset,
      line: before.split("\n").length,
      column: offset - before.lastIndexOf("\n") - 1,
    };
  };
  const origins: StylexOrigins = {
    schemaVersion: 1,
    stylesheet: "app.css",
    rules: generatedRules(css).map(rule => {
      const modules = new Set<string>();
      for (const token of rule.text.matchAll(/[-_a-zA-Z0-9]+/g)) {
        for (const owner of owners.get(token[0]) ?? []) modules.add(owner);
      }
      if (modules.size === 0) throw new Error("Cannot verify generated StyleX rule provenance");
      return {
        generated: { start: location(rule.start), end: location(rule.end) },
        origins: [...modules].sort().map(module => ({ module })),
      };
    }),
  };
  return { css, origins };
}
