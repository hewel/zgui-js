//! Synchronous locale and civil-time calls scoped to an active Work generation.

use std::{
    fmt::Display,
    rc::{Rc, Weak},
};

use host_services::locale::{
    CalendarLabelWidth, CollationSensitivity, DateTimeStyle, FormatDateTimeOptions, LocaleCollator,
    ZonedDuration, zone_name,
};
use icu_locale_core::preferences::extensions::unicode::keywords::HourCycle;
use jiff::{ZonedDateTime, civil::DateTime, tz::Disambiguation};
use rquickjs::{
    Array, Ctx, Exception, FromJs, Object, TypedArray, Value,
    function::{Func, Opt},
};

use super::{Work, data_property, owner};

const MAX_COLLATION_VALUES: u32 = 100_000;
const MAX_COLLATION_BYTES: usize = 8 * 1024 * 1024;

impl Work {
    pub(super) fn install_locale<'js>(
        self: &Rc<Self>,
        services: &Object<'js>,
    ) -> rquickjs::Result<()> {
        let weak = Rc::downgrade(self);
        services.prop(
            "getSystemTimeZone",
            data_property(Func::from(move |ctx: Ctx<'js>| {
                Ok::<_, rquickjs::Error>(active(&weak, &ctx)?.locale.system_time_zone().to_owned())
            })),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "getSystemLocale",
            data_property(Func::from(move |ctx: Ctx<'js>| {
                Ok::<_, rquickjs::Error>(active(&weak, &ctx)?.locale.system_locale())
            })),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "isValidTimeZone",
            data_property(Func::from(move |ctx: Ctx<'js>, time_zone: String| {
                Ok::<_, rquickjs::Error>(active(&weak, &ctx)?.locale.is_valid_time_zone(&time_zone))
            })),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "toZonedDateTime",
            data_property(Func::from(
                move |ctx: Ctx<'js>, epoch: Value<'js>, time_zone: Opt<Option<String>>| {
                    let work = active(&weak, &ctx)?;
                    let zoned = work
                        .locale
                        .to_zoned(
                            integer(&ctx, &epoch, "epochMs")?,
                            time_zone.0.flatten().as_deref(),
                        )
                        .map_err(|error| range_error(&ctx, &error))?;
                    zoned_parts(&ctx, &zoned)
                },
            )),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "fromZonedDateTime",
            data_property(Func::from(
                move |ctx: Ctx<'js>,
                      parts: Object<'js>,
                      time_zone: Opt<Option<String>>,
                      options: Opt<Option<Object<'js>>>| {
                    let work = active(&weak, &ctx)?;
                    let options = options.0.flatten();
                    let civil = civil_datetime(&ctx, &parts)?;
                    let epoch = work
                        .locale
                        .from_zoned(
                            civil,
                            time_zone.0.flatten().as_deref(),
                            disambiguation(&ctx, options.as_ref())?,
                        )
                        .map_err(|error| range_error(&ctx, &error))?;
                    Ok::<_, rquickjs::Error>(epoch as f64)
                },
            )),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "addZonedDuration",
            data_property(Func::from(
                move |ctx: Ctx<'js>,
                      epoch: Value<'js>,
                      duration: Object<'js>,
                      options: Opt<Option<Object<'js>>>| {
                    let work = active(&weak, &ctx)?;
                    let epoch = integer(&ctx, &epoch, "epochMs")?;
                    let duration = ZonedDuration {
                        years: integer_field(&ctx, &duration, "years", Some(0))?,
                        months: integer_field(&ctx, &duration, "months", Some(0))?,
                        weeks: integer_field(&ctx, &duration, "weeks", Some(0))?,
                        days: integer_field(&ctx, &duration, "days", Some(0))?,
                        hours: integer_field(&ctx, &duration, "hours", Some(0))?,
                        minutes: integer_field(&ctx, &duration, "minutes", Some(0))?,
                        seconds: integer_field(&ctx, &duration, "seconds", Some(0))?,
                        milliseconds: integer_field(&ctx, &duration, "milliseconds", Some(0))?,
                    };
                    let options = options.0.flatten();
                    let time_zone: Option<String> = option(options.as_ref(), "timeZone")?;
                    let epoch = work
                        .locale
                        .add_zoned_duration(
                            epoch,
                            duration,
                            time_zone.as_deref(),
                            disambiguation(&ctx, options.as_ref())?,
                        )
                        .map_err(|error| range_error(&ctx, &error))?;
                    Ok::<_, rquickjs::Error>(epoch as f64)
                },
            )),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "formatDateTime",
            data_property(Func::from(
                move |ctx: Ctx<'js>, epoch: Value<'js>, options: Opt<Option<Object<'js>>>| {
                    let work = active(&weak, &ctx)?;
                    let options = options.0.flatten();
                    let options = options.as_ref();
                    let locale: Option<String> = option(options, "locale")?;
                    let time_zone: Option<String> = option(options, "timeZone")?;
                    let hour_cycle: Option<String> = option(options, "hourCycle")?;
                    let hour_cycle = match hour_cycle.as_deref() {
                        None => None,
                        Some("h12") => Some(HourCycle::H12),
                        Some("h23") => Some(HourCycle::H23),
                        _ => return Err(Exception::throw_range(&ctx, "invalid hourCycle")),
                    };
                    let options = FormatDateTimeOptions {
                        locale: locale.as_deref(),
                        time_zone: time_zone.as_deref(),
                        date_style: style(&ctx, options, "dateStyle")?,
                        time_style: style(&ctx, options, "timeStyle")?,
                        hour_cycle,
                    };
                    work.locale
                        .format_date_time(integer(&ctx, &epoch, "epochMs")?, &options)
                        .map_err(|error| range_error(&ctx, &error))
                },
            )),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "getCalendarLabels",
            data_property(Func::from(
                move |ctx: Ctx<'js>, options: Opt<Option<Object<'js>>>| {
                    let work = active(&weak, &ctx)?;
                    let options = options.0.flatten();
                    let options = options.as_ref();
                    let locale: Option<String> = option(options, "locale")?;
                    let width: Option<String> = option(options, "width")?;
                    let width = match width.as_deref() {
                        None | Some("long") => CalendarLabelWidth::Long,
                        Some("short") => CalendarLabelWidth::Short,
                        Some("narrow") => CalendarLabelWidth::Narrow,
                        _ => {
                            return Err(Exception::throw_range(
                                &ctx,
                                "invalid calendar label width",
                            ));
                        }
                    };
                    let labels = work
                        .locale
                        .calendar_labels(locale.as_deref(), width)
                        .map_err(|error| range_error(&ctx, &error))?;
                    let result = Object::new(ctx)?;
                    result.prop("locale", data_property(labels.locale))?;
                    result.prop("months", data_property(labels.months))?;
                    result.prop("weekdays", data_property(labels.weekdays))?;
                    result.prop("firstDayOfWeek", data_property(labels.first_day_of_week))?;
                    Ok::<_, rquickjs::Error>(result)
                },
            )),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "compareStrings",
            data_property(Func::from(
                move |ctx: Ctx<'js>,
                      left: String,
                      right: String,
                      options: Opt<Option<Object<'js>>>| {
                    let work = active(&weak, &ctx)?;
                    let collator = collator(&work, &ctx, options.0.flatten().as_ref())?;
                    Ok::<_, rquickjs::Error>(collator.compare(&left, &right) as i32)
                },
            )),
        )?;
        let weak = Rc::downgrade(self);
        services.prop(
            "collatorSortIndices",
            data_property(Func::from(
                move |ctx: Ctx<'js>, values: Array<'js>, options: Opt<Option<Object<'js>>>| {
                    let work = active(&weak, &ctx)?;
                    let collator = collator(&work, &ctx, options.0.flatten().as_ref())?;
                    let count: u32 = values.as_object().get("length")?;
                    if count > MAX_COLLATION_VALUES {
                        return Err(Exception::throw_range(
                            &ctx,
                            "collation batch exceeds 100,000 strings",
                        ));
                    }
                    let mut bytes = 0;
                    let indices = collator.sort_indices(count, |index| {
                        let value: String = values.get(index as usize)?;
                        bytes += value.len();
                        if bytes > MAX_COLLATION_BYTES {
                            return Err(Exception::throw_range(
                                &ctx,
                                "collation batch exceeds 8 MiB of UTF-8",
                            ));
                        }
                        Ok(value)
                    })?;
                    TypedArray::new(ctx, indices)
                },
            )),
        )?;
        Ok(())
    }
}

fn active(weak: &Weak<Work>, ctx: &Ctx<'_>) -> rquickjs::Result<Rc<Work>> {
    let work = owner(weak, ctx)?;
    work.require_creation(ctx)?;
    Ok(work)
}

fn range_error(ctx: &Ctx<'_>, error: &impl Display) -> rquickjs::Error {
    Exception::throw_range(ctx, &error.to_string())
}

fn option<'js, T: FromJs<'js>>(
    options: Option<&Object<'js>>,
    key: &str,
) -> rquickjs::Result<Option<T>> {
    options.map_or(Ok(None), |object| object.get(key))
}

fn style(
    ctx: &Ctx<'_>,
    options: Option<&Object<'_>>,
    name: &str,
) -> rquickjs::Result<Option<DateTimeStyle>> {
    let value: Option<String> = option(options, name)?;
    match value.as_deref() {
        None => Ok(None),
        Some("full") => Ok(Some(DateTimeStyle::Full)),
        Some("long") => Ok(Some(DateTimeStyle::Long)),
        Some("medium") => Ok(Some(DateTimeStyle::Medium)),
        Some("short") => Ok(Some(DateTimeStyle::Short)),
        _ => Err(Exception::throw_range(ctx, &format!("invalid {name}"))),
    }
}

fn collator(
    work: &Work,
    ctx: &Ctx<'_>,
    options: Option<&Object<'_>>,
) -> rquickjs::Result<LocaleCollator> {
    let locale: Option<String> = option(options, "locale")?;
    let numeric = option(options, "numeric")?;
    let sensitivity: Option<String> = option(options, "sensitivity")?;
    let sensitivity = match sensitivity.as_deref() {
        None | Some("variant") => CollationSensitivity::Variant,
        Some("base") => CollationSensitivity::Base,
        Some("accent") => CollationSensitivity::Accent,
        Some("case") => CollationSensitivity::Case,
        _ => return Err(Exception::throw_range(ctx, "invalid collator sensitivity")),
    };
    work.locale
        .collator(locale.as_deref(), numeric, sensitivity)
        .map_err(|error| range_error(ctx, &error))
}

fn disambiguation(ctx: &Ctx<'_>, options: Option<&Object<'_>>) -> rquickjs::Result<Disambiguation> {
    let value: Option<String> = option(options, "disambiguation")?;
    match value.as_deref() {
        None | Some("compatible") => Ok(Disambiguation::Compatible),
        Some("earlier") => Ok(Disambiguation::Earlier),
        Some("later") => Ok(Disambiguation::Later),
        Some("reject") => Ok(Disambiguation::Reject),
        _ => Err(Exception::throw_range(ctx, "invalid disambiguation")),
    }
}

fn integer(ctx: &Ctx<'_>, value: &Value<'_>, name: &str) -> rquickjs::Result<i64> {
    const MAX_SAFE: f64 = crate::assets::MAX_SAFE_ID as f64;
    let value = value
        .as_number()
        .filter(|value| value.fract() == 0.0 && value.abs() <= MAX_SAFE)
        .ok_or_else(|| Exception::throw_range(ctx, &format!("{name} must be a safe integer")))?;
    Ok(value as i64)
}

fn integer_field<T: TryFrom<i64>>(
    ctx: &Ctx<'_>,
    input: &Object<'_>,
    name: &str,
    default: Option<i64>,
) -> rquickjs::Result<T> {
    let value: Value<'_> = input.get(name)?;
    let value = match default {
        Some(value_if_missing) if value.is_undefined() => value_if_missing,
        _ => integer(ctx, &value, name)?,
    };
    T::try_from(value).map_err(|_| Exception::throw_range(ctx, &format!("{name} is out of range")))
}

fn civil_datetime(ctx: &Ctx<'_>, parts: &Object<'_>) -> rquickjs::Result<DateTime> {
    let millisecond: i16 = integer_field(ctx, parts, "millisecond", Some(0))?;
    if !(0..=999).contains(&millisecond) {
        return Err(Exception::throw_range(ctx, "millisecond is out of range"));
    }
    DateTime::new(
        integer_field(ctx, parts, "year", None)?,
        integer_field(ctx, parts, "month", None)?,
        integer_field(ctx, parts, "day", None)?,
        integer_field(ctx, parts, "hour", Some(0))?,
        integer_field(ctx, parts, "minute", Some(0))?,
        integer_field(ctx, parts, "second", Some(0))?,
        i32::from(millisecond) * 1_000_000,
    )
    .map_err(|error| range_error(ctx, &error))
}

fn zoned_parts<'js>(ctx: &Ctx<'js>, zoned: &ZonedDateTime) -> rquickjs::Result<Object<'js>> {
    let result = Object::new(ctx.clone())?;
    let zone = zoned.time_zone();
    let offset = zone.to_offset_info(zoned.timestamp());
    result.prop("year", data_property(zoned.year()))?;
    result.prop("month", data_property(zoned.month()))?;
    result.prop("day", data_property(zoned.day()))?;
    result.prop("hour", data_property(zoned.hour()))?;
    result.prop("minute", data_property(zoned.minute()))?;
    result.prop("second", data_property(zoned.second()))?;
    result.prop("millisecond", data_property(zoned.millisecond()))?;
    result.prop(
        "dayOfWeek",
        data_property(zoned.weekday().to_monday_one_offset()),
    )?;
    result.prop("offsetSeconds", data_property(offset.offset().seconds()))?;
    result.prop("inDst", data_property(offset.dst().is_dst()))?;
    result.prop("timeZone", data_property(zone_name(zone)))?;
    result.prop("zoneAbbreviation", data_property(offset.abbreviation()))?;
    result.prop("daysInMonth", data_property(zoned.days_in_month()))?;
    Ok(result)
}
