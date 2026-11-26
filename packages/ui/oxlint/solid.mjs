import solidPlugin from "eslint-plugin-solid";

const DEFAULT_MODULE_SOURCES = ["@hewel/zgui", "#solid"];

function withZguiSolidSettings(context) {
  const baseSettings =
    context.settings && typeof context.settings === "object" ? context.settings : {};
  const userSolid =
    baseSettings.solid && typeof baseSettings.solid === "object" ? baseSettings.solid : {};
  const userSources = Array.isArray(userSolid.moduleSources) ? userSolid.moduleSources : [];
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
  });
}

const rules = Object.fromEntries(
  Object.entries(solidPlugin.rules ?? {}).map(([name, rule]) => [
    name,
    {
      ...rule,
      create(context) {
        return rule.create(withZguiSolidSettings(context));
      },
    },
  ]),
);

export default {
  meta: { name: "solid" },
  rules,
};
