import "@hewel/zgui/runtime-globals";
import {
  addZonedDuration,
  collatorSortIndices,
  compareStrings,
  formatDateTime,
  fromZonedDateTime,
  getCalendarLabels,
  getSystemLocale,
  getSystemTimeZone,
  isValidTimeZone,
  toZonedDateTime,
  type AddZonedDurationOptions,
  type CalendarLabels,
  type CalendarLabelsOptions,
  type CivilDateTimeInput,
  type CollatorOptions,
  type FormatDateTimeOptions,
  type ZonedDateTimeParts,
  type ZonedDisambiguation,
  type ZonedDisambiguationOptions,
  type ZonedDurationInput,
} from "@hewel/zgui-services";

export function localeTypes(): void {
  const zone: string = getSystemTimeZone();
  const locale: string = getSystemLocale();
  const valid: boolean = isValidTimeZone(zone);
  const civil: CivilDateTimeInput = { year: 2026, month: 3, day: 8, hour: undefined };
  const disambiguation: ZonedDisambiguation = "reject";
  const resolution: ZonedDisambiguationOptions = { disambiguation };
  const epoch: number = fromZonedDateTime(civil, undefined, resolution);
  const zoned: ZonedDateTimeParts = toZonedDateTime(epoch, zone);
  const hour: number = zoned.hour;
  const duration: ZonedDurationInput = { days: 1, minutes: 15 };
  const add: AddZonedDurationOptions = { timeZone: zone, disambiguation: undefined };
  const later: number = addZonedDuration(epoch, duration, add);
  const formatting: FormatDateTimeOptions = { locale, hourCycle: "h23", dateStyle: undefined };
  const formatted: string = formatDateTime(later, formatting);
  const calendar: CalendarLabelsOptions = { locale: undefined, width: "narrow" };
  const labels: CalendarLabels = getCalendarLabels(calendar);
  const collation: CollatorOptions = { locale, numeric: undefined, sensitivity: "accent" };
  const compare: number = compareStrings("boot-1", "boot-9", collation);
  const order: Uint32Array = collatorSortIndices(["boot-9", "boot-1"] as const, collation);
  void [valid, hour, formatted, compare, order];

  // @ts-expect-error Returned calendar labels are readonly.
  labels.months.push("extra");
  // @ts-expect-error Civil fields require numbers.
  fromZonedDateTime({ ...civil, day: "8" });
  // @ts-expect-error Only the documented disambiguation policies are supported.
  addZonedDuration(epoch, duration, { disambiguation: "first" });
  // @ts-expect-error Batches accept strings, not arbitrary values.
  collatorSortIndices([99]);
}

export function webGlobalTypes(): void {
  const url = new URL("/x?key=value", new URL("https://example.test"));
  const params = new URLSearchParams(url.searchParams);
  params.delete("key", "value");
  params.append("q", "中文");
  for (const [key, value] of params) url.searchParams.set(key, value);
  const maybe: URL | null = URL.parse(url.href);
  const valid: boolean = URL.canParse(url);
  const encoder = new TextEncoder();
  const bytes: Uint8Array<ArrayBuffer> = encoder.encode(params.toString());
  const encoded: TextEncoderEncodeIntoResult = encoder.encodeInto("中文", new Uint8Array(6));
  const options: TextDecoderOptions = { fatal: undefined, ignoreBOM: false };
  const stream: TextDecodeOptions = { stream: true };
  const decoder = new TextDecoder("utf8", options);
  const part: string = decoder.decode(new DataView(bytes.buffer), stream);
  const rest: string = decoder.decode();
  void [maybe, valid, encoded, part, rest];

  // @ts-expect-error Encoding into a clamped destination is not supported.
  encoder.encodeInto("x", new Uint8ClampedArray(1));
  // @ts-expect-error Decoding expects a BufferSource, not a plain array.
  decoder.decode([65]);
}
