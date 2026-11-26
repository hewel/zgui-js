import type { RuleTester } from "oxlint/plugins-dev";

export type OxlintRule = Parameters<RuleTester["run"]>[1];
export type CreateRule = Extract<OxlintRule, { create: unknown }>;
export type RuleContext = Parameters<CreateRule["create"]>[0];
export type RuleVisitor = ReturnType<CreateRule["create"]>;
export type AstProgram = RuleContext["sourceCode"]["ast"];
export type AstNode = Parameters<NonNullable<RuleVisitor[string]>>[0];
export type AstComment = AstProgram["comments"][number];

export type NodeOf<K extends keyof RuleVisitor> = Parameters<NonNullable<RuleVisitor[K]>>[0];

export type CallExpressionNode = NodeOf<"CallExpression">;
export type FunctionNode = NodeOf<"FunctionDeclaration"> | NodeOf<"ArrowFunctionExpression">;
export type ImportSpecifierNode = NodeOf<"ImportSpecifier">;
export type JSXOpeningElementNode = NodeOf<"JSXOpeningElement">;
export type JSXAttributeNode = NodeOf<"JSXAttribute">;
export type ObjectExpressionNode = NodeOf<"ObjectExpression">;
export type ObjectPropertyNode = Extract<
  ObjectExpressionNode["properties"][number],
  { type: "Property" }
>;
export type VariableDeclarationNode = NodeOf<"VariableDeclaration">;

export interface OxlintPlugin {
  meta: { name: string };
  rules: Record<string, OxlintRule>;
}

export function parentOf(node: AstNode): AstNode | null {
  if (!("parent" in node)) return null;
  return node.parent ?? null;
}

export function commentLines(comment: AstComment): number {
  return comment.loc.end.line - comment.loc.start.line + 1;
}

export function getAllComments(context: RuleContext): readonly AstComment[] {
  return context.sourceCode?.getAllComments?.() ?? [];
}

export function importedName(specifier: {
  type: string;
  imported?: { type: string; name?: string; value?: string };
}): string | null {
  if (specifier.type !== "ImportSpecifier" || !specifier.imported) return null;
  if (specifier.imported.type === "Identifier") return specifier.imported.name ?? null;
  return String(specifier.imported.value ?? "");
}

export function isFunctionNode(node: AstNode): node is FunctionNode {
  return (
    node.type === "FunctionDeclaration" ||
    node.type === "FunctionExpression" ||
    node.type === "ArrowFunctionExpression"
  );
}

export function isIife(fnNode: FunctionNode): boolean {
  let current = parentOf(fnNode);
  while (current && current.type === "ParenthesizedExpression") {
    current = parentOf(current);
  }
  return current?.type === "CallExpression" && current.callee === fnNode;
}

export function enclosingRefCallback(node: AstNode): FunctionNode | null {
  let fn: FunctionNode | null = null;
  for (let cur = parentOf(node); cur; cur = parentOf(cur)) {
    if (isFunctionNode(cur)) {
      if (isIife(cur)) continue;
      fn = cur;
      break;
    }
  }
  if (!fn) return null;
  for (let cur = parentOf(fn); cur; cur = parentOf(cur)) {
    if (cur.type === "JSXAttribute") {
      return cur.name?.type === "JSXIdentifier" && cur.name.name === "ref" ? fn : null;
    }
    if (
      cur.type === "JSXExpressionContainer" ||
      cur.type === "ArrayExpression" ||
      cur.type === "ParenthesizedExpression" ||
      cur.type === "ConditionalExpression" ||
      cur.type === "LogicalExpression" ||
      cur.type === "TSAsExpression" ||
      cur.type === "TSSatisfiesExpression"
    ) {
      continue;
    }
    return null;
  }
  return null;
}

export function isModuleTopLevel(node: AstNode): boolean {
  for (let cur = parentOf(node); cur; cur = parentOf(cur)) {
    if (isFunctionNode(cur)) {
      if (isIife(cur)) continue;
      return false;
    }
    if (
      cur.type === "MethodDefinition" ||
      cur.type === "PropertyDefinition" ||
      cur.type === "StaticBlock"
    ) {
      return false;
    }
  }
  return true;
}

export function calleeHelperName(callee: CallExpressionNode["callee"]): string | null {
  if (callee.type === "Identifier") return callee.name;
  if (
    callee.type === "MemberExpression" &&
    !callee.computed &&
    callee.property.type === "Identifier"
  ) {
    return callee.property.name;
  }
  return null;
}

export function findObjectGetterInSpread(
  expr: AstNode | null | undefined,
): ObjectPropertyNode | null {
  if (!expr) return null;
  switch (expr.type) {
    case "ParenthesizedExpression":
    case "TSAsExpression":
    case "TSSatisfiesExpression":
    case "TSTypeAssertion":
    case "TSNonNullExpression":
      return findObjectGetterInSpread(expr.expression);
    case "ConditionalExpression":
      return findObjectGetterInSpread(expr.consequent) ?? findObjectGetterInSpread(expr.alternate);
    case "LogicalExpression":
      return findObjectGetterInSpread(expr.left) ?? findObjectGetterInSpread(expr.right);
    case "SequenceExpression": {
      const last = expr.expressions?.at(-1);
      return last ? findObjectGetterInSpread(last) : null;
    }
    case "ObjectExpression":
      return (
        (expr.properties ?? []).find(
          (prop): prop is ObjectPropertyNode => prop.type === "Property" && prop.kind === "get",
        ) ?? null
      );
    default:
      return null;
  }
}

export function unwrapExpr(expr: AstNode | null | undefined): AstNode | null {
  let cur: AstNode | null | undefined = expr;
  while (
    cur &&
    (cur.type === "ParenthesizedExpression" ||
      cur.type === "TSAsExpression" ||
      cur.type === "TSSatisfiesExpression" ||
      cur.type === "TSTypeAssertion" ||
      cur.type === "TSNonNullExpression")
  ) {
    cur = cur.expression;
  }
  return cur ?? null;
}
