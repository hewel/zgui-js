import { omit } from "#solid";
import { mergeProps } from "../renderer/index.js";
import type { NativeElementProps, NativeNode } from "../types.js";
import { classes, element, part, useStylesheet, withDefaults } from "./shared.js";
import { sheet } from "./generated/EmptyStyle.js";

export type EmptyProps = NativeElementProps;
export type EmptyHeaderProps = NativeElementProps;
export type EmptyTitleProps = NativeElementProps;
export type EmptyDescriptionProps = NativeElementProps;
export type EmptyContentProps = NativeElementProps;
export type EmptyMediaVariant = "default" | "icon";
export interface EmptyMediaProps extends NativeElementProps {
  variant?: EmptyMediaVariant;
}

export const Empty = part("box", sheet, sheet.className);
export const EmptyHeader = part("box", sheet, "zui-empty__header");
export const EmptyTitle = part("label", sheet, "zui-empty__title");
export const EmptyDescription = part("box", sheet, "zui-empty__description");
export const EmptyContent = part("box", sheet, "zui-empty__content");

export function EmptyMedia(props: EmptyMediaProps = {}): NativeNode {
  useStylesheet(sheet);
  const rest = omit(props, "variant");
  return element(
    "box",
    mergeProps(
      rest,
      withDefaults(
        {
          role: "genericContainer" as const,
          ariaHidden: true,
          get "data-variant"() {
            return props.variant ?? "default";
          },
        },
        rest,
      ),
      {
        get __zguiClass() {
          return classes("zui-empty__media", props.variant === "icon" && "zui-empty__media--icon");
        },
      },
    ),
  );
}
