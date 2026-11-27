export type ZonedDisambiguation = "compatible" | "earlier" | "later" | "reject";

export interface ZonedDisambiguationOptions {
  /** Compatible chooses the earlier instant in a fold and the later one in a gap. */
  readonly disambiguation?: ZonedDisambiguation | undefined;
}

export interface CivilDateTimeInput {
  readonly year: number;
  readonly month: number;
  readonly day: number;
  readonly hour?: number | undefined;
  readonly minute?: number | undefined;
  readonly second?: number | undefined;
  readonly millisecond?: number | undefined;
}

export interface ZonedDateTimeParts {
  readonly year: number;
  readonly month: number;
  readonly day: number;
  readonly hour: number;
  readonly minute: number;
  readonly second: number;
  readonly millisecond: number;
  /** ISO weekday, 1 for Monday through 7 for Sunday. */
  readonly dayOfWeek: number;
  readonly offsetSeconds: number;
  readonly inDst: boolean;
  readonly timeZone: string;
  readonly zoneAbbreviation: string;
  readonly daysInMonth: number;
}

/** Whole units of the same sign. Calendar units precede elapsed clock units. */
export interface ZonedDurationInput {
  readonly years?: number | undefined;
  readonly months?: number | undefined;
  readonly weeks?: number | undefined;
  readonly days?: number | undefined;
  readonly hours?: number | undefined;
  readonly minutes?: number | undefined;
  readonly seconds?: number | undefined;
  readonly milliseconds?: number | undefined;
}

export interface AddZonedDurationOptions extends ZonedDisambiguationOptions {
  readonly timeZone?: string | undefined;
}

type DateTimeStyle = "full" | "long" | "medium" | "short";

export interface FormatDateTimeOptions {
  readonly locale?: string | undefined;
  readonly timeZone?: string | undefined;
  readonly dateStyle?: DateTimeStyle | undefined;
  readonly timeStyle?: DateTimeStyle | undefined;
  readonly hourCycle?: "h12" | "h23" | undefined;
}

export interface CalendarLabelsOptions {
  readonly locale?: string | undefined;
  readonly width?: "long" | "short" | "narrow" | undefined;
}

export interface CalendarLabels {
  readonly locale: string;
  /** Standalone Gregorian month labels, January first. */
  readonly months: readonly string[];
  /** Standalone weekday labels, Monday first regardless of locale. */
  readonly weekdays: readonly string[];
  /** Locale-specific week start as an ISO weekday (1–7). */
  readonly firstDayOfWeek: number;
}

export interface CollatorOptions {
  readonly locale?: string | undefined;
  /** Compare digit runs by numeric value. Defaults to true. */
  readonly numeric?: boolean | undefined;
  readonly sensitivity?: "base" | "accent" | "case" | "variant" | undefined;
}

interface HostLocaleBinding {
  readonly compareStrings: (a: string, b: string, options?: CollatorOptions) => number;
  readonly collatorSortIndices: (
    values: readonly string[],
    options?: CollatorOptions,
  ) => Uint32Array<ArrayBuffer>;
  readonly getSystemTimeZone: () => string;
  readonly getSystemLocale: () => string;
  readonly isValidTimeZone: (timeZone: string) => boolean;
  readonly toZonedDateTime: (epochMs: number, timeZone?: string) => ZonedDateTimeParts;
  readonly fromZonedDateTime: (
    parts: CivilDateTimeInput,
    timeZone?: string,
    options?: ZonedDisambiguationOptions,
  ) => number;
  readonly addZonedDuration: (
    epochMs: number,
    duration: ZonedDurationInput,
    options?: AddZonedDurationOptions,
  ) => number;
  readonly formatDateTime: (epochMs: number, options?: FormatDateTimeOptions) => string;
  readonly getCalendarLabels: (options?: CalendarLabelsOptions) => CalendarLabels;
}

function hostLocale(): HostLocaleBinding {
  // Native installs this binding before any application module is evaluated.
  const runtime = globalThis as unknown as { readonly __zguiServices: HostLocaleBinding };
  return runtime.__zguiServices;
}

/** Active session's IANA timezone. Call within mount, an effect, or a handler. */
export function getSystemTimeZone(): string {
  return hostLocale().getSystemTimeZone();
}

/** Active session's canonical BCP-47 locale. Call within mount, an effect, or a handler. */
export function getSystemLocale(): string {
  return hostLocale().getSystemLocale();
}

/** Whether this IANA zone (or alias) is available. Fixed offsets are not zone names. */
export function isValidTimeZone(timeZone: string): boolean {
  return hostLocale().isValidTimeZone(timeZone);
}

/** Convert whole epoch milliseconds in `timeZone` (defaults to the session zone). */
export function toZonedDateTime(epochMs: number, timeZone?: string): ZonedDateTimeParts {
  return hostLocale().toZonedDateTime(epochMs, timeZone);
}

/** Convert a civil time to epoch milliseconds. Missing clock fields are zero. Invalid dates and rejected gaps/folds throw RangeError. */
export function fromZonedDateTime(
  parts: CivilDateTimeInput,
  timeZone?: string,
  options?: ZonedDisambiguationOptions,
): number {
  return hostLocale().fromZonedDateTime(parts, timeZone, options);
}

/** Add calendar units as local time, then clock units as elapsed time. */
export function addZonedDuration(
  epochMs: number,
  duration: ZonedDurationInput,
  options?: AddZonedDurationOptions,
): number {
  return hostLocale().addZonedDuration(epochMs, duration, options);
}

/** Format in the requested locale and zone, defaulting to the session. With no styles both are medium; with one style only that part is shown. */
export function formatDateTime(epochMs: number, options?: FormatDateTimeOptions): string {
  return hostLocale().formatDateTime(epochMs, options);
}

/** Localized labels for a Gregorian calendar, defaulting to the session locale and long width. */
export function getCalendarLabels(options?: CalendarLabelsOptions): CalendarLabels {
  return hostLocale().getCalendarLabels(options);
}

/** Compare two strings with the session's locale unless overridden. Returns -1, 0, or 1. */
export function compareStrings(a: string, b: string, options?: CollatorOptions): number {
  return hostLocale().compareStrings(a, b, options);
}

/** Return original indices in stable collation order; accepts at most 100,000 strings and 8 MiB of UTF-8. */
export function collatorSortIndices(
  values: readonly string[],
  options?: CollatorOptions,
): Uint32Array<ArrayBuffer> {
  return hostLocale().collatorSortIndices(values, options);
}
