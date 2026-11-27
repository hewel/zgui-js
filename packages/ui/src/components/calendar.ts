import { For, createEffect, createMemo, createSignal, omit, onCleanup, untrack } from "#solid";
import { focus } from "../nodes.js";
import { mergeProps } from "../renderer/index.js";
import type { NativeKeyEvent, NativeElementProps, NativeNode } from "../types.js";
import {
  FIRST_DAY,
  LAST_DAY,
  LAST_MONTH,
  calendarLabels,
  calendarToday,
  civilDate,
  createDateSelection,
  firstDay,
  isoDate,
  isoMonth,
  monthIndex,
  monthOf,
  moveToMonth,
  parseDate,
  weekOffset,
  type DateSelectionProps,
  type SingleDateSelectionProps,
  type RangeDateSelectionProps,
} from "./civil-date.js";
import { sheet } from "./generated/CalendarStyle.js";
import { Select, type SelectOption } from "./select.js";
import { classes, element, useStylesheet } from "./shared.js";
import { createControllable } from "./state.js";

export type { DateRangeValue } from "./civil-date.js";

export interface CalendarDateOptions {
  readonly numberOfMonths?: 1 | 2 | undefined;
  readonly locale?: string | undefined;
  /** ISO weekday (Monday = 1, Sunday = 7). Defaults to the locale's first weekday. */
  readonly weekStartsOn?: 1 | 2 | 3 | 4 | 5 | 6 | 7 | undefined;
  readonly minDate?: string | undefined;
  readonly maxDate?: string | undefined;
  readonly isDateDisabled?: ((date: string) => boolean) | undefined;
  readonly captionLayout?: "label" | "dropdown" | undefined;
  /** Inclusive year choices for the dropdown. Does not limit arrow navigation. */
  readonly yearRange?: readonly [number, number] | undefined;
  readonly disabled?: boolean | undefined;
}

type CalendarOptions = Omit<NativeElementProps, "children" | "disabled"> &
  CalendarDateOptions & {
    readonly month?: string | undefined;
    readonly defaultMonth?: string | undefined;
    readonly onMonthChange?: ((month: string) => void) | undefined;
  };

export type CalendarProps = CalendarOptions & DateSelectionProps;

export function dateBounds(minDate: string | undefined, maxDate: string | undefined) {
  const min = minDate === undefined ? FIRST_DAY : parseDate(minDate);
  const max = maxDate === undefined ? LAST_DAY : parseDate(maxDate);
  if (max < min) throw new RangeError("maxDate must not precede minDate");
  return { min, max };
}

const clamp = (n: number, min: number, max: number) => Math.max(min, Math.min(n, max));
const dayMonth = (day: number) => monthOf(civilDate(day));

/** A Gregorian date grid with controlled or uncontrolled selection and visible month. */
export function Calendar(props?: CalendarOptions & SingleDateSelectionProps): NativeNode;
export function Calendar(props: CalendarOptions & RangeDateSelectionProps): NativeNode;
export function Calendar(props: CalendarProps): NativeNode;
export function Calendar(props: CalendarProps = {}): NativeNode {
  return mountCalendar(props);
}

export function mountCalendar(props: CalendarProps, autoFocus?: () => boolean): NativeNode {
  untrack(() => {
    dateBounds(props.minDate, props.maxDate);
    if (props.month !== undefined) monthIndex(props.month);
    if (props.defaultMonth !== undefined) monthIndex(props.defaultMonth);
  });
  const [value, setValue] = createDateSelection(props);
  useStylesheet(sheet);
  const bounds = createMemo(() => dateBounds(props.minDate, props.maxDate));
  const today = parseDate(isoDate(calendarToday()));
  const labels = createMemo(() => calendarLabels(props.locale));
  const weekStart = () => props.weekStartsOn ?? labels().firstDayOfWeek;
  const selection = createMemo(() => {
    const current = value();
    if (current === null) return null;
    if (typeof current === "string") return { start: parseDate(current), end: null };
    return {
      start: parseDate(current.start),
      end: current.end === null ? null : parseDate(current.end),
    };
  });
  const [hover, setHover] = createSignal<number | undefined>();
  const [readMonth, writeMonth] = createControllable<string>(
    {
      value: () => props.month,
      defaultValue: () => props.defaultMonth,
      onChange: () => props.onMonthChange,
    },
    untrack(() => isoMonth(dayMonth(selection()?.start ?? today))),
  );
  const count = () => props.numberOfMonths ?? 1;
  const limits = createMemo(() => {
    const { min, max } = bounds();
    const minMonth = dayMonth(min);
    const maxMonth = dayMonth(max);
    const first = Math.min(minMonth, LAST_MONTH - count() + 1);
    return { first, last: Math.max(first, maxMonth - count() + 1), minMonth, maxMonth };
  });
  const month = createMemo(() => clamp(monthIndex(readMonth()), limits().first, limits().last));
  const isVisible = (index: number) => index >= month() && index < month() + count();
  const [active, setActive] = createSignal<number | undefined>();
  const [pending, requestFocus] = createSignal<{ day: number; month: number } | undefined>();
  const nodes = new Map<number, NativeNode>();
  const isDisabled = (day: number, iso?: string) => {
    const { min, max } = bounds();
    return (
      !!props.disabled ||
      day < min ||
      day > max ||
      !!props.isDateDisabled?.(iso ?? isoDate(civilDate(day)))
    );
  };
  const tabStop = createMemo(() => {
    if (props.disabled) return undefined;
    const preferred = active() ?? selection()?.start ?? today;
    const start = Math.max(bounds().min, firstDay(month()));
    const end = Math.min(bounds().max, firstDay(month() + count()) - 1);
    if (preferred >= start && preferred <= end && !isDisabled(preferred)) return preferred;
    for (let day = start; day <= end; day++) if (!isDisabled(day)) return day;
    return undefined;
  });

  createEffect(
    () => [pending(), month(), autoFocus?.() ? tabStop() : undefined] as const,
    ([request, visible, initial], previous) => {
      const day =
        request?.month === visible && (request !== previous?.[0] || visible !== previous?.[1])
          ? request.day
          : previous?.[2] === undefined
            ? initial
            : undefined;
      if (day !== undefined && !isDisabled(day)) {
        const node = nodes.get(day);
        if (node) focus(node, { reveal: true });
      }
    },
  );

  function changeMonth(next: number) {
    const { first, last } = limits();
    const target = clamp(next, first, last);
    if (!props.disabled && target !== month()) {
      requestFocus(undefined);
      setHover(undefined);
      writeMonth(isoMonth(target));
    }
  }

  function moveFocus(day: number) {
    const index = dayMonth(day);
    const { first, last } = limits();
    const target = isVisible(index)
      ? month()
      : clamp(index < month() ? index : index - count() + 1, first, last);
    setActive(day);
    if (target !== month()) {
      requestFocus({ day, month: target });
      writeMonth(isoMonth(target));
    } else {
      requestFocus(undefined);
      const node = nodes.get(day);
      if (node) focus(node, { reveal: true });
    }
  }

  function choose(day: number, iso: string) {
    if (isDisabled(day, iso)) return;
    const current = selection();
    setHover(undefined);
    if (props.mode !== "range") setValue(iso);
    else if (current === null || current.end !== null) setValue({ start: iso, end: null });
    else if (day < current.start) setValue({ start: iso, end: isoDate(civilDate(current.start)) });
    else setValue({ start: isoDate(civilDate(current.start)), end: iso });
    if (!isVisible(dayMonth(day))) moveFocus(day);
  }

  function onDayKey(event: NativeKeyEvent, day: number, iso: string) {
    if (event.isComposing || event.modifiers.alt || event.modifiers.meta || event.modifiers.control)
      return;
    const key = event.key;
    if (key === "Enter" || key === " " || key === "Space") {
      event.preventDefault();
      choose(day, iso);
      return;
    }
    const offset = weekOffset(day, weekStart());
    let target: number;
    let stride: number;
    let length: number;
    switch (key) {
      case "ArrowLeft":
      case "ArrowRight":
      case "ArrowUp":
      case "ArrowDown":
        stride = key === "ArrowLeft" ? -1 : key === "ArrowRight" ? 1 : key === "ArrowUp" ? -7 : 7;
        target = day + stride;
        length = Math.ceil(31 / Math.abs(stride));
        break;
      case "Home":
      case "End":
        target = day - offset + (key === "End" ? 6 : 0);
        stride = key === "Home" ? 1 : -1;
        length = 7;
        break;
      case "PageUp":
      case "PageDown": {
        stride = key === "PageUp" ? -1 : 1;
        const index = dayMonth(day) + stride * (event.modifiers.shift ? 12 : 1);
        event.preventDefault();
        if (index < limits().minMonth || index > limits().maxMonth) return;
        const start = Math.max(firstDay(index), bounds().min);
        const end = Math.min(firstDay(index + 1) - 1, bounds().max);
        target = clamp(moveToMonth(day, index), start, end);
        length = stride < 0 ? target - start + 1 : end - target + 1;
        break;
      }
      default:
        return;
    }
    event.preventDefault();
    for (let step = 0; step < length; step++, target += stride) {
      if (target === day) return;
      if (!isDisabled(target)) {
        moveFocus(target);
        return;
      }
    }
  }

  const committedRange = createMemo(() => {
    const current = selection();
    if (!current || props.mode !== "range") return undefined;
    const end = current.end ?? current.start;
    return [Math.min(current.start, end), Math.max(current.start, end)] as const;
  });
  const paintedRange = createMemo(() => {
    const current = selection();
    const target = hover();
    if (
      props.mode !== "range" ||
      !current ||
      current.end !== null ||
      target === undefined ||
      isDisabled(target)
    )
      return committedRange();
    return [Math.min(current.start, target), Math.max(current.start, target)] as const;
  });

  function renderDay(day: number, index: number): NativeNode {
    if (day < FIRST_DAY || day > LAST_DAY)
      return element("box", { role: "gridCell", ariaHidden: true });
    const date = civilDate(day);
    const iso = isoDate(date);
    const outside = monthOf(date) !== index;
    const disabled = createMemo(() => isDisabled(day, iso));
    const single = () => props.mode !== "range" && selection()?.start === day;
    const rangeMark = () => {
      const range = paintedRange();
      if (!range || day < range[0] || day > range[1]) return null;
      return day === range[0] ? "start" : day === range[1] ? "end" : "middle";
    };
    const button = element("control", {
      class: "zui-calendar__day",
      role: "button",
      ariaLabel: outside ? `${iso} (outside month)` : iso,
      "data-outside": outside ? "true" : null,
      "data-today": today === day ? "true" : null,
      get tabIndex() {
        return !outside && tabStop() === day ? 0 : -1;
      },
      get disabled() {
        return disabled();
      },
      get selected() {
        const range = committedRange();
        return single() || (range !== undefined && day >= range[0] && day <= range[1]);
      },
      get "data-selected"() {
        return single() || rangeMark() !== null ? "true" : null;
      },
      get "data-selected-single"() {
        return single() ? "true" : null;
      },
      get "data-range"() {
        return rangeMark();
      },
      onFocus() {
        if (!outside) setActive(day);
      },
      onPointerEnter() {
        if (props.mode === "range" && selection()?.end === null && !disabled()) setHover(day);
      },
      onPointerLeave() {
        if (hover() === day) setHover(undefined);
      },
      onClick() {
        choose(day, iso);
      },
      onKeyDown(event) {
        onDayKey(event, day, iso);
      },
      children: element("box", { class: "zui-calendar__day-mark", children: String(date.day) }),
    });
    if (!outside) {
      nodes.set(day, button);
      onCleanup(() => {
        if (nodes.get(day) === button) nodes.delete(day);
      });
    }
    return element("box", { role: "gridCell", children: button });
  }

  function caption(index: number): NativeNode {
    const date = civilDate(firstDay(index));
    return element("row", {
      class: "zui-calendar__caption",
      children: () =>
        props.captionLayout === "dropdown"
          ? dropdowns(date.year, date.month, index)
          : element("text", {
              class: "zui-calendar__heading",
              role: "heading",
              ariaLive: "polite",
              get children() {
                return `${labels().months[date.month - 1]} ${date.year}`;
              },
            }),
    });
  }

  function dropdowns(year: number, selectedMonth: number, index: number): NativeNode {
    const monthOptions = createMemo(() =>
      labels().months.map((label, i) => {
        const target = year * 12 + i;
        return {
          label,
          value: String(i + 1),
          disabled: target < limits().minMonth || target > limits().maxMonth,
        };
      }),
    );
    const yearOptions = createMemo(() => {
      const { minMonth, maxMonth } = limits();
      const [from, to] = props.yearRange ?? [year - 100, year + 100];
      if (!Number.isInteger(from) || !Number.isInteger(to) || to < from)
        throw new RangeError("Invalid Calendar yearRange");
      const first = Math.max(Math.floor(minMonth / 12), from);
      const last = Math.min(Math.floor(maxMonth / 12), to);
      const options: SelectOption[] = Array.from(
        { length: Math.max(0, last - first + 1) },
        (_, i) => {
          const value = String(first + i);
          return { value, label: value };
        },
      );
      // Keep the current caption visible without making years outside the range selectable.
      if (year < first || year > last) {
        const value = String(year);
        const current = { value, label: value, disabled: true };
        if (year < first) options.unshift(current);
        else options.push(current);
      }
      return options;
    });
    return element("row", {
      children: [
        Select({
          label: "Month",
          size: "sm",
          value: String(selectedMonth),
          get disabled() {
            return props.disabled;
          },
          get options() {
            return monthOptions();
          },
          onValueChange(next) {
            changeMonth(year * 12 + Number(next) - 1 - (index - month()));
          },
        }),
        Select({
          label: "Year",
          size: "sm",
          value: String(year),
          get disabled() {
            return props.disabled;
          },
          get options() {
            return yearOptions();
          },
          onValueChange(next) {
            const { minMonth, maxMonth } = limits();
            const target = clamp(Number(next) * 12 + selectedMonth - 1, minMonth, maxMonth);
            changeMonth(target - (index - month()));
          },
        }),
      ],
    });
  }

  function renderMonth(index: number): NativeNode {
    const weekdayHeader = element("box", {
      class: "zui-calendar__week",
      role: "row",
      children: Array.from({ length: 7 }, (_, column) =>
        element("text", {
          class: "zui-calendar__weekday",
          role: "columnHeader",
          get children() {
            return labels().weekdays[(weekStart() - 1 + column) % 7];
          },
        }),
      ),
    });
    const weeks = createMemo(() => {
      const first = firstDay(index);
      const start = first - weekOffset(first, weekStart());
      const length = Math.ceil((firstDay(index + 1) - start) / 7);
      return Array.from({ length }, (_, row) => start + row * 7);
    });
    return element("column", {
      class: "zui-calendar__month",
      children: [
        caption(index),
        element("box", {
          class: "zui-calendar__grid",
          role: "grid",
          get ariaLabel() {
            return `${labels().months[index % 12]} ${Math.floor(index / 12)}`;
          },
          children: [
            weekdayHeader,
            For({
              get each() {
                return weeks();
              },
              children(start: number) {
                return element("box", {
                  class: "zui-calendar__week",
                  role: "row",
                  children: Array.from({ length: 7 }, (_, i) => renderDay(start + i, index)),
                });
              },
            }),
          ],
        }),
      ],
    });
  }

  const nav = element("row", {
    class: "zui-calendar__nav",
    role: "navigation",
    ariaLabel: "Calendar navigation",
    // The absolute navigation row spans the captions; only its buttons should intercept input.
    __zguiInvariantStyle: { pointerEvents: "none" },
    children: ([-1, 1] as const).map(delta => {
      const disabled = () =>
        !!props.disabled || (delta < 0 ? month() <= limits().first : month() >= limits().last);
      return element("control", {
        class: "zui-calendar__step",
        role: "button",
        ariaLabel: delta < 0 ? "Previous month" : "Next month",
        tabIndex: 0,
        __zguiInvariantStyle: { pointerEvents: "auto" },
        get disabled() {
          return disabled();
        },
        get "data-disabled"() {
          return disabled() ? "true" : null;
        },
        onClick() {
          if (!disabled()) changeMonth(month() + delta);
        },
        children: delta < 0 ? "‹" : "›",
      });
    }),
  });
  const rest = omit(
    props,
    "mode",
    "value",
    "defaultValue",
    "onValueChange",
    "month",
    "defaultMonth",
    "onMonthChange",
    "numberOfMonths",
    "locale",
    "weekStartsOn",
    "minDate",
    "maxDate",
    "isDateDisabled",
    "captionLayout",
    "yearRange",
    "disabled",
  );
  return element(
    "column",
    mergeProps(rest, {
      get class() {
        return classes(sheet.className, props.class);
      },
      children: element("row", {
        class: "zui-calendar__months",
        children: [
          For({
            get each() {
              return Array.from({ length: count() }, (_, i) => month() + i);
            },
            children: renderMonth,
          }),
          nav,
        ],
      }),
    }),
  );
}
