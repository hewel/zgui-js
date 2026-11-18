const UNTHEMED_PRIMITIVES = new Set(["Button", "TextField", "TextArea"]);
const SUPPRESSION_PATTERN = /\b(?:oxlint|eslint)-disable(?:-next-line|-line)?\b/;
const INTERNAL_PACKAGE_SRC = /(?:^|\/)packages\/[^/]+\/src(?:\/|$)/;
const MAX_COMMENT_LINES = 6;

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
