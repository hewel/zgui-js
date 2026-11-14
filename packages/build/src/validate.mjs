import { parse } from "@babel/parser";
import traverseModule from "@babel/traverse";
import { isBuiltin } from "node:module";

const traverse = traverseModule.default ?? traverseModule;
const globals = new Set([
  "undefined", "NaN", "Infinity", "globalThis", "__zgui", "console",
  "setTimeout", "clearTimeout", "requestAnimationFrame", "cancelAnimationFrame", "queueMicrotask",
  "Object", "Function", "Boolean", "Symbol", "Error", "AggregateError", "EvalError", "RangeError",
  "ReferenceError", "SyntaxError", "TypeError", "URIError", "Number", "BigInt", "Math", "Date", "String",
  "RegExp", "Array", "Int8Array", "Uint8Array", "Uint8ClampedArray", "Int16Array", "Uint16Array",
  "Int32Array", "Uint32Array", "Float32Array", "Float64Array", "BigInt64Array", "BigUint64Array",
  "Map", "Set", "WeakMap", "WeakSet", "ArrayBuffer", "SharedArrayBuffer", "DataView", "Atomics", "JSON",
  "Promise", "Reflect", "Proxy", "WeakRef", "FinalizationRegistry", "decodeURI", "decodeURIComponent",
  "encodeURI", "encodeURIComponent", "escape", "unescape", "eval", "isFinite", "isNaN", "parseFloat", "parseInt",
]);

function executesAtTopLevel(p) {
  let child = p;
  for (let parent = p.parentPath; parent; child = parent, parent = parent.parentPath) {
    // Method computed keys execute outside the method, unlike its body/parameters.
    if (parent.isFunction() && (child.key === "body" || child.listKey === "params")) return false;
  }
  return true;
}

export function validateImport(specifier) {
  if (isBuiltin(specifier) || /^(?:solid-js\/(?:web|server)|react-dom)(?:\/|$)/.test(specifier) ||
      /(?:^|\/)server(?:\.[cm]?js|\/|$)/.test(specifier)) {
    throw new Error(`Incompatible runtime import: ${specifier}`);
  }
  if (/\.(?:css|png|jpe?g|svg|woff2?|ttf|gif|webp)(?:[?#]|$)/i.test(specifier)) {
    throw new Error(`Runtime resource imports are unsupported; use app.css/assets: ${specifier}`);
  }
}

export function validateModule(code, { filename = "app.mjs", bundled = false, entry = false } = {}) {
  const ast = parse(code, { sourceType: "module", sourceFilename: filename, createImportExpressions: true });
  const exported = new Map();
  let program;
  traverse(ast, {
    Program(p) { program = p; },
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
      } else if (declaration?.id) exported.set(declaration.id.name, declaration.id.name);
      for (const item of p.node.specifiers) {
        if (item.type === "ExportSpecifier") exported.set(item.exported.name ?? item.exported.value, item.local.name);
      }
    },
    ImportExpression() { throw new Error("Dynamic imports are unsupported"); },
    AwaitExpression(p) {
      if (executesAtTopLevel(p)) throw new Error("Top-level await is unsupported");
    },
    ForOfStatement(p) {
      if (p.node.await && executesAtTopLevel(p)) throw new Error("Top-level await is unsupported");
    },
    CallExpression(p) {
      if (p.node.callee.type === "Identifier" && p.node.callee.name === "require" && !p.scope.hasBinding("require")) {
        throw new Error("Runtime require is unsupported");
      }
    },
    ReferencedIdentifier(p) {
      if (bundled && !p.scope.hasBinding(p.node.name, true) && !globals.has(p.node.name)) {
        throw new Error(`Unsupported runtime global: ${p.node.name}`);
      }
    },
  });
  if (entry) {
    const version = program.scope.getBinding(exported.get("hostApiVersion"));
    if (!version?.constant || version.path.node.type !== "VariableDeclarator" ||
        version.path.node.init?.type !== "NumericLiteral" || version.path.node.init.value !== 1) {
      throw new Error("Entry must export constant hostApiVersion = 1");
    }
    const mount = program.scope.getBinding(exported.get("mount"));
    const fn = mount?.path.node.type === "VariableDeclarator" ? mount.path.node.init : mount?.path.node;
    if (!mount?.constant || !fn || !["FunctionDeclaration", "FunctionExpression", "ArrowFunctionExpression"].includes(fn.type) || fn.async || fn.generator) {
      throw new Error("Entry must export a synchronous mount function");
    }
  }
  return ast;
}

export function validateGraph(graph) {
  const runtimes = graph.modules.filter(input => /\/solid-js\/dist\/(?:solid|dev)\.js$/.test(input));
  if (runtimes.length !== 1) throw new Error(`Expected one reactive Solid runtime, found ${runtimes.length}`);
  for (const input of graph.modules) {
    if (/\/solid-js\/(?:web|.*server)|\/react-dom\//.test(input)) throw new Error(`Incompatible module graph: ${input}`);
  }
  if (graph.chunks.length !== 1 || graph.chunks[0].imports.length || graph.chunks[0].dynamicImports.length) {
    throw new Error("Expected one self-contained ESM artifact");
  }
  return runtimes[0];
}
