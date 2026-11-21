import { createContext } from "#solid";
import type { Accessor } from "./types.js";

/** Reactive `:root` token stylesheet exported by an enclosing `<ThemeProvider>` for auxiliary windows. */
export const ThemeCssContext = createContext<Accessor<string> | null>(null);
