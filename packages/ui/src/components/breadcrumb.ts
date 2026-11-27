import { omit } from "#solid";
import type { NativeElementProps, NativeNode } from "../types.js";
import { CHEVRON_RIGHT, ELLIPSIS, Icon } from "./icons.js";
import { sheet } from "./generated/BreadcrumbStyle.js";
import { part, slot } from "./shared.js";

export const Breadcrumb = part("box", sheet, sheet.className, {
  role: "navigation",
  ariaLabel: "Breadcrumb",
});
export const BreadcrumbList = part("row", sheet, "zui-breadcrumb__list", { role: "list" });
export const BreadcrumbItem = part("box", sheet, "zui-breadcrumb__item", { role: "listItem" });
// Native control defaults activate on Enter and Space, including for an explicit link role.
export const BreadcrumbLink = part("control", sheet, "zui-breadcrumb__link", {
  role: "link",
  tabIndex: 0,
});
export const BreadcrumbPage = part("text", sheet, "zui-breadcrumb__page", {
  role: "label",
  ariaCurrent: "page",
});

const separator = part("box", sheet, "zui-breadcrumb__separator", { ariaHidden: true });
const ellipsis = part("box", sheet, "zui-breadcrumb__ellipsis", {
  role: "image",
  ariaLabel: "Collapsed path",
});

export function BreadcrumbSeparator(props: NativeElementProps = {}): NativeNode {
  const root = separator(omit(props, "children"));
  slot(root, () => props.children ?? Icon({ icon: CHEVRON_RIGHT }));
  return root;
}

export function BreadcrumbEllipsis(props: NativeElementProps = {}): NativeNode {
  const root = ellipsis(omit(props, "children"));
  slot(root, () => props.children ?? Icon({ icon: ELLIPSIS }));
  return root;
}
