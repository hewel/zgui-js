import {
  importedName,
  unwrapExpr,
  type AstNode,
  type AstProgram,
  type FunctionNode,
  type VariableDeclarationNode,
} from "./ast.js";

export type Binding =
  | { kind: "const"; init: AstNode | null; scope: Scope; mutated: boolean }
  | { kind: "import"; source: string; imported: string | null; scope: Scope; mutated: boolean }
  | { kind: "namespace"; source: string; imported: "*"; scope: Scope; mutated: boolean }
  | { kind: "other"; scope: Scope; mutated: boolean };

export interface Scope {
  parent: Scope | null;
  isFunctionScope: boolean;
  bindings: Map<string, Binding>;
}

export interface ScopeIndex {
  getScope(node: AstNode): Scope;
}

const RUNTIME_TS_EXPR_TYPES = new Set([
  "TSAsExpression",
  "TSSatisfiesExpression",
  "TSTypeAssertion",
  "TSNonNullExpression",
  "TSEnumDeclaration",
  "TSExportAssignment",
  "TSParameterProperty",
]);

function createScope(parent: Scope | null, isFunctionScope: boolean): Scope {
  return { parent, isFunctionScope, bindings: new Map() };
}

export function resolveBinding(scope: Scope, name: string): Binding | null {
  for (let cur: Scope | null = scope; cur; cur = cur.parent) {
    const binding = cur.bindings.get(name);
    if (binding) return binding;
  }
  return null;
}

function collectPatternNames(pattern: AstNode | null | undefined, out: string[] = []): string[] {
  if (!pattern) return out;
  switch (pattern.type) {
    case "Identifier":
      out.push(pattern.name);
      break;
    case "RestElement":
      collectPatternNames(pattern.argument, out);
      break;
    case "AssignmentPattern":
      collectPatternNames(pattern.left, out);
      break;
    case "ArrayPattern":
      for (const elem of pattern.elements ?? []) collectPatternNames(elem, out);
      break;
    case "ObjectPattern":
      for (const prop of pattern.properties ?? []) {
        if (prop.type === "Property") collectPatternNames(prop.value, out);
        else if (prop.type === "RestElement") collectPatternNames(prop.argument, out);
      }
      break;
  }
  return out;
}

function bindPatternNames(pattern: AstNode | null | undefined, scope: Scope): void {
  for (const name of collectPatternNames(pattern)) {
    scope.bindings.set(name, { kind: "other", scope, mutated: false });
  }
}

function registerVariableDecl(decl: VariableDeclarationNode, scope: Scope, varScope: Scope): void {
  if (decl.declare) return;
  const target = decl.kind === "var" ? varScope : scope;
  for (const item of decl.declarations ?? []) {
    if (decl.kind === "const" && item.id?.type === "Identifier") {
      const existing = target.bindings.get(item.id.name);
      target.bindings.set(item.id.name, {
        kind: "const",
        init: item.init ?? null,
        scope: target,
        mutated: existing?.mutated ?? false,
      });
    } else {
      bindPatternNames(item.id, target);
    }
  }
}

function hoistStatementDecl(stmt: AstNode | null | undefined, scope: Scope, varScope: Scope): void {
  if (!stmt) return;
  if (stmt.type === "ImportDeclaration") {
    if (stmt.importKind === "type" || typeof stmt.source?.value !== "string") return;
    const source = stmt.source.value;
    for (const spec of stmt.specifiers ?? []) {
      if (spec.type === "ImportSpecifier") {
        if (spec.importKind === "type") continue;
        scope.bindings.set(spec.local.name, {
          kind: "import",
          source,
          imported: importedName(spec),
          scope,
          mutated: false,
        });
      } else if (spec.type === "ImportNamespaceSpecifier") {
        scope.bindings.set(spec.local.name, {
          kind: "namespace",
          source,
          imported: "*",
          scope,
          mutated: false,
        });
      } else if (spec.type === "ImportDefaultSpecifier") {
        scope.bindings.set(spec.local.name, { kind: "other", scope, mutated: false });
      }
    }
    return;
  }
  if (stmt.type === "ExportNamedDeclaration" || stmt.type === "ExportDefaultDeclaration") {
    if (stmt.exportKind !== "type" && stmt.declaration) {
      hoistStatementDecl(stmt.declaration, scope, varScope);
    }
    return;
  }
  if (stmt.type === "VariableDeclaration") {
    registerVariableDecl(stmt, scope, varScope);
    return;
  }
  if (
    (stmt.type === "FunctionDeclaration" ||
      stmt.type === "ClassDeclaration" ||
      stmt.type === "TSEnumDeclaration") &&
    !stmt.declare &&
    stmt.id?.type === "Identifier"
  ) {
    scope.bindings.set(stmt.id.name, { kind: "other", scope, mutated: false });
  }
}

function rootMutatedIdentifier(expr: AstNode | null | undefined): string | null {
  const inner = unwrapExpr(expr);
  if (!inner) return null;
  if (inner.type === "Identifier") return inner.name;
  if (inner.type === "MemberExpression") return rootMutatedIdentifier(inner.object);
  return null;
}

function markMutated(expr: AstNode | null | undefined, scope: Scope): void {
  const name = rootMutatedIdentifier(expr);
  if (!name) return;
  const binding = resolveBinding(scope, name);
  if (binding) binding.mutated = true;
}

function isAstNodeLike(value: unknown): value is AstNode {
  return Boolean(
    value && typeof value === "object" && typeof (value as { type?: unknown }).type === "string",
  );
}

export function buildScopeIndex(program: AstProgram): ScopeIndex {
  const nodeScope = new WeakMap<AstNode, Scope>();
  const rootScope = createScope(null, true);

  function walkChildren(node: AstNode, scope: Scope, varScope: Scope): void {
    const record = node as unknown as Record<string, unknown>;
    for (const key of Object.keys(record)) {
      if (key === "parent" || key === "loc" || key === "range") continue;
      if (key === "typeAnnotation" || key === "typeParameters" || key === "returnType") continue;
      const value = record[key];
      if (!value || typeof value !== "object") continue;
      if (Array.isArray(value)) {
        for (const item of value) {
          if (isAstNodeLike(item)) walk(item, scope, varScope);
        }
      } else if (isAstNodeLike(value)) {
        walk(value, scope, varScope);
      }
    }
  }

  function walkFunction(node: FunctionNode, scope: Scope): void {
    const fnScope = createScope(scope, true);
    if (node.type === "FunctionExpression" && node.id?.type === "Identifier") {
      fnScope.bindings.set(node.id.name, { kind: "other", scope: fnScope, mutated: false });
    }
    for (const param of node.params ?? []) {
      const target = param.type === "TSParameterProperty" ? param.parameter : param;
      bindPatternNames(target, fnScope);
      walk(param, fnScope, fnScope);
    }
    if (node.body) walk(node.body, fnScope, fnScope);
  }

  function walk(node: AstNode | null | undefined, scope: Scope, varScope: Scope): void {
    if (!node || typeof node.type !== "string") return;
    if (node.type.startsWith("TS") && !RUNTIME_TS_EXPR_TYPES.has(node.type)) return;
    nodeScope.set(node, scope);

    switch (node.type) {
      case "Program":
        for (const stmt of node.body ?? []) hoistStatementDecl(stmt, rootScope, rootScope);
        for (const stmt of node.body ?? []) walk(stmt, rootScope, rootScope);
        return;
      case "ImportDeclaration":
        return;
      case "VariableDeclaration":
        registerVariableDecl(node, scope, varScope);
        walkChildren(node, scope, varScope);
        return;
      case "FunctionDeclaration":
        if (!node.declare) walkFunction(node, scope);
        return;
      case "FunctionExpression":
      case "ArrowFunctionExpression":
        walkFunction(node, scope);
        return;
      case "BlockStatement": {
        const blockScope = createScope(scope, false);
        for (const stmt of node.body ?? []) hoistStatementDecl(stmt, blockScope, varScope);
        for (const stmt of node.body ?? []) walk(stmt, blockScope, varScope);
        return;
      }
      case "ForStatement":
      case "ForInStatement":
      case "ForOfStatement": {
        const loopScope = createScope(scope, false);
        const head = node.type === "ForStatement" ? node.init : node.left;
        if (head?.type === "VariableDeclaration") registerVariableDecl(head, loopScope, varScope);
        walkChildren(node, loopScope, varScope);
        return;
      }
      case "CatchClause": {
        const catchScope = createScope(scope, false);
        if (node.param) bindPatternNames(node.param, catchScope);
        if (node.body) walk(node.body, catchScope, varScope);
        return;
      }
      case "SwitchStatement": {
        if (node.discriminant) walk(node.discriminant, scope, varScope);
        const switchScope = createScope(scope, false);
        for (const c of node.cases ?? []) {
          for (const stmt of c.consequent ?? []) hoistStatementDecl(stmt, switchScope, varScope);
        }
        for (const c of node.cases ?? []) walk(c, switchScope, varScope);
        return;
      }
      case "StaticBlock": {
        const staticScope = createScope(scope, true);
        for (const stmt of node.body ?? []) hoistStatementDecl(stmt, staticScope, staticScope);
        for (const stmt of node.body ?? []) walk(stmt, staticScope, staticScope);
        return;
      }
      case "ClassExpression": {
        if (node.superClass) walk(node.superClass, scope, varScope);
        const classScope = createScope(scope, false);
        if (node.id?.type === "Identifier") {
          classScope.bindings.set(node.id.name, {
            kind: "other",
            scope: classScope,
            mutated: false,
          });
        }
        if (node.body) walk(node.body, classScope, varScope);
        return;
      }
      case "AssignmentExpression":
        markMutated(node.left, scope);
        walkChildren(node, scope, varScope);
        return;
      case "UpdateExpression":
        markMutated(node.argument, scope);
        walkChildren(node, scope, varScope);
        return;
      case "UnaryExpression":
        if (node.operator === "delete") markMutated(node.argument, scope);
        walkChildren(node, scope, varScope);
        return;
      case "CallExpression":
        if (
          node.callee.type === "MemberExpression" &&
          !node.callee.computed &&
          node.callee.object.type === "Identifier" &&
          node.callee.object.name === "Object" &&
          node.callee.property.type === "Identifier" &&
          node.callee.property.name === "assign" &&
          node.arguments.length > 0
        ) {
          markMutated(node.arguments[0], scope);
        }
        walkChildren(node, scope, varScope);
        return;
      default:
        walkChildren(node, scope, varScope);
    }
  }

  walk(program, rootScope, rootScope);
  return { getScope: node => nodeScope.get(node) ?? rootScope };
}
