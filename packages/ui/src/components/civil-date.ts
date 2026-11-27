import { createMemo, untrack } from "#solid";
import { createControllable } from "./state.js";

export interface DateRangeValue {
  readonly start: string;
  readonly end: string | null;
}

export interface SingleDateSelectionProps {
  readonly mode?: "single" | undefined;
  readonly value?: string | null | undefined;
  readonly defaultValue?: string | null | undefined;
  readonly onValueChange?: ((value: string | null) => void) | undefined;
}

export interface RangeDateSelectionProps {
  readonly mode: "range";
  readonly value?: DateRangeValue | null | undefined;
  readonly defaultValue?: DateRangeValue | null | undefined;
  readonly onValueChange?: ((value: DateRangeValue | null) => void) | undefined;
}

export type DateSelectionProps = SingleDateSelectionProps | RangeDateSelectionProps;

interface CivilDate {
  readonly year: number;
  readonly month: number;
  readonly day: number;
}

const DAY_MS = 86_400_000;
const DAYS_PER_ERA = 146_097; // 400 Gregorian years.
const UNIX_EPOCH = 719_468; // Days from March 1, year 0, to January 1, 1970.
export const LAST_MONTH = 9999 * 12 + 11;

const pad = (value: number, width: number) => String(value).padStart(width, "0");
const monthYear = (index: number) => Math.floor(index / 12);
const monthNumber = (index: number) => (index % 12) + 1;
export const isoMonth = (index: number) =>
  `${pad(monthYear(index), 4)}-${pad(monthNumber(index), 2)}`;

export function monthIndex(value: string): number {
  if (!/^\d{4}-(0[1-9]|1[0-2])$/.test(value) || value.length !== 7)
    throw new RangeError(`Invalid ISO civil month: ${value}`);
  return Number(value.slice(0, 4)) * 12 + Number(value.slice(5)) - 1;
}

function monthLength(year: number, month: number): number {
  if (month === 2) return year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0) ? 29 : 28;
  return month === 4 || month === 6 || month === 9 || month === 11 ? 30 : 31;
}

// Count from March so the leap day is at the end of its year.
function ordinal(year: number, month: number, day: number): number {
  const marchYear = year - (month <= 2 ? 1 : 0);
  const era = Math.floor(marchYear / 400);
  const yearInEra = marchYear - era * 400;
  const marchMonth = month + (month > 2 ? -3 : 9);
  const dayInYear = Math.floor((153 * marchMonth + 2) / 5) + day - 1;
  return (
    era * DAYS_PER_ERA +
    yearInEra * 365 +
    Math.floor(yearInEra / 4) -
    Math.floor(yearInEra / 100) +
    dayInYear -
    UNIX_EPOCH
  );
}

export function parseDate(value: string): number {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value) || value.length !== 10)
    throw new RangeError(`Invalid ISO civil date: ${value}`);
  const index = monthIndex(value.slice(0, 7));
  const day = Number(value.slice(8));
  if (day < 1 || day > monthLength(monthYear(index), monthNumber(index)))
    throw new RangeError(`Invalid ISO civil date: ${value}`);
  return ordinal(monthYear(index), monthNumber(index), day);
}

export function civilDate(day: number): CivilDate {
  const epochDay = day + UNIX_EPOCH;
  const era = Math.floor(epochDay / DAYS_PER_ERA);
  const dayInEra = epochDay - era * DAYS_PER_ERA;
  // Remove prior leap days, putting the era's final leap day in year 399.
  const yearInEra = Math.floor(
    (dayInEra -
      Math.floor(dayInEra / 1460) +
      Math.floor(dayInEra / 36524) -
      Math.floor(dayInEra / 146096)) /
      365,
  );
  const dayInYear =
    dayInEra - (yearInEra * 365 + Math.floor(yearInEra / 4) - Math.floor(yearInEra / 100));
  const marchMonth = Math.floor((5 * dayInYear + 2) / 153);
  const month = marchMonth + (marchMonth < 10 ? 3 : -9);
  return {
    year: era * 400 + yearInEra + (month <= 2 ? 1 : 0),
    month,
    day: dayInYear - Math.floor((153 * marchMonth + 2) / 5) + 1,
  };
}

export const isoDate = (date: CivilDate) =>
  `${pad(date.year, 4)}-${pad(date.month, 2)}-${pad(date.day, 2)}`;
export const monthOf = (date: CivilDate) => date.year * 12 + date.month - 1;
export const firstDay = (index: number) => ordinal(monthYear(index), monthNumber(index), 1);
export const LAST_DAY = ordinal(9999, 12, 31);
export const FIRST_DAY = ordinal(0, 1, 1);
// Monday = 1; the epoch was a Thursday. Normalize the remainder for pre-epoch dates.
export const weekday = (day: number) => ((((day + 3) % 7) + 7) % 7) + 1;
export const weekOffset = (day: number, startsOn: number) => (weekday(day) - startsOn + 7) % 7;

export function moveToMonth(day: number, index: number): number {
  const year = monthYear(index);
  const month = monthNumber(index);
  return ordinal(year, month, Math.min(civilDate(day).day, monthLength(year, month)));
}

type Selection = string | DateRangeValue | null;

function validate(value: Selection | undefined): Selection | undefined {
  if (typeof value === "string") parseDate(value);
  else if (value != null) {
    parseDate(value.start);
    if (value.end !== null) parseDate(value.end);
  }
  return value;
}

export function createDateSelection(props: DateSelectionProps) {
  untrack(() => validate(props.value));
  const [read, write] = createControllable<Selection>(
    {
      value: () => validate(props.value),
      defaultValue: () => validate(props.defaultValue),
      onChange: () => next => {
        if (props.mode === "range") {
          if (typeof next !== "string") props.onValueChange?.(next);
        } else if (typeof next !== "object" || next === null) {
          props.onValueChange?.(next);
        }
      },
    },
    null,
  );
  const value = createMemo(read);
  return [value, write] as const;
}

interface CalendarLabels {
  readonly months: readonly string[];
  readonly weekdays: readonly string[];
  readonly firstDayOfWeek: number;
}

interface CalendarLocaleBinding {
  readonly getCalendarLabels: (options: {
    locale: string | undefined;
    width: "long" | "short";
  }) => CalendarLabels;
  readonly getSystemTimeZone: () => string;
  readonly toZonedDateTime: (epochMs: number, timeZone: string) => CivilDate;
  readonly formatDateTime: (
    epochMs: number,
    options: { locale: string | undefined; timeZone: string; dateStyle: "medium" },
  ) => string;
}

// The native host owns this binding; standalone UI does not install it.
const runtime = globalThis as typeof globalThis & {
  readonly __zguiServices?: CalendarLocaleBinding;
};

const MONTHS = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
];
const WEEKDAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
const SHORT_WEEKDAYS = WEEKDAYS.map(day => day.slice(0, 3));

export function calendarLabels(locale: string | undefined) {
  const host = runtime.__zguiServices;
  if (host) {
    const { months, firstDayOfWeek } = host.getCalendarLabels({ locale, width: "long" });
    const { weekdays } = host.getCalendarLabels({ locale, width: "short" });
    return { months, weekdays, firstDayOfWeek };
  }
  return {
    months: MONTHS,
    weekdays: SHORT_WEEKDAYS,
    firstDayOfWeek: locale === undefined || /^en-US(?:-|$)/i.test(locale) ? 7 : 1,
  };
}

export function calendarToday(): CivilDate {
  const host = runtime.__zguiServices;
  const now = Date.now();
  return host
    ? host.toZonedDateTime(now, host.getSystemTimeZone())
    : civilDate(Math.floor(now / DAY_MS));
}

export function formatCivilDate(value: string, locale: string | undefined): string {
  const host = runtime.__zguiServices;
  // The UTC carrier keeps a civil label unchanged even in zones that skipped an entire day.
  return host
    ? host.formatDateTime(parseDate(value) * DAY_MS, {
        locale,
        timeZone: "UTC",
        dateStyle: "medium",
      })
    : value;
}
