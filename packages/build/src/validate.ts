import type { BuildInfo } from "./index.ts";
import { parseSync, type ParseResult, type ParserOptions } from "rolldown/utils";
import { isBuiltin } from "node:module";
import path from "node:path";

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

const workerDisallowedGlobals = new Set([
  "__zgui",
  "performance",
  "setTimeout",
  "clearTimeout",
  "requestAnimationFrame",
  "cancelAnimationFrame",
]);

const workerGlobals = new Set([
  ...[...globals].filter(name => !workerDisallowedGlobals.has(name)),
  "postMessage",
  "self",
]);

export type CompilationGraph = BuildInfo["graph"];

type AstNode = Record<string, unknown> & { type: string };

interface Binding {
  name: string;
  node: AstNode;
  constant: boolean;
}

interface Scope {
  parent: Scope | null;
  kind: "program" | "function" | "block";
  bindings: Map<string, Binding>;
}

interface AncestorFrame {
  node: AstNode;
  key: string;
}

function isAstNode(value: unknown): value is AstNode {
  return value !== null && typeof value === "object" && typeof (value as AstNode).type === "string";
}

function isStringLiteral(node: unknown): node is AstNode & { value: string } {
  return isAstNode(node) && node.type === "Literal" && typeof node.value === "string";
}

function isNumberLiteral(node: unknown): node is AstNode & { value: number } {
  return isAstNode(node) && node.type === "Literal" && typeof node.value === "number";
}

function isFunctionNode(node: AstNode): boolean {
  return (
    node.type === "FunctionDeclaration" ||
    node.type === "FunctionExpression" ||
    node.type === "ArrowFunctionExpression"
  );
}

function createScope(parent: Scope | null, kind: Scope["kind"]): Scope {
  return { parent, kind, bindings: new Map() };
}

function nearestVarScope(scope: Scope): Scope {
  let current: Scope = scope;
  while (current.kind === "block" && current.parent) {
    current = current.parent;
  }
  return current;
}

function addBinding(scope: Scope, name: string, node: AstNode): void {
  const existing = scope.bindings.get(name);
  if (existing) {
    if (existing.node === node) return;
    existing.constant = false;
    return;
  }
  scope.bindings.set(name, { name, node, constant: true });
}

function resolveBinding(scope: Scope, name: string): Binding | undefined {
  for (let current: Scope | null = scope; current; current = current.parent) {
    const found = current.bindings.get(name);
    if (found) return found;
  }
  return undefined;
}

function hasBinding(scope: Scope, name: string): boolean {
  return resolveBinding(scope, name) !== undefined;
}

function collectPatternNames(pattern: unknown, names: string[]): void {
  if (!isAstNode(pattern)) return;
  switch (pattern.type) {
    case "Identifier":
      names.push(pattern.name as string);
      break;
    case "RestElement":
      collectPatternNames(pattern.argument, names);
      break;
    case "AssignmentPattern":
      collectPatternNames(pattern.left, names);
      break;
    case "ArrayPattern":
      for (const element of (pattern.elements as unknown[]) ?? []) {
        collectPatternNames(element, names);
      }
      break;
    case "ObjectPattern":
      for (const prop of (pattern.properties as unknown[]) ?? []) {
        if (!isAstNode(prop)) continue;
        if (prop.type === "RestElement") collectPatternNames(prop.argument, names);
        else if (prop.type === "Property") collectPatternNames(prop.value, names);
      }
      break;
    case "TSParameterProperty":
      collectPatternNames(pattern.parameter, names);
      break;
  }
}

function bindPattern(scope: Scope, pattern: unknown, node: AstNode): void {
  const names: string[] = [];
  collectPatternNames(pattern, names);
  for (const name of names) {
    addBinding(scope, name, node);
  }
}

function collectVarDeclarations(node: unknown, varScope: Scope): void {
  if (!isAstNode(node)) return;
  if (isFunctionNode(node) || node.type === "StaticBlock") return;
  if (node.type === "VariableDeclaration") {
    if (node.declare === true) return;
    if (node.kind === "var") {
      for (const decl of (node.declarations as unknown[]) ?? []) {
        if (isAstNode(decl)) bindPattern(varScope, decl.id, decl);
      }
    }
    return;
  }
  switch (node.type) {
    case "BlockStatement":
    case "Program":
      for (const stmt of (node.body as unknown[]) ?? []) collectVarDeclarations(stmt, varScope);
      break;
    case "ExportNamedDeclaration":
    case "ExportDefaultDeclaration":
      collectVarDeclarations(node.declaration, varScope);
      break;
    case "IfStatement":
      collectVarDeclarations(node.consequent, varScope);
      collectVarDeclarations(node.alternate, varScope);
      break;
    case "WhileStatement":
    case "DoWhileStatement":
    case "LabeledStatement":
    case "WithStatement":
      collectVarDeclarations(node.body, varScope);
      break;
    case "ForStatement":
      collectVarDeclarations(node.init, varScope);
      collectVarDeclarations(node.body, varScope);
      break;
    case "ForInStatement":
    case "ForOfStatement":
      collectVarDeclarations(node.left, varScope);
      collectVarDeclarations(node.body, varScope);
      break;
    case "SwitchStatement":
      for (const item of (node.cases as unknown[]) ?? []) {
        if (!isAstNode(item)) continue;
        for (const stmt of (item.consequent as unknown[]) ?? []) {
          collectVarDeclarations(stmt, varScope);
        }
      }
      break;
    case "TryStatement":
      collectVarDeclarations(node.block, varScope);
      if (isAstNode(node.handler)) collectVarDeclarations(node.handler.body, varScope);
      collectVarDeclarations(node.finalizer, varScope);
      break;
  }
}

function hoistStatementList(statements: unknown[], scope: Scope): void {
  const varScope = nearestVarScope(scope);
  for (const stmt of statements) {
    if (!isAstNode(stmt)) continue;
    if (stmt.declare === true) continue;
    switch (stmt.type) {
      case "ImportDeclaration": {
        if (stmt.importKind === "type") break;
        for (const spec of (stmt.specifiers as unknown[]) ?? []) {
          if (!isAstNode(spec)) continue;
          if (spec.importKind === "type") continue;
          if (isAstNode(spec.local) && spec.local.type === "Identifier") {
            addBinding(scope, spec.local.name as string, spec);
          }
        }
        break;
      }
      case "VariableDeclaration": {
        const target = stmt.kind === "var" ? varScope : scope;
        for (const decl of (stmt.declarations as unknown[]) ?? []) {
          if (isAstNode(decl)) bindPattern(target, decl.id, decl);
        }
        break;
      }
      case "FunctionDeclaration":
      case "ClassDeclaration":
      case "TSEnumDeclaration":
      case "TSModuleDeclaration":
        if (isAstNode(stmt.id) && stmt.id.type === "Identifier") {
          addBinding(scope, stmt.id.name as string, stmt);
        }
        break;
      case "ExportNamedDeclaration":
        if (stmt.exportKind === "type") break;
        if (isAstNode(stmt.declaration)) hoistStatementList([stmt.declaration], scope);
        break;
      case "ExportDefaultDeclaration":
        if (
          isAstNode(stmt.declaration) &&
          (stmt.declaration.type === "FunctionDeclaration" ||
            stmt.declaration.type === "ClassDeclaration") &&
          isAstNode(stmt.declaration.id) &&
          stmt.declaration.id.type === "Identifier"
        ) {
          addBinding(scope, stmt.declaration.id.name as string, stmt.declaration);
        }
        break;
      default:
        collectVarDeclarations(stmt, varScope);
        break;
    }
  }
}

function executesAtTopLevel(ancestors: readonly AncestorFrame[]): boolean {
  for (let i = ancestors.length - 1; i >= 0; i--) {
    const frame = ancestors[i]!;
    // Method computed keys execute outside the method, unlike its body/parameters.
    if (isFunctionNode(frame.node) && (frame.key === "body" || frame.key === "params")) {
      return false;
    }
  }
  return true;
}

function hasArgumentsBinding(ancestors: readonly AncestorFrame[]): boolean {
  for (let i = ancestors.length - 1; i >= 0; i--) {
    const frame = ancestors[i]!;
    if (
      (frame.node.type === "FunctionDeclaration" || frame.node.type === "FunctionExpression") &&
      (frame.key === "body" || frame.key === "params")
    ) {
      return true;
    }
  }
  return false;
}

function hasTypeofCheck(expr: unknown, name: string, expectedUndefined: boolean): boolean {
  if (!isAstNode(expr)) return false;
  if (expr.type === "ParenthesizedExpression") {
    return hasTypeofCheck(expr.expression, name, expectedUndefined);
  }
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
  const left = expr.left;
  const right = expr.right;
  return (
    (isAstNode(left) &&
      left.type === "UnaryExpression" &&
      left.operator === "typeof" &&
      isAstNode(left.argument) &&
      left.argument.type === "Identifier" &&
      left.argument.name === name &&
      isStringLiteral(right) &&
      right.value === "undefined") ||
    (isAstNode(right) &&
      right.type === "UnaryExpression" &&
      right.operator === "typeof" &&
      isAstNode(right.argument) &&
      right.argument.type === "Identifier" &&
      right.argument.name === name &&
      isStringLiteral(left) &&
      left.value === "undefined")
  );
}

function isTypeofGuarded(name: string, ancestors: readonly AncestorFrame[]): boolean {
  const parent = ancestors.at(-1);
  if (
    parent &&
    parent.node.type === "UnaryExpression" &&
    parent.node.operator === "typeof" &&
    parent.key === "argument"
  ) {
    return true;
  }
  for (let i = ancestors.length - 1; i >= 0; i--) {
    const frame = ancestors[i]!;
    if (frame.node.type === "LogicalExpression") {
      if (
        (frame.node.operator === "||" &&
          frame.key === "right" &&
          hasTypeofCheck(frame.node.left, name, true)) ||
        (frame.node.operator === "&&" &&
          frame.key === "right" &&
          hasTypeofCheck(frame.node.left, name, false))
      ) {
        return true;
      }
    }
    if (frame.node.type === "IfStatement" || frame.node.type === "ConditionalExpression") {
      if (frame.key === "consequent" && hasTypeofCheck(frame.node.test, name, false)) return true;
      if (frame.key === "alternate" && hasTypeofCheck(frame.node.test, name, true)) return true;
    }
    if (isFunctionNode(frame.node) || frame.node.type === "Program") break;
  }
  return false;
}

export function validateImport(specifier: string): void {
  if (
    isBuiltin(specifier) ||
    /^(?:solid-js\/(?:web|server)|react-dom)(?:\/|$)/.test(specifier) ||
    /(?:^|\/)server(?:\.[cm]?js|\/|$)/.test(specifier)
  ) {
    throw new Error(`Incompatible runtime import: ${specifier}`);
  }
  if (/\.module\.css$/i.test(specifier)) {
    return;
  }
  if (/\.(?:css|png|jpe?g|svg|woff2?|ttf|gif|webp)(?:[?#]|$)/i.test(specifier)) {
    throw new Error(`Runtime resource imports are unsupported; use app.css/assets: ${specifier}`);
  }
}

export function validateWorkerImport(specifier: string): void {
  if (/\.module\.css$/i.test(specifier)) {
    throw new Error(`Compute worker cannot import CSS module: ${specifier}`);
  }
  validateImport(specifier);
  if (
    /^@hewel\/zgui(?:$|\/|-services(?:$|\/))/.test(specifier) ||
    /^solid-js(?:$|\/)/.test(specifier) ||
    /^@solidjs\//.test(specifier)
  ) {
    throw new Error(`Compute worker cannot import UI, Solid, or host service module: ${specifier}`);
  }
}

function resolveChunkSpecifier(fromFilename: string, specifier: string): string | null {
  if (!specifier.startsWith("./") && !specifier.startsWith("../")) return null;
  const fromDir = path.posix.dirname(fromFilename);
  const resolved = path.posix.normalize(
    fromDir === "." ? specifier : path.posix.join(fromDir, specifier),
  );
  if (resolved.startsWith("..") || resolved.startsWith("/")) return null;
  return resolved;
}

function inferLang(filename: string, bundled: boolean): NonNullable<ParserOptions["lang"]> {
  if (bundled) return "js";
  if (/\.[cm]?tsx$/i.test(filename)) return "tsx";
  if (/\.[cm]?ts$/i.test(filename)) return "ts";
  if (/\.[cm]?jsx$/i.test(filename)) return "jsx";
  return "js";
}

function unwrapTsExpressions(node: unknown): AstNode | null {
  let current = node;
  while (isAstNode(current)) {
    if (
      current.type === "TSAsExpression" ||
      current.type === "TSSatisfiesExpression" ||
      current.type === "TSTypeAssertion" ||
      current.type === "TSNonNullExpression" ||
      current.type === "ParenthesizedExpression"
    ) {
      current = current.expression;
      continue;
    }
    return current;
  }
  return null;
}

export function validateModule(
  code: string,
  {
    filename = "app.mjs",
    bundled = false,
    entry = false,
    worker = false,
    allowedChunks,
    lang,
  }: {
    filename?: string;
    bundled?: boolean;
    entry?: boolean;
    worker?: boolean;
    allowedChunks?: ReadonlySet<string>;
    lang?: ParserOptions["lang"];
  } = {},
): ParseResult {
  const allowedGlobals = worker ? workerGlobals : globals;
  const parsed = parseSync(filename, code, {
    sourceType: "module",
    lang: lang ?? inferLang(filename, bundled),
  });
  if (parsed.errors.length > 0) {
    throw new SyntaxError(
      `Failed to parse ${filename}: ${parsed.errors.map(error => error.message).join("\n")}`,
    );
  }

  const programNode = parsed.program as unknown as AstNode;
  const programScope = createScope(null, "program");
  const exported = new Map<string, string>();
  const ancestors: AncestorFrame[] = [];

  const assertClosedChunkEdge = (specifier: string, errorMessage: string): void => {
    const resolved = resolveChunkSpecifier(filename, specifier);
    if (!resolved || !allowedChunks?.has(resolved)) {
      throw new Error(errorMessage);
    }
  };

  function checkIdentifierReference(name: string, scope: Scope): void {
    if (isTypeofGuarded(name, ancestors)) return;
    if (
      bundled &&
      !hasBinding(scope, name) &&
      !allowedGlobals.has(name) &&
      !(name === "arguments" && hasArgumentsBinding(ancestors))
    ) {
      throw new Error(`Unsupported runtime global: ${name}`);
    }
  }

  function markWrite(name: string, scope: Scope): void {
    const binding = resolveBinding(scope, name);
    if (binding) binding.constant = false;
  }

  function pushAndWalk(parent: AstNode, key: string, child: unknown, scope: Scope): void {
    if (!isAstNode(child)) return;
    ancestors.push({ node: parent, key });
    try {
      walkNode(child, scope);
    } finally {
      ancestors.pop();
    }
  }

  function walkPattern(
    parent: AstNode,
    key: string,
    pattern: unknown,
    scope: Scope,
    isAssignmentTarget: boolean,
  ): void {
    if (!isAstNode(pattern)) return;
    ancestors.push({ node: parent, key });
    try {
      switch (pattern.type) {
        case "Identifier":
          if (isAssignmentTarget) {
            const name = pattern.name as string;
            markWrite(name, scope);
            checkIdentifierReference(name, scope);
          }
          break;
        case "MemberExpression":
          // Assignment target like `obj.x = 1` or `obj[k] = 1` does not reassign `obj`.
          walkNode(pattern, scope);
          break;
        case "RestElement":
          walkPattern(pattern, "argument", pattern.argument, scope, isAssignmentTarget);
          break;
        case "AssignmentPattern":
          walkPattern(pattern, "left", pattern.left, scope, isAssignmentTarget);
          pushAndWalk(pattern, "right", pattern.right, scope);
          break;
        case "ArrayPattern":
          for (const element of (pattern.elements as unknown[]) ?? []) {
            walkPattern(pattern, "elements", element, scope, isAssignmentTarget);
          }
          break;
        case "ObjectPattern":
          for (const prop of (pattern.properties as unknown[]) ?? []) {
            if (!isAstNode(prop)) continue;
            if (prop.type === "RestElement") {
              walkPattern(prop, "argument", prop.argument, scope, isAssignmentTarget);
            } else if (prop.type === "Property") {
              if (prop.computed === true) pushAndWalk(prop, "key", prop.key, scope);
              walkPattern(prop, "value", prop.value, scope, isAssignmentTarget);
            }
          }
          break;
        case "TSParameterProperty":
          walkPattern(pattern, "parameter", pattern.parameter, scope, isAssignmentTarget);
          break;
        case "TSAsExpression":
        case "TSSatisfiesExpression":
        case "TSTypeAssertion":
        case "TSNonNullExpression":
        case "ParenthesizedExpression":
          walkPattern(pattern, "expression", pattern.expression, scope, isAssignmentTarget);
          break;
      }
    } finally {
      ancestors.pop();
    }
  }

  function walkFunction(node: AstNode, outerScope: Scope): void {
    const fnScope = createScope(outerScope, "function");
    if (node.type === "FunctionExpression" && isAstNode(node.id) && node.id.type === "Identifier") {
      addBinding(fnScope, node.id.name as string, node);
    }
    const params = (node.params as unknown[]) ?? [];
    for (const param of params) {
      if (isAstNode(param)) bindPattern(fnScope, param, param);
    }
    for (const param of params) {
      walkPattern(node, "params", param, fnScope, false);
    }
    if (isAstNode(node.body)) {
      if (node.body.type === "BlockStatement") {
        const bodyStatements = (node.body.body as unknown[]) ?? [];
        const bodyScope = createScope(fnScope, "block");
        hoistStatementList(bodyStatements, bodyScope);
        ancestors.push({ node, key: "body" });
        try {
          for (const stmt of bodyStatements) {
            pushAndWalk(node.body, "body", stmt, bodyScope);
          }
        } finally {
          ancestors.pop();
        }
      } else {
        pushAndWalk(node, "body", node.body, fnScope);
      }
    }
  }

  function walkClass(node: AstNode, outerScope: Scope): void {
    if (Array.isArray(node.decorators)) {
      for (const dec of node.decorators) pushAndWalk(node, "decorators", dec, outerScope);
    }
    pushAndWalk(node, "superClass", node.superClass, outerScope);
    const classScope =
      node.type === "ClassExpression" && isAstNode(node.id) && node.id.type === "Identifier"
        ? createScope(outerScope, "block")
        : outerScope;
    if (classScope !== outerScope && isAstNode(node.id) && node.id.type === "Identifier") {
      addBinding(classScope, node.id.name as string, node);
    }
    if (!isAstNode(node.body)) return;
    ancestors.push({ node, key: "body" });
    try {
      for (const member of (node.body.body as unknown[]) ?? []) {
        if (!isAstNode(member)) continue;
        ancestors.push({ node: node.body, key: "body" });
        try {
          if (Array.isArray(member.decorators)) {
            for (const dec of member.decorators) pushAndWalk(member, "decorators", dec, classScope);
          }
          if (
            member.type === "MethodDefinition" ||
            member.type === "PropertyDefinition" ||
            member.type === "AccessorProperty"
          ) {
            if (member.computed === true) pushAndWalk(member, "key", member.key, classScope);
            pushAndWalk(member, "value", member.value, classScope);
          } else if (member.type === "StaticBlock") {
            const staticScope = createScope(classScope, "function");
            const stmts = (member.body as unknown[]) ?? [];
            hoistStatementList(stmts, staticScope);
            for (const stmt of stmts) pushAndWalk(member, "body", stmt, staticScope);
          }
        } finally {
          ancestors.pop();
        }
      }
    } finally {
      ancestors.pop();
    }
  }

  function walkNode(node: AstNode, scope: Scope): void {
    if (node.declare === true) return;
    switch (node.type) {
      case "Program": {
        const body = (node.body as unknown[]) ?? [];
        hoistStatementList(body, scope);
        for (const stmt of body) pushAndWalk(node, "body", stmt, scope);
        break;
      }
      case "BlockStatement": {
        const blockScope = createScope(scope, "block");
        const body = (node.body as unknown[]) ?? [];
        hoistStatementList(body, blockScope);
        for (const stmt of body) pushAndWalk(node, "body", stmt, blockScope);
        break;
      }
      case "ImportDeclaration": {
        if (node.importKind === "type") break;
        if (isStringLiteral(node.source)) {
          validateImport(node.source.value);
          if (bundled) {
            assertClosedChunkEdge(
              node.source.value,
              "Bundle contains an unresolved runtime import",
            );
          }
        }
        break;
      }
      case "ExportAllDeclaration": {
        if (node.exportKind === "type") break;
        if (isStringLiteral(node.source)) {
          validateImport(node.source.value);
          if (bundled) {
            assertClosedChunkEdge(node.source.value, "Bundle contains an external re-export");
          }
        }
        break;
      }
      case "ExportNamedDeclaration": {
        if (node.exportKind === "type") break;
        const specifiers = (node.specifiers as unknown[]) ?? [];
        const allTypeSpecifiers =
          specifiers.length > 0 &&
          specifiers.every(spec => isAstNode(spec) && spec.exportKind === "type");
        if (isStringLiteral(node.source)) {
          if (allTypeSpecifiers) break;
          validateImport(node.source.value);
          if (bundled) {
            assertClosedChunkEdge(node.source.value, "Bundle contains an external re-export");
          }
        } else {
          for (const spec of specifiers) {
            if (!isAstNode(spec) || spec.type !== "ExportSpecifier") continue;
            if (spec.exportKind === "type") continue;
            if (isAstNode(spec.local) && spec.local.type === "Identifier") {
              const exportedName =
                isAstNode(spec.exported) && spec.exported.type === "Identifier"
                  ? (spec.exported.name as string)
                  : isStringLiteral(spec.exported)
                    ? spec.exported.value
                    : "";
              if (exportedName) exported.set(exportedName, spec.local.name as string);
              pushAndWalk(spec, "local", spec.local, scope);
            }
          }
        }
        if (isAstNode(node.declaration)) {
          const decl = node.declaration;
          if (decl.type === "VariableDeclaration") {
            for (const item of (decl.declarations as unknown[]) ?? []) {
              if (isAstNode(item) && isAstNode(item.id) && item.id.type === "Identifier") {
                exported.set(item.id.name as string, item.id.name as string);
              }
            }
          } else if (isAstNode(decl.id) && decl.id.type === "Identifier") {
            exported.set(decl.id.name as string, decl.id.name as string);
          }
          pushAndWalk(node, "declaration", decl, scope);
        }
        break;
      }
      case "ExportDefaultDeclaration":
        pushAndWalk(node, "declaration", node.declaration, scope);
        break;
      case "ImportExpression": {
        if (worker) {
          throw new Error("Dynamic imports are unsupported in compute workers");
        }
        if (!isStringLiteral(node.source)) {
          throw new Error("Dynamic imports must use a static string literal specifier");
        }
        validateImport(node.source.value);
        if (bundled) {
          assertClosedChunkEdge(node.source.value, "Bundle contains an unresolved dynamic import");
        }
        pushAndWalk(node, "options", node.options, scope);
        break;
      }
      case "AwaitExpression":
        if (executesAtTopLevel(ancestors)) throw new Error("Top-level await is unsupported");
        pushAndWalk(node, "argument", node.argument, scope);
        break;
      case "ForOfStatement": {
        if (node.await === true && executesAtTopLevel(ancestors)) {
          throw new Error("Top-level await is unsupported");
        }
        const forScope = createScope(scope, "block");
        if (isAstNode(node.left) && node.left.type === "VariableDeclaration") {
          hoistStatementList([node.left], forScope);
          pushAndWalk(node, "left", node.left, forScope);
        } else {
          walkPattern(node, "left", node.left, forScope, true);
        }
        pushAndWalk(node, "right", node.right, forScope);
        pushAndWalk(node, "body", node.body, forScope);
        break;
      }
      case "ForInStatement": {
        const forScope = createScope(scope, "block");
        if (isAstNode(node.left) && node.left.type === "VariableDeclaration") {
          hoistStatementList([node.left], forScope);
          pushAndWalk(node, "left", node.left, forScope);
        } else {
          walkPattern(node, "left", node.left, forScope, true);
        }
        pushAndWalk(node, "right", node.right, forScope);
        pushAndWalk(node, "body", node.body, forScope);
        break;
      }
      case "ForStatement": {
        const forScope = createScope(scope, "block");
        if (isAstNode(node.init) && node.init.type === "VariableDeclaration") {
          hoistStatementList([node.init], forScope);
        }
        pushAndWalk(node, "init", node.init, forScope);
        pushAndWalk(node, "test", node.test, forScope);
        pushAndWalk(node, "update", node.update, forScope);
        pushAndWalk(node, "body", node.body, forScope);
        break;
      }
      case "CallExpression": {
        if (
          isAstNode(node.callee) &&
          node.callee.type === "Identifier" &&
          node.callee.name === "require" &&
          !hasBinding(scope, "require")
        ) {
          throw new Error("Runtime require is unsupported");
        }
        pushAndWalk(node, "callee", node.callee, scope);
        for (const arg of (node.arguments as unknown[]) ?? []) {
          pushAndWalk(node, "arguments", arg, scope);
        }
        break;
      }
      case "NewExpression":
        pushAndWalk(node, "callee", node.callee, scope);
        for (const arg of (node.arguments as unknown[]) ?? []) {
          pushAndWalk(node, "arguments", arg, scope);
        }
        break;
      case "Identifier":
        checkIdentifierReference(node.name as string, scope);
        break;
      case "VariableDeclaration":
        for (const decl of (node.declarations as unknown[]) ?? []) {
          pushAndWalk(node, "declarations", decl, scope);
        }
        break;
      case "VariableDeclarator":
        walkPattern(node, "id", node.id, scope, false);
        pushAndWalk(node, "init", node.init, scope);
        break;
      case "FunctionDeclaration":
      case "FunctionExpression":
      case "ArrowFunctionExpression":
        walkFunction(node, scope);
        break;
      case "ClassDeclaration":
      case "ClassExpression":
        walkClass(node, scope);
        break;
      case "AssignmentExpression":
        walkPattern(node, "left", node.left, scope, true);
        pushAndWalk(node, "right", node.right, scope);
        break;
      case "UpdateExpression":
        if (isAstNode(node.argument) && node.argument.type === "Identifier") {
          markWrite(node.argument.name as string, scope);
        }
        pushAndWalk(node, "argument", node.argument, scope);
        break;
      case "MemberExpression":
        pushAndWalk(node, "object", node.object, scope);
        if (node.computed === true) pushAndWalk(node, "property", node.property, scope);
        break;
      case "ChainExpression":
      case "ExpressionStatement":
      case "ParenthesizedExpression":
      case "TSAsExpression":
      case "TSSatisfiesExpression":
      case "TSTypeAssertion":
      case "TSNonNullExpression":
      case "TSInstantiationExpression":
      case "TSExportAssignment":
        pushAndWalk(node, "expression", node.expression, scope);
        break;
      case "UnaryExpression":
      case "ReturnStatement":
      case "ThrowStatement":
      case "SpreadElement":
      case "YieldExpression":
        pushAndWalk(node, "argument", node.argument, scope);
        break;
      case "BinaryExpression":
      case "LogicalExpression":
        pushAndWalk(node, "left", node.left, scope);
        pushAndWalk(node, "right", node.right, scope);
        break;
      case "ConditionalExpression":
      case "IfStatement":
        pushAndWalk(node, "test", node.test, scope);
        pushAndWalk(node, "consequent", node.consequent, scope);
        pushAndWalk(node, "alternate", node.alternate, scope);
        break;
      case "WhileStatement":
      case "DoWhileStatement":
        pushAndWalk(node, "test", node.test, scope);
        pushAndWalk(node, "body", node.body, scope);
        break;
      case "LabeledStatement":
        pushAndWalk(node, "body", node.body, scope);
        break;
      case "SwitchStatement": {
        pushAndWalk(node, "discriminant", node.discriminant, scope);
        const switchScope = createScope(scope, "block");
        const cases = (node.cases as unknown[]) ?? [];
        for (const item of cases) {
          if (isAstNode(item))
            hoistStatementList((item.consequent as unknown[]) ?? [], switchScope);
        }
        for (const item of cases) {
          if (!isAstNode(item)) continue;
          pushAndWalk(item, "test", item.test, switchScope);
          for (const stmt of (item.consequent as unknown[]) ?? []) {
            pushAndWalk(item, "consequent", stmt, switchScope);
          }
        }
        break;
      }
      case "TryStatement":
        pushAndWalk(node, "block", node.block, scope);
        if (isAstNode(node.handler)) {
          const catchScope = createScope(scope, "block");
          if (isAstNode(node.handler.param)) {
            bindPattern(catchScope, node.handler.param, node.handler.param);
            walkPattern(node.handler, "param", node.handler.param, catchScope, false);
          }
          pushAndWalk(node.handler, "body", node.handler.body, catchScope);
        }
        pushAndWalk(node, "finalizer", node.finalizer, scope);
        break;
      case "ArrayExpression":
        for (const element of (node.elements as unknown[]) ?? []) {
          pushAndWalk(node, "elements", element, scope);
        }
        break;
      case "ObjectExpression":
        for (const prop of (node.properties as unknown[]) ?? []) {
          if (!isAstNode(prop)) continue;
          if (prop.type === "SpreadElement") {
            pushAndWalk(prop, "argument", prop.argument, scope);
          } else if (prop.type === "Property") {
            if (prop.computed === true) pushAndWalk(prop, "key", prop.key, scope);
            pushAndWalk(prop, "value", prop.value, scope);
          }
        }
        break;
      case "SequenceExpression":
      case "TemplateLiteral":
        for (const expr of (node.expressions as unknown[]) ?? []) {
          pushAndWalk(node, "expressions", expr, scope);
        }
        break;
      case "TaggedTemplateExpression":
        pushAndWalk(node, "tag", node.tag, scope);
        pushAndWalk(node, "quasi", node.quasi, scope);
        break;
      case "Decorator":
        pushAndWalk(node, "expression", node.expression, scope);
        break;
      case "TSEnumDeclaration":
        for (const member of (node.members as unknown[]) ?? []) {
          if (isAstNode(member)) pushAndWalk(member, "initializer", member.initializer, scope);
        }
        break;
      case "TSModuleDeclaration":
        if (isAstNode(node.body)) {
          const nsScope = createScope(scope, "block");
          if (node.body.type === "TSModuleBlock") {
            const body = (node.body.body as unknown[]) ?? [];
            hoistStatementList(body, nsScope);
            for (const stmt of body) pushAndWalk(node.body, "body", stmt, nsScope);
          } else {
            pushAndWalk(node, "body", node.body, nsScope);
          }
        }
        break;
      case "TSImportEqualsDeclaration":
        if (
          node.importKind !== "type" &&
          isAstNode(node.moduleReference) &&
          node.moduleReference.type === "TSExternalModuleReference" &&
          isStringLiteral(node.moduleReference.expression)
        ) {
          validateImport(node.moduleReference.expression.value);
        }
        break;
      case "Literal":
      case "ThisExpression":
      case "Super":
      case "MetaProperty":
      case "PrivateIdentifier":
      case "EmptyStatement":
      case "DebuggerStatement":
      case "BreakStatement":
      case "ContinueStatement":
        break;
      default:
        // Type-only TS* declarations and annotations have no runtime evaluation.
        if (node.type.startsWith("TS")) break;
        break;
    }
  }

  walkNode(programNode, programScope);

  if (entry) {
    const version = programScope.bindings.get(exported.get("hostApiVersion") ?? "");
    const versionInit =
      version?.node.type === "VariableDeclarator" ? unwrapTsExpressions(version.node.init) : null;
    if (!version?.constant || !isNumberLiteral(versionInit) || versionInit.value !== 1) {
      throw new Error("Entry must export constant hostApiVersion = 1");
    }
    const mount = programScope.bindings.get(exported.get("mount") ?? "");
    const mountNode =
      mount?.node.type === "VariableDeclarator"
        ? unwrapTsExpressions(mount.node.init)
        : (mount?.node ?? null);
    if (
      !mount?.constant ||
      !mountNode ||
      !isFunctionNode(mountNode) ||
      mountNode.async === true ||
      mountNode.generator === true
    ) {
      throw new Error("Entry must export a synchronous mount function");
    }
  }
  return parsed;
}

export function validateGraph(
  graph: CompilationGraph,
  { allowMultipleChunks = true }: { allowMultipleChunks?: boolean } = {},
): string {
  const runtimes = graph.modules.filter(input =>
    /\/solid-js\/dist\/(?:solid(?:\.dev|\.observe)?|dev)\.js$/.test(input),
  );
  const signals = graph.modules.filter(input =>
    /\/@solidjs\/signals\/dist\/(?:(?:prod|observe)\/index|dev)\.js$/.test(input),
  );
  if (runtimes.length > 1 || signals.length > 1) {
    const details = [
      ...(runtimes.length > 1 ? [`solid-js: ${runtimes.join(", ")}`] : []),
      ...(signals.length > 1 ? [`@solidjs/signals: ${signals.join(", ")}`] : []),
    ];
    throw new Error(
      `Expected single Solid runtime instances, found solid=${runtimes.length}, signals=${signals.length} (${details.join("; ")})`,
    );
  }
  if (runtimes.length !== 1 && signals.length !== 1) {
    throw new Error(
      `Expected one reactive Solid runtime, found solid=${runtimes.length}, signals=${signals.length}`,
    );
  }
  for (const input of graph.modules) {
    if (/\/solid-js\/(?:web|.*server)|\/react-dom\//.test(input))
      throw new Error(`Incompatible module graph: ${input}`);
  }
  if (graph.chunks.length === 0) {
    throw new Error("Expected at least one ESM artifact");
  }
  if (
    !allowMultipleChunks &&
    (graph.chunks.length !== 1 ||
      graph.chunks[0].imports.length ||
      graph.chunks[0].dynamicImports.length)
  ) {
    throw new Error("Expected one self-contained ESM artifact");
  }
  const knownChunks = new Set(graph.chunks.map(chunk => chunk.fileName));
  for (const chunk of graph.chunks) {
    for (const edge of [...chunk.imports, ...chunk.dynamicImports]) {
      if (!knownChunks.has(edge)) {
        throw new Error(`Chunk graph contains an unresolved external edge: ${edge}`);
      }
    }
  }
  return runtimes[0] ?? signals[0];
}
