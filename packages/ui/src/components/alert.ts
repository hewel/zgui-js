import { omit } from "#solid";
import { memo, mergeProps } from "../renderer/index.js";
import type { NativeElementProps, NativeNode } from "../types.js";
import { classes, element, part, useStylesheet, withDefaults } from "./shared.js";
import { Icon } from "./icons.js";
import { ALERT_TRIANGLE, INFO } from "./generated/icons.js";
import { sheet } from "./generated/AlertStyle.js";

export type AlertVariant = "default" | "destructive";

export interface AlertProps extends NativeElementProps {
  variant?: AlertVariant;
  live?: boolean;
  icon?: boolean;
}

export type AlertTitleProps = NativeElementProps;
export type AlertDescriptionProps = NativeElementProps;

export const AlertTitle = part("label", sheet, "zui-alert__title");
export const AlertDescription = part("box", sheet, "zui-alert__description");

export function Alert(props: AlertProps = {}): NativeNode {
  useStylesheet(sheet);
  const rest = omit(props, "variant", "live", "icon", "children");
  const mark = memo(() =>
    (props.icon ?? true)
      ? Icon({
          get icon() {
            return (props.variant ?? "default") === "destructive" ? ALERT_TRIANGLE : INFO;
          },
          class: "zui-alert__icon",
        })
      : null,
  );
  const content = memo(() => props.children);
  return element(
    "box",
    mergeProps(
      rest,
      withDefaults(
        {
          get role() {
            return (props.live ?? true) ? ("alert" as const) : ("group" as const);
          },
          get ariaLive() {
            return (props.live ?? true) ? ("polite" as const) : null;
          },
          get "data-variant"() {
            return props.variant ?? "default";
          },
          get "data-icon"() {
            return String(props.icon ?? true);
          },
        },
        rest,
      ),
      {
        get __zguiClass() {
          return classes(sheet.className, "zui-alert", `zui-alert--${props.variant ?? "default"}`);
        },
        get children() {
          return [mark, content];
        },
      },
    ),
  );
}
