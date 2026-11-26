import { createContext } from "#solid";
import type { Accessor } from "./types.js";

/** Reactive `:root` tokens for managed child regions owned under a `<ThemeProvider>`. */
export const ThemeCssContext = createContext<Accessor<string> | null>(null);
