import { splitProps } from "#solid";
import { memo, mergeProps } from "../renderer/index.js";
import { Image } from "../image.js";
import type { ImageProps, NativeElementProps, NativeNode } from "../types.js";
import { classes, element, part, useStylesheet } from "./shared.js";
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
    { role: "image" as const, ariaHidden: true },
    props,
    { get __zguiClass() { return classes("zui-avatar__image"); } },
  ));
}

export const AvatarFallback = part("box", sheet, "zui-avatar__fallback");
export const AvatarGroupCount = part("box", sheet, "zui-avatar-group__count");

export function AvatarBadge(props: AvatarBadgeProps = {}): NativeNode {
  useStylesheet(sheet);
  const [own, rest] = splitProps(props, ["label"]);
  return element("box", mergeProps(
    {
      get role() { return own.label != null ? "image" as const : "genericContainer" as const; },
      get ariaHidden() { return own.label == null; },
      get ariaLabel() { return own.label ?? null; },
    },
    rest,
    { get __zguiClass() { return classes("zui-avatar__badge"); } },
  ));
}

export function AvatarGroup(props: AvatarGroupProps = {}): NativeNode {
  useStylesheet(sheet);
  const [own, rest] = splitProps(props, ["size"]);
  return element("box", mergeProps(
    {
      role: "group" as const,
      get "data-size"() { return own.size ?? "md"; },
    },
    rest,
    { get __zguiClass() { return classes("zui-avatar-group"); } },
  ));
}

export function Avatar(props: AvatarProps = {}): NativeNode {
  useStylesheet(sheet);
  const [own, rest] = splitProps(props, ["src", "size", "label", "children"]);
  const present = memo(() => own.src != null);
  const image = memo(() => present() ? AvatarImage({ get src() { return own.src; } }) : null);
  const content = memo(() => own.children);
  return element("box", mergeProps(
    {
      role: "image" as const,
      get ariaLabel() { return own.label ?? null; },
      get ariaHidden() { return own.label == null; },
      get "data-size"() { return own.size ?? "md"; },
    },
    rest,
    {
      get __zguiClass() {
        return classes(sheet.className, "zui-avatar",
        own.size === "sm" && "zui-avatar--sm",
        own.size === "lg" && "zui-avatar--lg", );
      },
      get children() { return [content, image]; },
    },
  ));
}
