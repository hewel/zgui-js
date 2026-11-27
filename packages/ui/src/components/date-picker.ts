import { createEffect, createMemo, omit, untrack } from "#solid";
import { focus } from "../nodes.js";
import { mergeProps } from "../renderer/index.js";
import type { NativeElementProps, NativeNode } from "../types.js";
import { Button } from "./button.js";
import { dateBounds, mountCalendar, type CalendarDateOptions } from "./calendar.js";
import {
  createDateSelection,
  formatCivilDate,
  type DateRangeValue,
  type DateSelectionProps,
  type SingleDateSelectionProps,
  type RangeDateSelectionProps,
} from "./civil-date.js";
import { sheet } from "./generated/DatePickerStyle.js";
import { mountPopoverContent, Popover, PopoverTrigger } from "./popover.js";
import { classes, element, useStylesheet } from "./shared.js";
import { createControllable } from "./state.js";

type DatePickerOptions = Omit<NativeElementProps, "children" | "disabled" | "open"> &
  CalendarDateOptions & {
    readonly open?: boolean | undefined;
    readonly defaultOpen?: boolean | undefined;
    readonly onOpenChange?: ((open: boolean) => void) | undefined;
    readonly placeholder?: string | undefined;
    readonly clearable?: boolean | undefined;
    readonly closeOnSelect?: boolean | undefined;
    readonly triggerTestId?: string | undefined;
    readonly contentTestId?: string | undefined;
    readonly calendarTestId?: string | undefined;
  };

export type DatePickerProps = DatePickerOptions & DateSelectionProps;

/** A popover-anchored civil date or date range input. */
export function DatePicker(props?: DatePickerOptions & SingleDateSelectionProps): NativeNode;
export function DatePicker(props: DatePickerOptions & RangeDateSelectionProps): NativeNode;
export function DatePicker(props: DatePickerProps): NativeNode;
export function DatePicker(props: DatePickerProps = {}): NativeNode {
  untrack(() => dateBounds(props.minDate, props.maxDate));
  const [value, setValue] = createDateSelection(props);
  useStylesheet(sheet);
  const [readOpen, writeOpen] = createControllable<boolean>(
    {
      value: () => props.open,
      defaultValue: () => props.defaultOpen,
      onChange: () => props.onOpenChange,
    },
    false,
  );
  const open = createMemo(() => readOpen() && !props.disabled);
  let trigger: NativeNode | undefined;
  createEffect(open, (isOpen, wasOpen) => {
    if (!isOpen && wasOpen && trigger && !props.disabled) focus(trigger, { reveal: true });
  });

  const label = createMemo(() => {
    const current = value();
    if (current === null) return props.placeholder ?? "Pick a date";
    if (typeof current === "string") return formatCivilDate(current, props.locale);
    const start = formatCivilDate(current.start, props.locale);
    return current.end === null
      ? `${start} – …`
      : `${start} – ${formatCivilDate(current.end, props.locale)}`;
  });

  function select(next: string | DateRangeValue | null) {
    setValue(next);
    if (
      props.closeOnSelect !== false &&
      next !== null &&
      (typeof next === "string" || next.end !== null)
    )
      writeOpen(false);
  }

  const calendarOptions = {
    get testId() {
      return props.calendarTestId;
    },
    get numberOfMonths() {
      return props.numberOfMonths;
    },
    get locale() {
      return props.locale;
    },
    get weekStartsOn() {
      return props.weekStartsOn;
    },
    get minDate() {
      return props.minDate;
    },
    get maxDate() {
      return props.maxDate;
    },
    get isDateDisabled() {
      return props.isDateDisabled;
    },
    get captionLayout() {
      return props.captionLayout;
    },
    get yearRange() {
      return props.yearRange;
    },
    get disabled() {
      return props.disabled;
    },
  };
  const rest = omit(
    props,
    "mode",
    "value",
    "defaultValue",
    "onValueChange",
    "numberOfMonths",
    "locale",
    "weekStartsOn",
    "minDate",
    "maxDate",
    "isDateDisabled",
    "captionLayout",
    "yearRange",
    "disabled",
    "open",
    "defaultOpen",
    "onOpenChange",
    "placeholder",
    "clearable",
    "closeOnSelect",
    "triggerTestId",
    "contentTestId",
    "calendarTestId",
  );
  return element(
    "column",
    mergeProps(rest, {
      get class() {
        return classes(sheet.className, props.class);
      },
      children: Popover({
        get open() {
          return open();
        },
        onOpenChange(next) {
          if (!next || !props.disabled) writeOpen(next);
        },
        get children() {
          trigger = PopoverTrigger({
            class: "zui-date-picker__trigger",
            get testId() {
              return props.triggerTestId;
            },
            get disabled() {
              return props.disabled;
            },
            get "data-empty"() {
              return value() === null ? "true" : null;
            },
            children: element("text", { class: "zui-date-picker__label", children: label }),
          });
          return [
            element("row", {
              children: [
                trigger,
                () =>
                  props.clearable
                    ? Button({
                        variant: "ghost",
                        size: "icon",
                        ariaLabel: "Clear date",
                        get disabled() {
                          return props.disabled || value() === null;
                        },
                        onClick() {
                          if (props.disabled || value() === null) return;
                          setValue(null);
                          if (trigger) focus(trigger, { reveal: true });
                        },
                        children: "×",
                      })
                    : null,
              ],
            }),
            mountPopoverContent(
              {
                class: "zui-date-picker__surface",
                style: { width: "max-content", padding: "0px" },
                get testId() {
                  return props.contentTestId;
                },
                get inert() {
                  return !open();
                },
                children: () =>
                  props.mode === "range"
                    ? mountCalendar(
                        mergeProps(calendarOptions, {
                          mode: "range" as const,
                          get value() {
                            const current = value();
                            return typeof current === "string" ? null : current;
                          },
                          onValueChange: select,
                        }),
                        open,
                      )
                    : mountCalendar(
                        mergeProps(calendarOptions, {
                          get value() {
                            const current = value();
                            return typeof current === "string" ? current : null;
                          },
                          onValueChange: select,
                        }),
                        open,
                      ),
              },
              { autoFocus: false, restore: false },
            ),
          ];
        },
      }),
    }),
  );
}
