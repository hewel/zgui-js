const UNTHEMED_PRIMITIVES = new Set(["Button", "TextField", "TextArea"]);
const SUPPRESSION_PATTERN = /\b(?:oxlint|eslint)-disable(?:-next-line|-line)?\b/;
const INTERNAL_PACKAGE_SRC = /(?:^|\/)packages\/[^/]+\/src(?:\/|$)/;
const MAX_COMMENT_LINES = 6;

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
  "registerImageBytes",
  "readTextAsset",
]);

const RUNTIME_SERVICE_APIS = new Set([
  "connectUnixSocket",
  "spawnCommand",
  "onControlMessage",
  "loadImageAsset",
  "loadImageBytes",
  "registerImageBytes",
  "readTextAsset",
  "getResolvedCapabilities",
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
    if (cur.type === "MethodDefinition" || cur.type === "PropertyDefinition" || cur.type === "StaticBlock") {
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

const rules = {
  "no-direct-bridge": {
    meta: {
      type: "problem",
      docs: { description: "Disallow direct globalThis.__zgui bridge access outside host/runtime internals" },
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
          if (parent?.type === "MemberExpression" && !parent.computed && parent.property === node) return;
          context.report({ node, message });
        },
      };
    },
  },

  "no-raw-insert-or-provider": {
    meta: {
      type: "problem",
      docs: { description: "Disallow raw Solid universal insert() and direct Context provider invocations" },
      schema: [],
    },
    create(context) {
      const providerMessage =
        "Do not invoke Context providers directly; use provide() from shared.ts so children stay wrapped in a getter under the provider owner.";
      const isContextName = name => /^[A-Z]/.test(name) && (/Context$/.test(name) || name === "Group" || name === "Internal");
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
              attr => attr.type === "JSXAttribute" && attr.name?.type === "JSXIdentifier" && attr.name.name === "children",
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
      docs: { description: "Disallow importing unthemed Button/TextField/TextArea alongside @hewel/zgui/components" },
      schema: [],
    },
    create(context) {
      return {
        Program(program) {
          const filename = (context.filename ?? "").replace(/\\/g, "/");
          const inComponentsDir =
            filename.includes("/packages/ui/src/components/") || filename.startsWith("packages/ui/src/components/");
          const imports = program.body.filter(stmt => stmt.type === "ImportDeclaration");
          const usesComponents = imports.some(decl => decl.source.value === "@hewel/zgui/components");
          if (!usesComponents && !inComponentsDir) return;
          for (const decl of imports) {
            if (decl.importKind === "type") continue;
            const source = decl.source.value;
            const fromRawRoot =
              source === "@hewel/zgui" ||
              (inComponentsDir && /^(\.\.\/index|\.\.\/button|\.\.\/input)(\.js|\.ts)?$/.test(source));
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
      docs: { description: "Disallow calling owner-scoped cleanup helpers inside JSX ref callbacks" },
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
      docs: { description: "Disallow calling @hewel/zgui-services runtime APIs at module top level before mount()" },
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
      docs: { description: "Disallow importing solid-js or solid-js/web directly instead of @hewel/zgui" },
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
};

export default {
  meta: { name: "zgui" },
  rules,
};
