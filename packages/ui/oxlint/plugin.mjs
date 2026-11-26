const UNTHEMED_PRIMITIVES = new Set(["Button", "TextField", "TextArea"]);
const SUPPRESSION_PATTERN = /\b(?:oxlint|eslint)-disable(?:-next-line|-line)?\b/;
const INTERNAL_PACKAGE_SRC = /(?:^|\/)packages\/[^/]+\/src(?:\/|$)/;
const MAX_COMMENT_LINES = 6;
const NATIVE_JSX_TAGS = new Set([
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
const ZGUI_COMPONENT_SOURCES = new Set([
  "@hewel/zgui",
  "@hewel/zgui/components",
  "@hewel/zgui/primitives",
]);
const STATIC_STYLESHEET_HELPERS = new Set(["mountStylesheet", "mountShellStylesheet"]);

const OWNER_CLEANUP_HELPERS = new Set([
  "observeGeometry",
  "trapFocus",
  "lockScroll",
  "registerDismissLayer",
  "mountStylesheet",
  "mountShellStylesheet",
  "createCanvasScene",
  "createDelay",
  "createTimeout",
  "createInterval",
  "createSpring",
  "onCleanup",
  "requestSessionLock",
  "connectUnixSocket",
  "spawnCommand",
  "onControlMessage",
  "loadImageAsset",
  "loadImageBytes",
  "registerImageAsset",
  "registerImageBytes",
  "readTextAsset",
  "useBeforeLeave",
  "bindControlNavigation",
  "onCloseRequest",
  "useWindowState",
  "clipboardReadText",
  "clipboardReadImage",
  "openDatabase",
  "openKvStore",
  "openAuthorizedDirectory",
  "watchDirectory",
  "httpRequest",
  "openEventStream",
  "connectWebSocket",
  "spawnWorker",
]);

const RUNTIME_SERVICE_APIS = new Set([
  "connectUnixSocket",
  "spawnCommand",
  "onControlMessage",
  "loadImageAsset",
  "loadImageBytes",
  "registerImageAsset",
  "registerImageBytes",
  "readTextAsset",
  "getResolvedCapabilities",
  "readClipboardText",
  "clipboardReadText",
  "writeClipboardText",
  "readClipboardImage",
  "clipboardReadImage",
  "writeClipboardImage",
  "openDatabase",
  "openKvStore",
  "openFileDialog",
  "saveFileDialog",
  "openDirectoryDialog",
  "openAuthorizedDirectory",
  "persistHandle",
  "restoreBookmark",
  "listBookmarks",
  "revokeBookmark",
  "watchDirectory",
  "httpRequest",
  "openEventStream",
  "connectWebSocket",
  "spawnWorker",
]);

function commentLines(comment) {
  return comment.loc.end.line - comment.loc.start.line + 1;
}

function getAllComments(context) {
  return context.sourceCode?.getAllComments?.() ?? [];
}

function importedName(specifier) {
  if (specifier.type !== "ImportSpecifier") return null;
  if (specifier.imported.type === "Identifier") return specifier.imported.name;
  return String(specifier.imported.value ?? "");
}

function isFunctionNode(node) {
  return (
    node.type === "FunctionDeclaration" ||
    node.type === "FunctionExpression" ||
    node.type === "ArrowFunctionExpression"
  );
}

function isIife(fnNode) {
  let current = fnNode.parent;
  while (current && current.type === "ParenthesizedExpression") current = current.parent;
  return current?.type === "CallExpression" && current.callee === fnNode;
}

function enclosingRefCallback(node) {
  let fn = null;
  for (let cur = node.parent; cur; cur = cur.parent) {
    if (isFunctionNode(cur)) {
      if (isIife(cur)) continue;
      fn = cur;
      break;
    }
  }
  if (!fn) return null;
  for (let cur = fn.parent; cur; cur = cur.parent) {
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

function isModuleTopLevel(node) {
  for (let cur = node.parent; cur; cur = cur.parent) {
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

function calleeHelperName(callee) {
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

function findObjectGetterInSpread(expr) {
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
        (expr.properties ?? []).find(prop => prop.type === "Property" && prop.kind === "get") ??
        null
      );
    default:
      return null;
  }
}

function unwrapExpr(expr) {
  let cur = expr;
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
  return cur;
}

function createScope(parent, isFunctionScope) {
  return { parent, isFunctionScope, bindings: new Map() };
}

function resolveBinding(scope, name) {
  for (let cur = scope; cur; cur = cur.parent) {
    const binding = cur.bindings.get(name);
    if (binding) return binding;
  }
  return null;
}

function collectPatternNames(pattern, out = []) {
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

function bindPatternNames(pattern, scope) {
  for (const name of collectPatternNames(pattern)) {
    scope.bindings.set(name, { kind: "other", scope, mutated: false });
  }
}

function registerVariableDecl(decl, scope, varScope) {
  if (decl.declare) return;
  const target = decl.kind === "var" ? varScope : scope;
  for (const item of decl.declarations ?? []) {
    if (decl.kind === "const" && item.id?.type === "Identifier") {
      target.bindings.set(item.id.name, {
        kind: "const",
        init: item.init ?? null,
        scope: target,
        mutated: false,
      });
    } else {
      bindPatternNames(item.id, target);
    }
  }
}

function hoistStatementDecl(stmt, scope, varScope) {
  if (!stmt) return;
  if (stmt.type === "ImportDeclaration") {
    if (stmt.importKind === "type" || typeof stmt.source?.value !== "string") return;
    const source = stmt.source.value;
    for (const spec of stmt.specifiers ?? []) {
      if (spec.importKind === "type") continue;
      if (spec.type === "ImportSpecifier") {
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

function rootMutatedIdentifier(expr) {
  const inner = unwrapExpr(expr);
  if (!inner) return null;
  if (inner.type === "Identifier") return inner.name;
  if (inner.type === "MemberExpression") return rootMutatedIdentifier(inner.object);
  return null;
}

function markMutated(expr, scope) {
  const name = rootMutatedIdentifier(expr);
  if (!name) return;
  const binding = resolveBinding(scope, name);
  if (binding) binding.mutated = true;
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

function buildScopeIndex(program) {
  const nodeScope = new WeakMap();
  const rootScope = createScope(null, true);

  function walkChildren(node, scope, varScope) {
    for (const key of Object.keys(node)) {
      if (key === "parent" || key === "loc" || key === "range") continue;
      if (key === "typeAnnotation" || key === "typeParameters" || key === "returnType") continue;
      const value = node[key];
      if (!value || typeof value !== "object") continue;
      if (Array.isArray(value)) {
        for (const item of value) {
          if (item && typeof item.type === "string") walk(item, scope, varScope);
        }
      } else if (typeof value.type === "string") {
        walk(value, scope, varScope);
      }
    }
  }

  function walkFunction(node, scope) {
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

  function walk(node, scope, varScope) {
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
        const head = node.init ?? node.left;
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

function resolveConstExpr(expr, scope, visited = new Set()) {
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

function isNullishLiteral(expr, scope) {
  if (!expr) return false;
  if (expr.type === "Literal") return expr.value === null;
  return (
    expr.type === "Identifier" && expr.name === "undefined" && !resolveBinding(scope, "undefined")
  );
}

function extractStaticString(expr, scope, visited = new Set()) {
  const { expr: node } = resolveConstExpr(expr, scope, visited);
  if (!node) return null;
  if (node.type === "Literal" && typeof node.value === "string") return node.value;
  if (node.type === "TemplateLiteral" && node.expressions.length === 0) {
    return node.quasis[0]?.value?.cooked ?? node.quasis[0]?.value?.raw ?? "";
  }
  return null;
}

function extractStaticNumber(expr, scope, visited = new Set()) {
  const { expr: node } = resolveConstExpr(expr, scope, visited);
  if (!node) return null;
  if (node.type === "Literal" && typeof node.value === "number") return node.value;
  if (node.type === "UnaryExpression" && node.operator === "-") {
    const arg = unwrapExpr(node.argument);
    if (arg?.type === "Literal" && typeof arg.value === "number") return -arg.value;
  }
  return null;
}

function extractStaticPrimitive(expr, scope, visited = new Set()) {
  const str = extractStaticString(expr, scope, visited);
  if (str !== null) return str;
  return extractStaticNumber(expr, scope, visited);
}

function isFiniteStaticConditional(expr, scope, visited = new Set()) {
  const { expr: inner, scope: innerScope } = resolveConstExpr(expr, scope, visited);
  if (!inner || inner.type !== "ConditionalExpression") return false;
  const checkBranch = (branch, branchScope) => {
    const { expr: b, scope: bScope } = resolveConstExpr(branch, branchScope, visited);
    if (!b) return false;
    if (b.type === "ConditionalExpression") {
      return checkBranch(b.consequent, bScope) && checkBranch(b.alternate, bScope);
    }
    const str = extractStaticString(b, bScope, visited);
    if (str !== null) return str.trim().length > 0;
    return extractStaticNumber(b, bScope, visited) !== null;
  };
  return checkBranch(inner.consequent, innerScope) && checkBranch(inner.alternate, innerScope);
}

function normalizeStyleKey(rawKey) {
  if (rawKey.startsWith("--")) return rawKey;
  return rawKey.replace(/[A-Z]/g, ch => `-${ch.toLowerCase()}`);
}

function analyzeStyleObject(objNode, objScope, seenObjects = new Set()) {
  const entries = new Map();
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

    let rawKey = null;
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

function collectTopLevelConditionalObjects(expr, scope) {
  const leaves = [];
  const visit = (curExpr, curScope) => {
    const { expr: resolved, scope: resolvedScope } = resolveConstExpr(curExpr, curScope);
    if (!resolved) return false;
    if (resolved.type === "ConditionalExpression") {
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
    l => l.kind === "object" && [...l.info.entries.values()].some(e => e.kind === "static"),
  );
  if (nonEmptyObjects.length === 0) return null;
  return nonEmptyObjects.flatMap(o =>
    [...o.info.entries.values()].filter(e => e.kind === "static").map(e => e.key),
  );
}

function isReportableStaticKey(normalizedKey) {
  return !normalizedKey.startsWith("--") || normalizedKey.startsWith("--zgui-");
}

function describeTargetJsxElement(openingEl, scope) {
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
    const parts = [];
    let cur = name;
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

function classifyStyleAttribute(attrValue, scope) {
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

const rules = {
  "no-direct-bridge": {
    meta: {
      type: "problem",
      docs: {
        description:
          "Disallow direct globalThis.__zgui bridge access outside host/runtime internals",
      },
      schema: [],
    },
    create(context) {
      const message =
        "Do not access globalThis.__zgui directly. Import typed SDK helpers from @hewel/zgui (mountStylesheet, createPortal, focus, registerDismissLayer, lockScroll, trapFocus) or packages/ui/src/components/shared.ts.";
      return {
        MemberExpression(node) {
          const prop = node.property;
          const isBridgeProp =
            (!node.computed && prop?.type === "Identifier" && prop.name === "__zgui") ||
            (node.computed && prop?.type === "Literal" && prop.value === "__zgui");
          if (isBridgeProp) context.report({ node, message });
        },
        Identifier(node) {
          if (node.name !== "__zgui") return;
          const parent = node.parent;
          if (parent?.type === "MemberExpression" && !parent.computed && parent.property === node)
            return;
          context.report({ node, message });
        },
      };
    },
  },

  "no-raw-insert-or-provider": {
    meta: {
      type: "problem",
      docs: {
        description:
          "Disallow raw Solid universal insert() and direct Context provider invocations",
      },
      schema: [],
    },
    create(context) {
      const providerMessage =
        "Do not invoke Context providers directly; use provide() from shared.ts so children stay wrapped in a getter under the provider owner.";
      const isContextName = name =>
        /^[A-Z]/.test(name) && (/Context$/.test(name) || name === "Group" || name === "Internal");
      return {
        CallExpression(node) {
          if (node.callee.type === "Identifier" && node.callee.name === "insert") {
            context.report({
              node,
              message:
                "Do not call raw Solid universal insert(...) directly; use append() or slot() from shared.ts with an explicit marker so independent siblings stay isolated.",
            });
            return;
          }
          if (
            (node.callee.type === "Identifier" && isContextName(node.callee.name)) ||
            (node.callee.type === "MemberExpression" &&
              !node.callee.computed &&
              node.callee.property.type === "Identifier" &&
              node.callee.property.name === "Provider")
          ) {
            context.report({ node, message: providerMessage });
          }
        },
        JSXOpeningElement(node) {
          const isProviderJsx =
            (node.name.type === "JSXIdentifier" && isContextName(node.name.name)) ||
            (node.name.type === "JSXMemberExpression" &&
              node.name.property.type === "JSXIdentifier" &&
              node.name.property.name === "Provider");
          if (
            isProviderJsx &&
            node.attributes.some(
              attr =>
                attr.type === "JSXAttribute" &&
                attr.name?.type === "JSXIdentifier" &&
                attr.name.name === "children",
            )
          ) {
            context.report({
              node,
              message:
                "Do not pass an explicit children prop to a Context provider in JSX; place JSX children inside the element or use provide().",
            });
          }
        },
      };
    },
  },

  "no-unthemed-primitives-with-components": {
    meta: {
      type: "problem",
      docs: {
        description:
          "Disallow importing unthemed Button/TextField/TextArea alongside @hewel/zgui/components",
      },
      schema: [],
    },
    create(context) {
      return {
        Program(program) {
          const filename = (context.filename ?? "").replace(/\\/g, "/");
          const inComponentsDir =
            filename.includes("/packages/ui/src/components/") ||
            filename.startsWith("packages/ui/src/components/");
          const imports = program.body.filter(stmt => stmt.type === "ImportDeclaration");
          const usesComponents = imports.some(
            decl => decl.source.value === "@hewel/zgui/components",
          );
          if (!usesComponents && !inComponentsDir) return;
          for (const decl of imports) {
            if (decl.importKind === "type") continue;
            const source = decl.source.value;
            const fromRawRoot =
              source === "@hewel/zgui" ||
              (inComponentsDir &&
                /^(\.\.\/index|\.\.\/button|\.\.\/input)(\.js|\.ts)?$/.test(source));
            if (!fromRawRoot) continue;
            for (const spec of decl.specifiers) {
              if (spec.importKind === "type") continue;
              const name = importedName(spec);
              if (name && UNTHEMED_PRIMITIVES.has(name)) {
                context.report({
                  node: spec,
                  message: `Import themed ${name} from @hewel/zgui/components instead of unthemed @hewel/zgui when using components.`,
                });
              }
            }
          }
        },
      };
    },
  },

  "no-internal-package-imports": {
    meta: {
      type: "problem",
      docs: { description: "Disallow reaching into packages/*/src from apps or public code" },
      schema: [],
    },
    create(context) {
      const checkSource = node => {
        const source = node.source;
        if (!source || typeof source.value !== "string") return;
        if (INTERNAL_PACKAGE_SRC.test(source.value)) {
          context.report({
            node: source,
            message:
              "Do not import internal files from packages/*/src; import from public package entry points (@hewel/zgui, @hewel/zgui/components, @hewel/zgui-services).",
          });
        }
      };
      return {
        ImportDeclaration: checkSource,
        ExportNamedDeclaration: checkSource,
        ExportAllDeclaration: checkSource,
      };
    },
  },

  "no-cleanup-in-ref": {
    meta: {
      type: "problem",
      docs: {
        description: "Disallow calling owner-scoped cleanup helpers inside JSX ref callbacks",
      },
      schema: [],
    },
    create(context) {
      return {
        CallExpression(node) {
          const name = calleeHelperName(node.callee);
          if (!name || !OWNER_CLEANUP_HELPERS.has(name)) return;
          if (!enclosingRefCallback(node)) return;
          context.report({
            node,
            message: `Do not call owner-scoped helper ${name}() inside a JSX ref callback; Solid 2.0.1 dev mode throws [CLEANUP_IN_FORBIDDEN_SCOPE]. Store the node in a signal (const [node, setNode] = createSignal<NativeNode | null>(null)), pass ref={setNode}, and call ${name}(node, ...) or wire it inside createEffect in the component body.`,
          });
        },
      };
    },
  },

  "no-top-level-service-call": {
    meta: {
      type: "problem",
      docs: {
        description:
          "Disallow calling @hewel/zgui-services runtime APIs at module top level before mount()",
      },
      schema: [],
    },
    create(context) {
      const localApis = new Map();
      const namespaces = new Set();
      return {
        ImportDeclaration(decl) {
          if (decl.importKind === "type" || decl.source?.value !== "@hewel/zgui-services") return;
          for (const spec of decl.specifiers) {
            if (spec.importKind === "type") continue;
            if (spec.type === "ImportSpecifier") {
              const name = importedName(spec);
              if (name && RUNTIME_SERVICE_APIS.has(name)) {
                localApis.set(spec.local.name, name);
              }
            } else if (spec.type === "ImportNamespaceSpecifier") {
              namespaces.add(spec.local.name);
            }
          }
        },
        CallExpression(node) {
          if (!isModuleTopLevel(node)) return;
          let api = null;
          if (node.callee.type === "Identifier") {
            api = localApis.get(node.callee.name) ?? null;
          } else if (
            node.callee.type === "MemberExpression" &&
            !node.callee.computed &&
            node.callee.object.type === "Identifier" &&
            namespaces.has(node.callee.object.name) &&
            node.callee.property.type === "Identifier" &&
            RUNTIME_SERVICE_APIS.has(node.callee.property.name)
          ) {
            api = node.callee.property.name;
          }
          if (!api) return;
          context.report({
            node,
            message: `Do not call @hewel/zgui-services runtime API ${api}() at module top level before mount(); preflight evaluation forbids host service side effects. Call ${api}() inside mount() or a component/effect body.`,
          });
        },
      };
    },
  },

  "no-solid-web-imports": {
    meta: {
      type: "problem",
      docs: {
        description: "Disallow importing solid-js or solid-js/web directly instead of @hewel/zgui",
      },
      schema: [],
    },
    create(context) {
      const checkSource = node => {
        const source = node.source;
        if (!source || typeof source.value !== "string") return;
        if (source.value === "solid-js" || source.value.startsWith("solid-js/")) {
          context.report({
            node: source,
            message: `Do not import from "${source.value}" in zgui-js apps; import reactive primitives (createSignal, createMemo, createEffect, onCleanup, Show, For, createStore, etc.) from "@hewel/zgui" so the native Solid 2.0.1 runtime stays unified.`,
          });
        }
      };
      return {
        ImportDeclaration: checkSource,
        ExportNamedDeclaration: checkSource,
        ExportAllDeclaration: checkSource,
      };
    },
  },

  "no-lint-suppression": {
    meta: {
      type: "problem",
      docs: { description: "Disallow inline oxlint/eslint disable directives" },
      schema: [],
    },
    create(context) {
      return {
        Program() {
          for (const comment of getAllComments(context)) {
            if (SUPPRESSION_PATTERN.test(comment.value)) {
              context.report({
                loc: comment.loc,
                message:
                  "Do not suppress lint rules with inline disable comments; fix the architecture violation or scope exclusions explicitly in .oxlintrc.json.",
              });
            }
          }
        },
      };
    },
  },

  "no-essay-comments": {
    meta: {
      type: "suggestion",
      docs: { description: "Disallow multi-paragraph comments exceeding 6 lines" },
      schema: [],
    },
    create(context) {
      const message =
        "Keep comments concise (<= 6 lines); document only non-obvious invariants or public API contracts instead of multi-line prose.";
      return {
        Program() {
          const comments = getAllComments(context);
          let runStart = null;
          let runEnd = null;
          let runCount = 0;
          const flushRun = () => {
            if (runStart && runEnd && runCount > MAX_COMMENT_LINES) {
              context.report({
                loc: { start: runStart.loc.start, end: runEnd.loc.end },
                message,
              });
            }
            runStart = null;
            runEnd = null;
            runCount = 0;
          };
          for (const comment of comments) {
            if (comment.type === "Block") {
              flushRun();
              if (commentLines(comment) > MAX_COMMENT_LINES) {
                context.report({ loc: comment.loc, message });
              }
              continue;
            }
            if (comment.type === "Line") {
              if (runEnd && comment.loc.start.line === runEnd.loc.end.line + 1) {
                runEnd = comment;
                runCount += 1;
              } else {
                flushRun();
                runStart = comment;
                runEnd = comment;
                runCount = 1;
              }
            }
          }
          flushRun();
        },
      };
    },
  },

  "no-getter-object-spread": {
    meta: {
      type: "problem",
      docs: {
        description:
          "Disallow spreading object literals containing getters, which eagerly evaluates getters and breaks Solid reactivity",
      },
      schema: [],
    },
    create(context) {
      return {
        ObjectExpression(node) {
          for (const prop of node.properties ?? []) {
            if (prop.type !== "SpreadElement") continue;
            const getter = findObjectGetterInSpread(prop.argument);
            if (!getter) continue;
            const keyName =
              getter.key?.type === "Identifier"
                ? getter.key.name
                : getter.key?.type === "Literal"
                  ? String(getter.key.value)
                  : "getter";
            context.report({
              node: prop,
              message: `Do not spread an object literal containing \`get ${keyName}()\`; object spread eagerly evaluates getters once (triggering Solid [STRICT_READ_UNTRACKED] and losing reactivity). Use mergeProps(...) or Object.defineProperty(...) to preserve the getter descriptor.`,
            });
          }
        },
      };
    },
  },

  "no-static-component-stylesheet": {
    meta: {
      type: "problem",
      docs: {
        description:
          "Disallow installing static stylesheets with mountStylesheet()/mountShellStylesheet() inside components or functions",
      },
      schema: [],
    },
    create(context) {
      let scopeIndex = null;
      return {
        Program(program) {
          scopeIndex = buildScopeIndex(program);
        },
        CallExpression(node) {
          if (!scopeIndex || isModuleTopLevel(node)) return;
          if (node.arguments.length < 2 || node.arguments[1].type === "SpreadElement") return;
          const scope = scopeIndex.getScope(node);
          const callee = unwrapExpr(node.callee);
          let helperName = null;
          if (callee?.type === "Identifier") {
            const binding = resolveBinding(scope, callee.name);
            if (
              binding?.kind === "import" &&
              binding.source === "@hewel/zgui" &&
              STATIC_STYLESHEET_HELPERS.has(binding.imported)
            ) {
              helperName = binding.imported;
            }
          } else if (
            callee?.type === "MemberExpression" &&
            !callee.computed &&
            callee.object.type === "Identifier" &&
            callee.property.type === "Identifier" &&
            STATIC_STYLESHEET_HELPERS.has(callee.property.name)
          ) {
            const binding = resolveBinding(scope, callee.object.name);
            if (binding?.kind === "namespace" && binding.source === "@hewel/zgui") {
              helperName = callee.property.name;
            }
          }
          if (!helperName) return;
          if (extractStaticString(node.arguments[1], scope) === null) return;
          context.report({
            node,
            message: `Do not call ${helperName}() with a static stylesheet inside a component or function; move static rules to *.module.css or src/app.css (runtime static stylesheet installation disables StyleDependencies and rebuilds Stylist indexes). Keep ${helperName}() at module top level or pass dynamic runtime CSS only.`,
          });
        },
      };
    },
  },

  "prefer-stylesheet": {
    meta: {
      type: "suggestion",
      docs: {
        description:
          "Prefer *.module.css or src/app.css over static inline style attributes and finite conditional style values",
      },
      schema: [],
    },
    create(context) {
      let scopeIndex = null;
      return {
        Program(program) {
          scopeIndex = buildScopeIndex(program);
        },
        JSXOpeningElement(node) {
          if (!scopeIndex) return;
          const scope = scopeIndex.getScope(node);
          const tagName = describeTargetJsxElement(node, scope);
          if (!tagName) return;
          for (const attr of node.attributes ?? []) {
            if (
              attr.type !== "JSXAttribute" ||
              attr.name?.type !== "JSXIdentifier" ||
              attr.name.name !== "style"
            ) {
              continue;
            }
            const finding = classifyStyleAttribute(attr.value, scope);
            if (!finding) continue;
            let message = "";
            if (finding.kind === "static-string") {
              message = `Move static inline style string on <${tagName}> to *.module.css or src/app.css (e.g. class={styles.name}). Static inline styles bypass SharedStyles and CascadeMemo sharing.`;
            } else if (finding.kind === "static-object") {
              const keys = finding.keys.join(", ");
              message = `Move static inline style (${keys}) on <${tagName}> to *.module.css or src/app.css (e.g. class={styles.name}). Static inline styles bypass SharedStyles and CascadeMemo sharing.`;
            } else if (finding.kind === "mixed-object") {
              const keys = finding.staticKeys.join(", ");
              message = `Move static style properties (${keys}) on <${tagName}> to *.module.css or src/app.css and keep only dynamic runtime properties inline.`;
            } else if (finding.kind === "conditional-static") {
              const keys = finding.condKeys.join(", ");
              message = `Move finite conditional style properties (${keys}) on <${tagName}> to *.module.css using class={cx(...)} or [data-*] selectors instead of inline ternary values.`;
            } else {
              const sKeys = finding.staticKeys.join(", ");
              const cKeys = finding.condKeys.join(", ");
              message = `Move static properties (${sKeys}) and finite conditional properties (${cKeys}) on <${tagName}> to *.module.css (using class={cx(...)} or [data-*]) or src/app.css, keeping only dynamic runtime properties inline.`;
            }
            context.report({ node: attr, message });
          }
        },
      };
    },
  },
};

export default {
  meta: { name: "zgui" },
  rules,
};
