import { splitProps } from "#solid";
import { memo, mergeProps } from "../renderer/index.js";
import type { NativeElementProps, NativeNode } from "../types.js";
import { classes, element, part, useStylesheet } from "./shared.js";
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
  const [own, rest] = splitProps(props, ["variant", "live", "icon", "children"]);
  const mark = memo(() => (own.icon ?? true) ? Icon({
    get icon() {
      return (own.variant ?? "default") === "destructive" ? ALERT_TRIANGLE : INFO;
    },
    class: "zui-alert__icon",
  }) : null);
  const content = memo(() => own.children);
  return element("box", mergeProps(
    {
      get role() { return (own.live ?? true) ? "alert" as const : "group" as const; },
      get ariaLive() { return (own.live ?? true) ? "polite" as const : null; },
      get "data-variant"() { return own.variant ?? "default"; },
      get "data-icon"() { return String(own.icon ?? true); },
    },
    rest,
    {
      get class() {
        return classes(
          sheet.className, "zui-alert", `zui-alert--${own.variant ?? "default"}`,
          rest.class,
        );
      },
      get children() {
        return [mark, content];
      },
    },
  ));
}
