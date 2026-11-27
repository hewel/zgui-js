import {
  calleeHelperName,
  commentLines,
  enclosingRefCallback,
  findObjectGetterInSpread,
  getAllComments,
  importedName,
  isModuleTopLevel,
  parentOf,
  unwrapExpr,
  type AstComment,
  type AstNode,
  type OxlintPlugin,
  type OxlintRule,
} from "./ast.js";
import { buildScopeIndex, resolveBinding, type ScopeIndex } from "./scope.js";
import {
  classifyStyleAttribute,
  describeTargetJsxElement,
  extractStaticString,
} from "./style-analysis.js";

const UNTHEMED_PRIMITIVES = new Set(["Button", "TextField", "TextArea"]);
const SUPPRESSION_PATTERN = /\b(?:oxlint|eslint)-disable(?:-next-line|-line)?\b/;
const INTERNAL_PACKAGE_SRC = /(?:^|\/)packages\/[^/]+\/src(?:\/|$)/;
const MAX_COMMENT_LINES = 6;
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
  "onActivation",
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
  "getInitialActivation",
  "onActivation",
  "loadImageAsset",
  "loadImageBytes",
  "registerImageAsset",
  "registerImageBytes",
  "readTextAsset",
  "getResolvedCapabilities",
  "getSystemTimeZone",
  "getSystemLocale",
  "isValidTimeZone",
  "toZonedDateTime",
  "fromZonedDateTime",
  "addZonedDuration",
  "formatDateTime",
  "getCalendarLabels",
  "compareStrings",
  "collatorSortIndices",
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

function isForbiddenSolidSource(value: string): boolean {
  return (
    value === "solid-js" ||
    value.startsWith("solid-js/") ||
    value === "@solidjs/web" ||
    value.startsWith("@solidjs/web/") ||
    value === "@solidjs/signals" ||
    value.startsWith("@solidjs/signals/") ||
    value === "@solidjs/universal" ||
    value.startsWith("@solidjs/universal/")
  );
}

function isContextName(name: string): boolean {
  return /^[A-Z]/.test(name) && (/Context$/.test(name) || name === "Group" || name === "Internal");
}

const rules: Record<string, OxlintRule> = {
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
          const parent = parentOf(node);
          if (parent?.type === "MemberExpression" && !parent.computed && parent.property === node) {
            return;
          }
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
              if (spec.type !== "ImportSpecifier" || spec.importKind === "type") continue;
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
      const checkSource = (node: { source?: AstNode | null }) => {
        const source = node.source;
        if (!source || source.type !== "Literal" || typeof source.value !== "string") return;
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
      const localApis = new Map<string, string>();
      const namespaces = new Set<string>();
      return {
        ImportDeclaration(decl) {
          if (decl.importKind === "type" || decl.source?.value !== "@hewel/zgui-services") return;
          for (const spec of decl.specifiers) {
            if (spec.type === "ImportSpecifier") {
              if (spec.importKind === "type") continue;
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
          let api: string | null = null;
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
        description:
          "Disallow importing solid-js or @solidjs/* runtime packages directly instead of @hewel/zgui",
      },
      schema: [],
    },
    create(context) {
      const checkSource = (node: { source?: AstNode | null }) => {
        const source = node.source;
        if (!source || source.type !== "Literal" || typeof source.value !== "string") return;
        if (isForbiddenSolidSource(source.value)) {
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
          let runStart: AstComment | null = null;
          let runEnd: AstComment | null = null;
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
      let scopeIndex: ScopeIndex | null = null;
      return {
        Program(program) {
          scopeIndex = buildScopeIndex(program);
        },
        CallExpression(node) {
          if (!scopeIndex || isModuleTopLevel(node)) return;
          const secondArg = node.arguments[1];
          if (!secondArg || secondArg.type === "SpreadElement") return;
          const scope = scopeIndex.getScope(node);
          const callee = unwrapExpr(node.callee);
          let helperName: string | null = null;
          if (callee?.type === "Identifier") {
            const binding = resolveBinding(scope, callee.name);
            if (
              binding?.kind === "import" &&
              binding.source === "@hewel/zgui" &&
              binding.imported &&
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
          if (extractStaticString(secondArg, scope) === null) return;
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
      let scopeIndex: ScopeIndex | null = null;
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

const plugin: OxlintPlugin = {
  meta: { name: "zgui" },
  rules,
};

export default plugin;
