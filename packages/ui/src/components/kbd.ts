import type { NativeElementProps } from "../types.js";
import { part } from "./shared.js";
import { sheet } from "./generated/KbdStyle.js";

export type KbdProps = NativeElementProps;
export type KbdGroupProps = NativeElementProps;

export const Kbd = part("box", sheet, sheet.className);
export const KbdGroup = part("box", sheet, "zui-kbd-group");
