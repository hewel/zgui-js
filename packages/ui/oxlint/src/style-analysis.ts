import {
  unwrapExpr,
  type AstNode,
  type JSXAttributeNode,
  type JSXOpeningElementNode,
  type ObjectExpressionNode,
} from "./ast.js";
import { resolveBinding, type Binding, type Scope } from "./scope.js";

export const NATIVE_JSX_TAGS = new Set([
  "box",
  "row",
  "column",
  "stack",
  "text",
  "label",
  "scroll",
  "control",
  "field",
  "editor",
  "image",
  "vector",
]);

export const ZGUI_COMPONENT_SOURCES = new Set([
  "@hewel/zgui",
  "@hewel/zgui/components",
  "@hewel/zgui/primitives",
]);

type StyleEntryKind = "static" | "conditional-static" | "nullish" | "dynamic";

interface StyleEntry {
  kind: StyleEntryKind;
  key: string;
  normalizedKey: string;
}

interface StyleObjectInfo {
  entries: Map<string, StyleEntry>;
  hasDynamic: boolean;
  hasUnknownSpread: boolean;
}

export type StyleFinding =
  | { kind: "static-string" }
  | { kind: "static-object"; keys: string[] }
  | { kind: "mixed-object"; staticKeys: string[] }
  | { kind: "conditional-static"; condKeys: string[] }
  | { kind: "mixed-and-conditional"; staticKeys: string[]; condKeys: string[] };

function resolveConstExpr(
  expr: AstNode | null | undefined,
  scope: Scope,
  visited = new Set<Binding>(),
): { expr: AstNode | null; scope: Scope } {
  const inner = unwrapExpr(expr);
  if (!inner) return { expr: null, scope };
  if (inner.type === "Identifier") {
    const binding = resolveBinding(scope, inner.name);
    if (binding?.kind === "const" && !binding.mutated && binding.init && !visited.has(binding)) {
      const next = new Set(visited);
      next.add(binding);
      return resolveConstExpr(binding.init, binding.scope, next);
    }
  }
  return { expr: inner, scope };
}

function isNullishLiteral(expr: AstNode | null | undefined, scope: Scope): boolean {
  if (!expr) return false;
  if (expr.type === "Literal") return expr.value === null;
  return (
    expr.type === "Identifier" && expr.name === "undefined" && !resolveBinding(scope, "undefined")
  );
}

export function extractStaticString(
  expr: AstNode | null | undefined,
  scope: Scope,
  visited = new Set<Binding>(),
): string | null {
  const { expr: node } = resolveConstExpr(expr, scope, visited);
  if (!node) return null;
  if (node.type === "Literal" && typeof node.value === "string") return node.value;
  if (node.type === "TemplateLiteral" && node.expressions.length === 0) {
    return node.quasis[0]?.value?.cooked ?? node.quasis[0]?.value?.raw ?? "";
  }
  return null;
}

function extractStaticNumber(
  expr: AstNode | null | undefined,
  scope: Scope,
  visited = new Set<Binding>(),
): number | null {
  const { expr: node } = resolveConstExpr(expr, scope, visited);
  if (!node) return null;
  if (node.type === "Literal" && typeof node.value === "number") return node.value;
  if (node.type === "UnaryExpression" && node.operator === "-") {
    const arg = unwrapExpr(node.argument);
    if (arg?.type === "Literal" && typeof arg.value === "number") return -arg.value;
  }
  return null;
}

function extractStaticPrimitive(
  expr: AstNode | null | undefined,
  scope: Scope,
  visited = new Set<Binding>(),
): string | number | null {
  const str = extractStaticString(expr, scope, visited);
  if (str !== null) return str;
  return extractStaticNumber(expr, scope, visited);
}

function isFiniteStaticConditional(
  expr: AstNode | null | undefined,
  scope: Scope,
  visited = new Set<Binding>(),
): boolean {
  const { expr: inner, scope: innerScope } = resolveConstExpr(expr, scope, visited);
  if (!inner || inner.type !== "ConditionalExpression") return false;
  const seen = new Set<AstNode>();
  const checkBranch = (branch: AstNode, branchScope: Scope): boolean => {
    const { expr: b, scope: bScope } = resolveConstExpr(branch, branchScope, visited);
    if (!b || seen.has(b)) return false;
    if (b.type === "ConditionalExpression") {
      seen.add(b);
      return checkBranch(b.consequent, bScope) && checkBranch(b.alternate, bScope);
    }
    const str = extractStaticString(b, bScope, visited);
    if (str !== null) return str.trim().length > 0;
    return extractStaticNumber(b, bScope, visited) !== null;
  };
  seen.add(inner);
  return checkBranch(inner.consequent, innerScope) && checkBranch(inner.alternate, innerScope);
}

function normalizeStyleKey(rawKey: string): string {
  if (rawKey.startsWith("--")) return rawKey;
  return rawKey.replace(/[A-Z]/g, ch => `-${ch.toLowerCase()}`);
}

function analyzeStyleObject(
  objNode: ObjectExpressionNode,
  objScope: Scope,
  seenObjects = new Set<ObjectExpressionNode>(),
): StyleObjectInfo {
  const entries = new Map<string, StyleEntry>();
  let hasDynamic = false;
  let hasUnknownSpread = false;

  for (const prop of objNode.properties ?? []) {
    if (prop.type === "SpreadElement") {
      const { expr: arg, scope: argScope } = resolveConstExpr(prop.argument, objScope);
      if (arg?.type === "ObjectExpression" && !seenObjects.has(arg)) {
        const nextSeen = new Set(seenObjects);
        nextSeen.add(arg);
        const sub = analyzeStyleObject(arg, argScope, nextSeen);
        for (const [k, v] of sub.entries) entries.set(k, v);
        if (sub.hasDynamic) hasDynamic = true;
        if (sub.hasUnknownSpread) hasUnknownSpread = true;
      } else if (!isNullishLiteral(arg, argScope)) {
        hasUnknownSpread = true;
      }
      continue;
    }
    if (prop.type !== "Property") continue;

    let rawKey: string | null = null;
    if (prop.computed) {
      rawKey = extractStaticString(prop.key, objScope);
    } else if (prop.key?.type === "Identifier") {
      rawKey = prop.key.name;
    } else if (prop.key?.type === "Literal") {
      rawKey = String(prop.key.value);
    }
    if (rawKey === null) {
      hasDynamic = true;
      continue;
    }

    const normalizedKey = normalizeStyleKey(rawKey);
    if (prop.kind !== "init") {
      entries.set(normalizedKey, { kind: "dynamic", key: rawKey, normalizedKey });
      hasDynamic = true;
      continue;
    }

    const { expr: valExpr, scope: valScope } = resolveConstExpr(prop.value, objScope);
    if (isNullishLiteral(valExpr, valScope)) {
      entries.set(normalizedKey, { kind: "nullish", key: rawKey, normalizedKey });
    } else if (extractStaticPrimitive(prop.value, objScope) !== null) {
      entries.set(normalizedKey, { kind: "static", key: rawKey, normalizedKey });
    } else if (!normalizedKey.startsWith("--") && isFiniteStaticConditional(valExpr, valScope)) {
      entries.set(normalizedKey, { kind: "conditional-static", key: rawKey, normalizedKey });
    } else {
      entries.set(normalizedKey, { kind: "dynamic", key: rawKey, normalizedKey });
      hasDynamic = true;
    }
  }
  return { entries, hasDynamic, hasUnknownSpread };
}

type ConditionalLeaf = { kind: "nullish" } | { kind: "object"; info: StyleObjectInfo };

function collectTopLevelConditionalObjects(expr: AstNode, scope: Scope): string[] | null {
  const leaves: ConditionalLeaf[] = [];
  const seen = new Set<AstNode>();
  const visit = (curExpr: AstNode, curScope: Scope): boolean => {
    const { expr: resolved, scope: resolvedScope } = resolveConstExpr(curExpr, curScope);
    if (!resolved || seen.has(resolved)) return false;
    if (resolved.type === "ConditionalExpression") {
      seen.add(resolved);
      return visit(resolved.consequent, resolvedScope) && visit(resolved.alternate, resolvedScope);
    }
    if (isNullishLiteral(resolved, resolvedScope)) {
      leaves.push({ kind: "nullish" });
      return true;
    }
    if (resolved.type === "ObjectExpression") {
      const info = analyzeStyleObject(resolved, resolvedScope);
      const condCount = [...info.entries.values()].filter(
        e => e.kind === "conditional-static",
      ).length;
      if (info.hasDynamic || info.hasUnknownSpread || condCount > 0) return false;
      leaves.push({ kind: "object", info });
      return true;
    }
    return false;
  };
  if (!visit(expr, scope)) return null;
  const nonEmptyObjects = leaves.filter(
    (leaf): leaf is { kind: "object"; info: StyleObjectInfo } =>
      leaf.kind === "object" && [...leaf.info.entries.values()].some(e => e.kind === "static"),
  );
  if (nonEmptyObjects.length === 0) return null;
  return nonEmptyObjects.flatMap(o =>
    [...o.info.entries.values()].filter(e => e.kind === "static").map(e => e.key),
  );
}

function isReportableStaticKey(normalizedKey: string): boolean {
  return !normalizedKey.startsWith("--") || normalizedKey.startsWith("--zgui-");
}

export function describeTargetJsxElement(
  openingEl: JSXOpeningElementNode,
  scope: Scope,
): string | null {
  const name = openingEl.name;
  if (name.type === "JSXIdentifier") {
    if (NATIVE_JSX_TAGS.has(name.name)) return name.name;
    const binding = resolveBinding(scope, name.name);
    if (binding?.kind === "import" && ZGUI_COMPONENT_SOURCES.has(binding.source)) {
      return name.name;
    }
    return null;
  }
  if (name.type === "JSXMemberExpression") {
    const parts: string[] = [];
    let cur: JSXOpeningElementNode["name"] = name;
    while (cur.type === "JSXMemberExpression") {
      if (cur.property?.type !== "JSXIdentifier") return null;
      parts.unshift(cur.property.name);
      cur = cur.object;
    }
    if (cur.type !== "JSXIdentifier") return null;
    const binding = resolveBinding(scope, cur.name);
    if (binding?.kind === "namespace" && ZGUI_COMPONENT_SOURCES.has(binding.source)) {
      return `${cur.name}.${parts.join(".")}`;
    }
  }
  return null;
}

export function classifyStyleAttribute(
  attrValue: JSXAttributeNode["value"],
  scope: Scope,
): StyleFinding | null {
  if (!attrValue) return null;
  if (attrValue.type === "Literal") {
    if (typeof attrValue.value === "string" && attrValue.value.trim().length > 0) {
      return { kind: "static-string" };
    }
    return null;
  }
  if (attrValue.type !== "JSXExpressionContainer") return null;
  const expr = attrValue.expression;
  if (!expr || expr.type === "JSXEmptyExpression") return null;

  const { expr: resolved, scope: resolvedScope } = resolveConstExpr(expr, scope);
  if (!resolved) return null;

  const staticStr = extractStaticString(resolved, resolvedScope);
  if (staticStr !== null) {
    return staticStr.trim().length > 0 ? { kind: "static-string" } : null;
  }

  if (resolved.type === "ObjectExpression") {
    const info = analyzeStyleObject(resolved, resolvedScope);
    const staticEntries = [...info.entries.values()].filter(e => e.kind === "static");
    const condEntries = [...info.entries.values()].filter(e => e.kind === "conditional-static");
    if (!info.hasDynamic && !info.hasUnknownSpread && condEntries.length === 0) {
      if (staticEntries.length === 0) return null;
      return { kind: "static-object", keys: staticEntries.map(e => e.key) };
    }
    const reportableStatic = staticEntries
      .filter(e => isReportableStaticKey(e.normalizedKey))
      .map(e => e.key);
    const condKeys = condEntries.map(e => e.key);
    if (reportableStatic.length === 0 && condKeys.length === 0) return null;
    if (condKeys.length > 0 && reportableStatic.length === 0) {
      return { kind: "conditional-static", condKeys };
    }
    if (reportableStatic.length > 0 && condKeys.length === 0) {
      return { kind: "mixed-object", staticKeys: reportableStatic };
    }
    return { kind: "mixed-and-conditional", staticKeys: reportableStatic, condKeys };
  }

  if (resolved.type === "ConditionalExpression") {
    const keys = collectTopLevelConditionalObjects(resolved, resolvedScope);
    if (keys && keys.length > 0) {
      return { kind: "static-object", keys: [...new Set(keys)] };
    }
  }
  return null;
}
