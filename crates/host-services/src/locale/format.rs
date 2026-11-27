use std::borrow::Cow;

use icu_calendar::{Date, Gregorian, Iso, week::WeekInformation};
use icu_datetime::{
    DateTimeFormatter, DateTimeFormatterPreferences,
    fieldsets::{
        T, YMD, YMDE,
        enums::{CompositeFieldSet, DateAndTimeFieldSet, DateFieldSet, TimeFieldSet, ZoneFieldSet},
        zone,
    },
    options::{Length, TimePrecision},
    pattern::{DateTimePattern, FixedCalendarDateTimeNames, MonthNameLength, WeekdayNameLength},
};
use icu_locale_core::{Locale, preferences::extensions::unicode::keywords::HourCycle};
use icu_time::{TimeZoneInfo, ZonedDateTime, zone::models::AtTime};
use jiff_icu::ConvertFrom;
use writeable::TryWriteable;

use super::{LocaleContext, LocaleError, parse_locale};

/// Requested date or time detail.
#[derive(Clone, Copy, Debug)]
pub enum DateTimeStyle {
    Full,
    Long,
    Medium,
    Short,
}

impl DateTimeStyle {
    fn length(self) -> Length {
        match self {
            Self::Full | Self::Long => Length::Long,
            Self::Medium => Length::Medium,
            Self::Short => Length::Short,
        }
    }

    fn precision(self) -> TimePrecision {
        match self {
            Self::Short => TimePrecision::Minute,
            _ => TimePrecision::Second,
        }
    }

    fn zone(self) -> Option<ZoneFieldSet> {
        match self {
            Self::Full => Some(ZoneFieldSet::SpecificLong(zone::SpecificLong)),
            Self::Long => Some(ZoneFieldSet::SpecificShort(zone::SpecificShort)),
            _ => None,
        }
    }
}

/// Per-call date/time formatting overrides. Without styles, both default to medium.
#[derive(Debug, Default)]
pub struct FormatDateTimeOptions<'a> {
    pub locale: Option<&'a str>,
    pub time_zone: Option<&'a str>,
    pub date_style: Option<DateTimeStyle>,
    pub time_style: Option<DateTimeStyle>,
    pub hour_cycle: Option<HourCycle>,
}

/// Width for standalone month and weekday labels.
#[derive(Clone, Copy, Debug, Default)]
pub enum CalendarLabelWidth {
    #[default]
    Long,
    Short,
    Narrow,
}

/// Gregorian months (Jan–Dec), ISO weekdays (Mon–Sun), and regional week start.
#[derive(Debug)]
pub struct CalendarLabels {
    pub locale: String,
    pub months: Vec<String>,
    pub weekdays: Vec<String>,
    /// ISO weekday number, Monday = 1.
    pub first_day_of_week: i8,
}

impl LocaleContext {
    pub(super) fn resolve_locale(
        &self,
        locale: Option<&str>,
    ) -> Result<Cow<'_, Locale>, LocaleError> {
        locale.map_or_else(
            || Ok(Cow::Borrowed(&self.locale)),
            |tag| parse_locale(tag).map(Cow::Owned),
        )
    }

    /// Format an epoch instant with CLDR patterns in the requested locale and zone.
    ///
    /// # Errors
    /// Rejects invalid locale or zone overrides, out-of-range instants and ICU failures.
    pub fn format_date_time(
        &self,
        epoch_ms: i64,
        options: &FormatDateTimeOptions<'_>,
    ) -> Result<String, LocaleError> {
        let locale = self.resolve_locale(options.locale)?;
        let mut prefs: DateTimeFormatterPreferences = locale.as_ref().into();
        if let Some(hour_cycle) = options.hour_cycle {
            prefs.hour_cycle = Some(hour_cycle);
        }
        let zoned = self.to_zoned(epoch_ms, options.time_zone)?;
        let input = ZonedDateTime::<Iso, TimeZoneInfo<AtTime>>::convert_from(&zoned);
        let fields = match (options.date_style, options.time_style) {
            (None, None) => date_and_time(DateTimeStyle::Medium, DateTimeStyle::Medium),
            (Some(date), Some(time)) => date_and_time(date, time),
            (Some(DateTimeStyle::Full), None) => {
                CompositeFieldSet::Date(DateFieldSet::YMDE(YMDE::long()))
            }
            (Some(date), None) => {
                CompositeFieldSet::Date(DateFieldSet::YMD(YMD::for_length(date.length())))
            }
            (None, Some(time)) => {
                let field = TimeFieldSet::T(
                    T::for_length(time.length()).with_time_precision(time.precision()),
                );
                match time.zone() {
                    Some(zone) => CompositeFieldSet::TimeZone(field.with_zone(zone)),
                    None => CompositeFieldSet::Time(field),
                }
            }
        };
        let formatter = DateTimeFormatter::try_new(prefs, fields).map_err(LocaleError::icu)?;
        Ok(formatter.format(&input).to_string())
    }

    /// Return standalone CLDR names and the locale's first weekday.
    ///
    /// # Errors
    /// Rejects malformed locale tags and ICU data or formatting errors.
    pub fn calendar_labels(
        &self,
        locale: Option<&str>,
        width: CalendarLabelWidth,
    ) -> Result<CalendarLabels, LocaleError> {
        let locale = self.resolve_locale(locale)?;
        let first_day_of_week = WeekInformation::try_new(locale.as_ref().into())
            .map_err(LocaleError::icu)?
            .first_weekday as i8;
        let (month, weekday, month_pattern, weekday_pattern) = match width {
            CalendarLabelWidth::Long => (
                MonthNameLength::StandaloneWide,
                WeekdayNameLength::StandaloneWide,
                "LLLL",
                "cccc",
            ),
            CalendarLabelWidth::Short => (
                MonthNameLength::StandaloneAbbreviated,
                WeekdayNameLength::StandaloneAbbreviated,
                "LLL",
                "ccc",
            ),
            CalendarLabelWidth::Narrow => (
                MonthNameLength::StandaloneNarrow,
                WeekdayNameLength::StandaloneNarrow,
                "LLLLL",
                "ccccc",
            ),
        };
        let mut names =
            FixedCalendarDateTimeNames::<Gregorian, DateFieldSet>::try_new(locale.as_ref().into())
                .map_err(LocaleError::icu)?;
        names.include_month_names(month).map_err(LocaleError::icu)?;
        names
            .include_weekday_names(weekday)
            .map_err(LocaleError::icu)?;
        let month_pattern =
            DateTimePattern::try_from_pattern_str(month_pattern).map_err(LocaleError::icu)?;
        let weekday_pattern =
            DateTimePattern::try_from_pattern_str(weekday_pattern).map_err(LocaleError::icu)?;
        let month_formatter = names.with_pattern_unchecked(&month_pattern);
        let weekday_formatter = names.with_pattern_unchecked(&weekday_pattern);
        let months = (1..=12)
            .map(|month| {
                let date = Date::try_new_gregorian(2024, month, 1).map_err(LocaleError::icu)?;
                month_formatter
                    .format(&date)
                    .try_write_to_string()
                    .map(Cow::into_owned)
                    .map_err(|(error, _)| LocaleError::IcuPattern(error))
            })
            .collect::<Result<_, _>>()?;
        // 2024-01-01 was a Monday. Keep labels in ISO order independently of the locale week start.
        let weekdays = (1..=7)
            .map(|day| {
                let date = Date::try_new_gregorian(2024, 1, day).map_err(LocaleError::icu)?;
                weekday_formatter
                    .format(&date)
                    .try_write_to_string()
                    .map(Cow::into_owned)
                    .map_err(|(error, _)| LocaleError::IcuPattern(error))
            })
            .collect::<Result<_, _>>()?;
        Ok(CalendarLabels {
            locale: locale.to_string(),
            months,
            weekdays,
            first_day_of_week,
        })
    }
}

fn date_and_time(date: DateTimeStyle, time: DateTimeStyle) -> CompositeFieldSet {
    let precision = time.precision();
    let fields = match date {
        DateTimeStyle::Full => DateAndTimeFieldSet::YMDET(YMDE::long().with_time(precision)),
        _ => DateAndTimeFieldSet::YMDT(YMD::for_length(date.length()).with_time(precision)),
    };
    match time.zone() {
        Some(zone) => CompositeFieldSet::DateTimeZone(fields.with_zone(zone)),
        None => CompositeFieldSet::DateTime(fields),
    }
}
