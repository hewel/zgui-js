import solidPlugin from "eslint-plugin-solid";
import type { CreateRule, OxlintPlugin, OxlintRule, RuleContext } from "./ast.js";

const DEFAULT_MODULE_SOURCES = ["@hewel/zgui", "#solid"] as const;

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object";
}

function withZguiSolidSettings(context: RuleContext): RuleContext {
  const baseSettings: Record<string, unknown> = isRecord(context.settings) ? context.settings : {};
  const userSolid: Record<string, unknown> = isRecord(baseSettings.solid) ? baseSettings.solid : {};
  const userSources = Array.isArray(userSolid.moduleSources)
    ? userSolid.moduleSources.filter((entry): entry is string => typeof entry === "string")
    : [];
  const moduleSources = [...new Set([...DEFAULT_MODULE_SOURCES, ...userSources])];
  const mergedSettings = {
    ...baseSettings,
    solid: {
      ...userSolid,
      version: userSolid.version ?? 2,
      moduleSources,
    },
  };
  return Object.create(context, {
    settings: { value: mergedSettings, enumerable: true, configurable: true },
  }) as RuleContext;
}

const upstreamRules = solidPlugin.rules as unknown as Record<string, CreateRule>;

const rules: Record<string, OxlintRule> = Object.fromEntries(
  Object.entries(upstreamRules).map(([name, rule]): [string, OxlintRule] => [
    name,
    {
      ...rule,
      create(context: RuleContext) {
        return rule.create(withZguiSolidSettings(context));
      },
    },
  ]),
);

const plugin: OxlintPlugin = {
  meta: { name: "solid" },
  rules,
};

export default plugin;
