import { omit, useContext } from "#solid";
import { mergeProps } from "../renderer/index.js";
import type { NativeChild, NativeElementProps, NativeNode } from "../types.js";
import type { ButtonProps } from "./button.js";
import { sheet as alertSheet } from "./generated/AlertDialogStyle.js";
import { sheet as dialogSheet } from "./generated/DialogStyle.js";
import { OverlayContext, modalSurface, overlayClose } from "./overlay.js";
import { part, useStylesheet, withDefaults } from "./shared.js";

export {
  Dialog as AlertDialog,
  DialogTrigger as AlertDialogTrigger,
  DialogHeader as AlertDialogHeader,
  DialogTitle as AlertDialogTitle,
  DialogDescription as AlertDialogDescription,
} from "./dialog.js";
export type { OverlayRootProps as AlertDialogProps } from "./overlay.js";

export interface AlertDialogContentProps extends Omit<NativeElementProps, "role"> {
  readonly size?: "default" | "sm" | undefined;
  readonly dismissOnEscape?: boolean | undefined;
}

export function AlertDialogContent(props: AlertDialogContentProps = {}): NativeChild {
  useStylesheet(dialogSheet);
  useStylesheet(alertSheet);
  const context = useContext(OverlayContext);
  const rest = omit(props, "size");
  return modalSurface(
    context,
    mergeProps(
      rest,
      withDefaults(
        {
          get ariaLabelledBy() {
            return context?.title.node() ?? null;
          },
          get ariaDescribedBy() {
            return context?.description.node() ?? null;
          },
        },
        rest,
      ),
      {
        role: "alertDialog" as const,
        dismissOnOutsidePress: false,
        get "data-size"() {
          return props.size ?? "default";
        },
      },
    ),
    { class: `${dialogSheet.className} ${alertSheet.className} zui-dialog`, role: "alertDialog" },
  );
}

export const AlertDialogMedia = part("box", alertSheet, "zui-alert-dialog__media");
export const AlertDialogFooter = part("row", dialogSheet, "zui-dialog__footer");

export function AlertDialogAction(props: ButtonProps = {}): NativeNode {
  return overlayClose(mergeProps(props, withDefaults({ variant: "default" as const }, props)));
}

export function AlertDialogCancel(props: ButtonProps = {}): NativeNode {
  return overlayClose(mergeProps(props, withDefaults({ variant: "outline" as const }, props)));
}
