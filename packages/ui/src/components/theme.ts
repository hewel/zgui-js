import { createContext, createUniqueId, onCleanup, useContext } from "#solid";
import { createElement, effect, insertNode, setProp, spread } from "../renderer/index.js";
import { getReducedMotion, installStylesheet, removeStylesheet } from "../nodes.js";
import type { Accessor, NativeChild, NativeNode } from "../types.js";
import { provide, slot } from "./shared.js";
import { themes, themeTokens } from "./generated/themes.js";

export type ColorScheme = "system" | "light" | "dark";
export type ThemePreset = keyof typeof themes;
export type ThemeToken = typeof themeTokens[number];
export interface Theme {
  readonly preset?: ThemePreset;
  readonly tokens?: Partial<Record<ThemeToken, string>>;
}
export interface ThemeProviderProps {
  scheme?: ColorScheme;
  light?: Theme;
  dark?: Theme;
  children?: NativeChild;
}
export interface ThemeContext {
  scheme: Accessor<ColorScheme>;
  light: Accessor<Theme>;
  dark: Accessor<Theme>;
}

const ThemeSheet = "zgui-ui-theme";
const tokenSet = new Set<string>(themeTokens);
const Internal = createContext<ThemeContext | null>(null);

export function useTheme(): ThemeContext | undefined {
  return useContext(Internal) ?? undefined;
}

function presetDeclarations(theme: Theme, scheme: "light" | "dark"): string {
  const preset = theme.preset ?? "base";
  if (!Object.hasOwn(themes, preset)) throw new TypeError(`Unknown theme preset: ${preset}`);
  return themes[preset][scheme];
}

function overrideDeclarations(theme: Theme): string {
  if (theme.tokens === undefined) return "";
  let out = "";
  for (const [name, value] of Object.entries(theme.tokens)) {
    if (!tokenSet.has(name)) throw new TypeError(`Unknown theme token: ${name}`);
    if (typeof value !== "string" || value === "") {
      throw new TypeError(`Theme token ${name} must be a nonempty string`);
    }
    if (value.includes(";") || value.includes("{") || value.includes("}") || value.includes("/*")) {
      throw new TypeError(`Theme token ${name} contains unsafe characters`);
    }
    out += `${name}: ${value}; `;
  }
  return out;
}

function rule(selector: string, scheme: "light" | "dark", theme: Theme): string {
  const motion = getReducedMotion()
    ? themeTokens.filter(name => name.startsWith("--zui-motion-duration-")).map(name => `${name}: 0ms;`).join(" ")
    : "";
  const declarations = `color-scheme: ${scheme}; ${presetDeclarations(theme, scheme)} ${overrideDeclarations(theme)} ${motion}`;
  return `${selector} { ${declarations} }`;
}

function themeSheet(selector: string, light: Theme, dark: Theme, scheme: ColorScheme): string {
  if (scheme !== "system" && scheme !== "light" && scheme !== "dark") {
    throw new TypeError(`Unknown color scheme: ${scheme}`);
  }
  let css = "";
  if (scheme === "light" || scheme === "system") css += rule(selector, "light", light);
  if (scheme === "dark" || scheme === "system") {
    const darkRule = rule(selector, "dark", dark);
    css += (css === "" ? "" : "\n")
      + (scheme === "system" ? `@media (prefers-color-scheme: dark) { ${darkRule} }` : darkRule);
  }
  return css;
}

export function portalThemeScope(theme: ThemeContext, child: NativeNode): NativeNode {
  const id = `zui-theme-${createUniqueId().replace(/[^a-zA-Z0-9_-]/g, "-")}`;
  const sheetName = `${ThemeSheet}-${id}`;
  const wrapper: NativeNode = createElement("box");
  setProp(wrapper, "__zguiClass", id);
  setProp(wrapper, "__zguiInvariantStyle", { display: "contents" });
  effect(
    () => [theme.light(), theme.dark(), theme.scheme()] as const,
    ([light, dark, scheme]) => {
      installStylesheet(sheetName, `@layer zgui.tokens { ${themeSheet(`.${id}`, light, dark, scheme)} }`);
    },
  );
  onCleanup(() => removeStylesheet(sheetName));
  insertNode(wrapper, child);
  return wrapper;
}

export function ThemeProvider(props: ThemeProviderProps = {}): NativeChild {
  const parent = useContext(Internal);
  const id = `zui-theme-${createUniqueId().replace(/[^a-zA-Z0-9_-]/g, "-")}`;
  const selector = parent === null ? ":root" : `.${id}`;
  const sheetName = `${ThemeSheet}-${id}`;
  const context: ThemeContext = {
    scheme: () => props.scheme ?? "system",
    light: () => props.light ?? {},
    dark: () => props.dark ?? {},
  };
  const wrapper: NativeNode = createElement("box");
  spread(wrapper, { __zguiClass: id, __zguiInvariantStyle: { display: "contents" } }, true);
  effect(
    () => [context.light(), context.dark(), context.scheme()] as const,
    ([light, dark, scheme]) => {
      installStylesheet(sheetName, `@layer zgui.tokens { ${themeSheet(selector, light, dark, scheme)} }`);
    },
  );
  onCleanup(() => removeStylesheet(sheetName));
  slot(wrapper, provide(Internal, context, props));
  return wrapper;
}
