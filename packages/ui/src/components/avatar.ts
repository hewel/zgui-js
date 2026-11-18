import { omit } from "#solid";
import { memo, mergeProps } from "../renderer/index.js";
import { Image } from "../image.js";
import type { ImageProps, NativeElementProps, NativeNode } from "../types.js";
import { classes, element, part, useStylesheet, withDefaults } from "./shared.js";
import { sheet } from "./generated/AvatarStyle.js";

export type AvatarSize = "sm" | "md" | "lg";
export interface AvatarProps extends NativeElementProps {
  src?: string | null;
  size?: AvatarSize;
  label?: string | null;
}
export type AvatarImageProps = ImageProps;
export type AvatarFallbackProps = NativeElementProps;
export interface AvatarBadgeProps extends NativeElementProps { label?: string | null; }
export interface AvatarGroupProps extends NativeElementProps { size?: AvatarSize; }
export type AvatarGroupCountProps = NativeElementProps;

export function AvatarImage(props: AvatarImageProps = {}): NativeNode {
  useStylesheet(sheet);
  return Image(mergeProps(
    props,
    withDefaults({ role: "image" as const, ariaHidden: true }, props),
    { get __zguiClass() { return classes("zui-avatar__image"); } },
  ));
}

export const AvatarFallback = part("box", sheet, "zui-avatar__fallback");
export const AvatarGroupCount = part("box", sheet, "zui-avatar-group__count");

export function AvatarBadge(props: AvatarBadgeProps = {}): NativeNode {
  useStylesheet(sheet);
  const rest = omit(props, "label");
  return element("box", mergeProps(
    rest,
    withDefaults({
      get role() { return props.label != null ? "image" as const : "genericContainer" as const; },
      get ariaHidden() { return props.label == null; },
      get ariaLabel() { return props.label ?? null; },
    }, rest),
    { get __zguiClass() { return classes("zui-avatar__badge"); } },
  ));
}

export function AvatarGroup(props: AvatarGroupProps = {}): NativeNode {
  useStylesheet(sheet);
  const rest = omit(props, "size");
  return element("box", mergeProps(
    rest,
    withDefaults({
      role: "group" as const,
      get "data-size"() { return props.size ?? "md"; },
    }, rest),
    { get __zguiClass() { return classes("zui-avatar-group"); } },
  ));
}

export function Avatar(props: AvatarProps = {}): NativeNode {
  useStylesheet(sheet);
  const rest = omit(props, "src", "size", "label", "children");
  const present = memo(() => props.src != null);
  const image = memo(() => present() ? AvatarImage({ get src() { return props.src; } }) : null);
  const content = memo(() => props.children);
  return element("box", mergeProps(
    rest,
    withDefaults({
      role: "image" as const,
      get ariaLabel() { return props.label ?? null; },
      get ariaHidden() { return props.label == null; },
      get "data-size"() { return props.size ?? "md"; },
    }, rest),
    {
      get __zguiClass() {
        return classes(sheet.className, "zui-avatar",
        props.size === "sm" && "zui-avatar--sm",
        props.size === "lg" && "zui-avatar--lg", );
      },
      get children() { return [content, image]; },
    },
  ));
}
