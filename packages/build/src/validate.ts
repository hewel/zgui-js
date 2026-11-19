import { parse, type ParseResult } from "@babel/parser";
import traverseModule from "@babel/traverse";
import type { NodePath } from "@babel/traverse";
import type { types as t } from "@babel/core";
import { isBuiltin } from "node:module";

const traverse =
  (traverseModule as typeof traverseModule & { default?: typeof traverseModule }).default ??
  traverseModule;
const globals = new Set([
  "undefined",
  "NaN",
  "Infinity",
  "globalThis",
  "__zgui",
  "console",
  "performance",
  "setTimeout",
  "clearTimeout",
  "requestAnimationFrame",
  "cancelAnimationFrame",
  "queueMicrotask",
  "Object",
  "Function",
  "Boolean",
  "Symbol",
  "Error",
  "AggregateError",
  "EvalError",
  "RangeError",
  "ReferenceError",
  "SyntaxError",
  "TypeError",
  "URIError",
  "Number",
  "BigInt",
  "Math",
  "Date",
  "String",
  "RegExp",
  "Array",
  "Int8Array",
  "Uint8Array",
  "Uint8ClampedArray",
  "Int16Array",
  "Uint16Array",
  "Int32Array",
  "Uint32Array",
  "Float32Array",
  "Float64Array",
  "BigInt64Array",
  "BigUint64Array",
  "Map",
  "Set",
  "WeakMap",
  "WeakSet",
  "ArrayBuffer",
  "SharedArrayBuffer",
  "DataView",
  "Atomics",
  "JSON",
  "Promise",
  "Reflect",
  "Proxy",
  "WeakRef",
  "FinalizationRegistry",
  "decodeURI",
  "decodeURIComponent",
  "encodeURI",
  "encodeURIComponent",
  "escape",
  "unescape",
  "eval",
  "isFinite",
  "isNaN",
  "parseFloat",
  "parseInt",
]);

export interface CompilationChunk {
  fileName: string;
  modules: string[];
  imports: string[];
  dynamicImports: string[];
  exports: string[];
}

export interface CompilationGraph {
  modules: string[];
  chunks: CompilationChunk[];
}

function executesAtTopLevel(p: NodePath): boolean {
  let child: NodePath = p;
  for (let parent = p.parentPath; parent; child = parent, parent = parent.parentPath) {
    // Method computed keys execute outside the method, unlike its body/parameters.
    if (parent.isFunction() && (child.key === "body" || child.listKey === "params")) return false;
  }
  return true;
}

function hasArgumentsBinding(p: NodePath): boolean {
  let child: NodePath = p;
  for (let parent = p.parentPath; parent; child = parent, parent = parent.parentPath) {
    if (
      parent.isFunction() &&
      !parent.isArrowFunctionExpression() &&
      (child.key === "body" || child.listKey === "params")
    )
      return true;
  }
  return false;
}

function hasTypeofCheck(
  expr: t.Node | null | undefined,
  name: string,
  expectedUndefined: boolean,
): boolean {
  if (!expr) return false;
  if (expr.type === "LogicalExpression" && expr.operator === "&&" && !expectedUndefined) {
    return hasTypeofCheck(expr.left, name, false) || hasTypeofCheck(expr.right, name, false);
  }
  if (expr.type === "LogicalExpression" && expr.operator === "||" && expectedUndefined) {
    return hasTypeofCheck(expr.left, name, true) || hasTypeofCheck(expr.right, name, true);
  }
  if (expr.type !== "BinaryExpression") return false;
  const op = expr.operator;
  const matches = expectedUndefined ? op === "===" || op === "==" : op === "!==" || op === "!=";
  if (!matches) return false;
  return (
    (expr.left.type === "UnaryExpression" &&
      expr.left.operator === "typeof" &&
      expr.left.argument.type === "Identifier" &&
      expr.left.argument.name === name &&
      expr.right.type === "StringLiteral" &&
      expr.right.value === "undefined") ||
    (expr.right.type === "UnaryExpression" &&
      expr.right.operator === "typeof" &&
      expr.right.argument.type === "Identifier" &&
      expr.right.argument.name === name &&
      expr.left.type === "StringLiteral" &&
      expr.left.value === "undefined")
  );
}

function isTypeofGuarded(p: NodePath<t.Identifier | t.JSXIdentifier>): boolean {
  const name = p.node.name;
  if (p.parentPath.isUnaryExpression({ operator: "typeof" }) && p.key === "argument") return true;
  let child: NodePath = p;
  for (let cur: NodePath | null = p.parentPath; cur; child = cur, cur = cur.parentPath) {
    if (cur.isLogicalExpression()) {
      if (
        (cur.node.operator === "||" &&
          child.key === "right" &&
          hasTypeofCheck(cur.node.left, name, true)) ||
        (cur.node.operator === "&&" &&
          child.key === "right" &&
          hasTypeofCheck(cur.node.left, name, false))
      ) {
        return true;
      }
    }
    if (cur.isIfStatement() || cur.isConditionalExpression()) {
      if (child.key === "consequent" && hasTypeofCheck(cur.node.test, name, false)) return true;
      if (child.key === "alternate" && hasTypeofCheck(cur.node.test, name, true)) return true;
    }
    if (cur.isFunction() || cur.isProgram()) break;
  }
  return false;
}

export function validateImport(specifier: string): void {
  if (
    /^@stylexjs\/(?:babel-plugin|shared)(?:\/|$)/.test(specifier) ||
    /^@stylexjs\/stylex\/(?:lib\/)?(?:stylex-inject|stylesheet)(?:[./]|$)/.test(specifier)
  ) {
    throw new Error(`Unavailable native StyleX host import: ${specifier}`);
  }
  if (
    isBuiltin(specifier) ||
    /^(?:solid-js\/(?:web|server)|react-dom)(?:\/|$)/.test(specifier) ||
    /(?:^|\/)server(?:\.[cm]?js|\/|$)/.test(specifier)
  ) {
    throw new Error(`Incompatible runtime import: ${specifier}`);
  }
  if (/\.(?:css|png|jpe?g|svg|woff2?|ttf|gif|webp)(?:[?#]|$)/i.test(specifier)) {
    throw new Error(`Runtime resource imports are unsupported; use app.css/assets: ${specifier}`);
  }
}

export function validateModule(
  code: string,
  {
    filename = "app.mjs",
    bundled = false,
    entry = false,
  }: {
    filename?: string;
    bundled?: boolean;
    entry?: boolean;
  } = {},
): ParseResult<t.File> {
  const ast = parse(code, {
    sourceType: "module",
    sourceFilename: filename,
    createImportExpressions: true,
  });
  const exported = new Map<string, string>();
  let program!: NodePath<t.Program>;
  traverse(ast, {
    Program(p) {
      program = p;
    },
    ImportDeclaration(p) {
      validateImport(p.node.source.value);
      if (bundled) throw new Error("Bundle contains an unresolved runtime import");
    },
    ExportAllDeclaration(p) {
      validateImport(p.node.source.value);
      if (bundled) throw new Error("Bundle contains an external re-export");
    },
    ExportNamedDeclaration(p) {
      if (p.node.source) {
        validateImport(p.node.source.value);
        if (bundled) throw new Error("Bundle contains an external re-export");
      }
      const declaration = p.node.declaration;
      if (declaration?.type === "VariableDeclaration") {
        for (const item of declaration.declarations) {
          if (item.id.type === "Identifier") exported.set(item.id.name, item.id.name);
        }
      } else if (declaration && "id" in declaration && declaration.id?.type === "Identifier") {
        exported.set(declaration.id.name, declaration.id.name);
      }
      for (const item of p.node.specifiers) {
        if (item.type === "ExportSpecifier")
          exported.set(
            item.exported.type === "Identifier" ? item.exported.name : item.exported.value,
            item.local.name,
          );
      }
    },
    ImportExpression() {
      throw new Error("Dynamic imports are unsupported");
    },
    AwaitExpression(p) {
      if (executesAtTopLevel(p)) throw new Error("Top-level await is unsupported");
    },
    ForOfStatement(p) {
      if (p.node.await && executesAtTopLevel(p)) throw new Error("Top-level await is unsupported");
    },
    CallExpression(p) {
      if (
        p.node.callee.type === "Identifier" &&
        p.node.callee.name === "require" &&
        !p.scope.hasBinding("require")
      ) {
        throw new Error("Runtime require is unsupported");
      }
    },
    ReferencedIdentifier(p) {
      if (isTypeofGuarded(p)) return;
      if (
        bundled &&
        !p.scope.hasBinding(p.node.name, true) &&
        !globals.has(p.node.name) &&
        !(p.node.name === "arguments" && hasArgumentsBinding(p))
      ) {
        throw new Error(`Unsupported runtime global: ${p.node.name}`);
      }
    },
  });
  if (entry) {
    const version = program.scope.getBinding(exported.get("hostApiVersion") ?? "");
    if (
      !version?.constant ||
      version.path.node.type !== "VariableDeclarator" ||
      version.path.node.init?.type !== "NumericLiteral" ||
      version.path.node.init.value !== 1
    ) {
      throw new Error("Entry must export constant hostApiVersion = 1");
    }
    const mount = program.scope.getBinding(exported.get("mount") ?? "");
    const fn =
      mount?.path.node.type === "VariableDeclarator" ? mount.path.node.init : mount?.path.node;
    if (
      !mount?.constant ||
      !fn ||
      (fn.type !== "FunctionDeclaration" &&
        fn.type !== "FunctionExpression" &&
        fn.type !== "ArrowFunctionExpression") ||
      fn.async ||
      fn.generator
    ) {
      throw new Error("Entry must export a synchronous mount function");
    }
  }
  return ast;
}

export function validateGraph(graph: CompilationGraph): string {
  const runtimes = graph.modules.filter(input =>
    /\/solid-js\/dist\/(?:solid(?:\.dev|\.observe)?|dev)\.js$/.test(input),
  );
  const signals = graph.modules.filter(input =>
    /\/@solidjs\/signals\/dist\/(?:(?:prod|observe)\/index|dev)\.js$/.test(input),
  );
  if (runtimes.length !== 1 && signals.length !== 1) {
    throw new Error(
      `Expected one reactive Solid runtime, found solid=${runtimes.length}, signals=${signals.length}`,
    );
  }
  if (runtimes.length > 1 || signals.length > 1) {
    throw new Error(
      `Expected single Solid runtime instances, found solid=${runtimes.length}, signals=${signals.length}`,
    );
  }
  for (const input of graph.modules) {
    if (
      /\/@stylexjs\/(?:babel-plugin|shared)\//.test(input) ||
      /\/@stylexjs\/stylex\/lib\/(?:(?:es|cjs)\/(?:inject|stylesheet)|stylex-inject|stylesheet)(?:[./]|$)/.test(
        input,
      )
    ) {
      throw new Error(`Unavailable native StyleX host module: ${input}`);
    }
    if (/\/solid-js\/(?:web|.*server)|\/react-dom\//.test(input))
      throw new Error(`Incompatible module graph: ${input}`);
  }
  if (
    graph.chunks.length !== 1 ||
    graph.chunks[0].imports.length ||
    graph.chunks[0].dynamicImports.length
  ) {
    throw new Error("Expected one self-contained ESM artifact");
  }
  return runtimes[0] ?? signals[0];
}
