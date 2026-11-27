//! Session locale and IANA timezone defaults, with civil time arithmetic.

use icu_locale::LocaleCanonicalizer;
use icu_locale_core::{Locale, locale};
use jiff::{
    Span, Timestamp, ZonedDateTime,
    civil::DateTime,
    tz::{self, Disambiguation, TimeZone, TimeZoneDatabase},
};

mod collation;
mod format;

pub use collation::{CollationSensitivity, LocaleCollator};
pub use format::{CalendarLabelWidth, CalendarLabels, DateTimeStyle, FormatDateTimeOptions};

/// Locale and timezone snapshot for one session, including its zoneinfo source.
#[derive(Clone, Debug)]
pub struct LocaleContext {
    locale: Locale,
    time_zone: TimeZone,
    database: TimeZoneDatabase,
}

/// Invalid locale, zone, or arithmetic input.
#[derive(Debug)]
pub enum LocaleError {
    InvalidLocale,
    InvalidTimeZone,
    MixedDurationSigns,
    Time(jiff::Error),
    Icu(Box<dyn std::error::Error + Send + Sync>),
    IcuPattern(icu_datetime::pattern::FormattedDateTimePatternError),
}

impl std::fmt::Display for LocaleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidLocale => f.write_str("invalid BCP-47 locale"),
            Self::InvalidTimeZone => f.write_str("unknown IANA time zone"),
            Self::MixedDurationSigns => f.write_str("duration fields must have the same sign"),
            Self::Time(error) => error.fmt(f),
            Self::Icu(error) => error.fmt(f),
            Self::IcuPattern(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for LocaleError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Time(error) => Some(error),
            Self::Icu(error) => Some(error.as_ref()),
            _ => None,
        }
    }
}

impl From<jiff::Error> for LocaleError {
    fn from(error: jiff::Error) -> Self {
        Self::Time(error)
    }
}

impl LocaleError {
    fn icu(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Icu(Box::new(error))
    }
}

fn parse_locale(tag: &str) -> Result<Locale, LocaleError> {
    if tag.contains('_') {
        return Err(LocaleError::InvalidLocale);
    }
    let mut locale = Locale::try_from_str(tag).map_err(|_| LocaleError::InvalidLocale)?;
    LocaleCanonicalizer::new_common().canonicalize(&mut locale);
    Ok(locale)
}

fn named_zone(database: &TimeZoneDatabase, name: &str) -> Result<TimeZone, LocaleError> {
    let zone = database
        .get(name)
        .map_err(|_| LocaleError::InvalidTimeZone)?;
    if zone.iana_name().is_none() {
        return Err(LocaleError::InvalidTimeZone);
    }
    Ok(zone)
}

impl LocaleContext {
    /// Read live desktop defaults. Unavailable or unnamed zones fall back to UTC;
    /// the POSIX C locale and unavailable/malformed locale variables use en-US.
    pub fn live() -> Self {
        let locale = ["LC_ALL", "LC_TIME", "LANG"]
            .into_iter()
            .filter_map(|key| std::env::var(key).ok())
            .find(|value| !value.is_empty())
            .and_then(|value| {
                let tag = value.split(['.', '@']).next()?.replace('_', "-");
                if matches!(tag.as_str(), "C" | "POSIX") {
                    return None;
                }
                parse_locale(&tag).ok()
            })
            .unwrap_or(locale!("en-US"));
        let time_zone = TimeZone::try_system()
            .ok()
            .filter(|zone| zone.iana_name().is_some())
            .unwrap_or(TimeZone::UTC);
        Self {
            locale,
            time_zone,
            database: tz::db().clone(),
        }
    }

    /// UTC / en-US for a preview with no overrides; never reads the environment.
    pub fn preview_default() -> Self {
        Self {
            locale: locale!("en-US"),
            time_zone: TimeZone::UTC,
            database: TimeZoneDatabase::bundled(),
        }
    }

    /// Resolve deterministic preview defaults using bundled IANA data, never TZDIR.
    ///
    /// # Errors
    /// Rejects malformed locale tags and unknown IANA time zones.
    pub fn fixture(locale: Option<&str>, time_zone: Option<&str>) -> Result<Self, LocaleError> {
        let mut context = Self::preview_default();
        if let Some(tag) = locale {
            context.locale = parse_locale(tag)?;
        }
        if let Some(name) = time_zone {
            context.time_zone = named_zone(&context.database, name)?;
        }
        Ok(context)
    }

    /// Return the canonical BCP-47 locale of this session.
    pub fn system_locale(&self) -> String {
        self.locale.to_string()
    }

    /// Return the IANA time zone name of this session.
    pub fn system_time_zone(&self) -> &str {
        zone_name(&self.time_zone)
    }

    /// Check whether a name denotes an IANA time zone in this session's database.
    pub fn is_valid_time_zone(&self, name: &str) -> bool {
        named_zone(&self.database, name).is_ok()
    }

    fn zone(&self, name: Option<&str>) -> Result<TimeZone, LocaleError> {
        name.map_or_else(
            || Ok(self.time_zone.clone()),
            |name| named_zone(&self.database, name),
        )
    }

    /// Convert an instant into the requested or session-default time zone.
    ///
    /// # Errors
    /// Rejects timestamps outside Jiff's supported range and unknown zones.
    pub fn to_zoned(
        &self,
        epoch_ms: i64,
        time_zone: Option<&str>,
    ) -> Result<ZonedDateTime, LocaleError> {
        Ok(Timestamp::from_millisecond(epoch_ms)?.to_zoned_datetime(self.zone(time_zone)?))
    }

    /// Resolve a civil date/time to an instant under the supplied DST policy.
    ///
    /// # Errors
    /// Rejects unknown zones, out-of-range results, and gaps/folds under `Reject`.
    pub fn from_zoned(
        &self,
        civil: DateTime,
        time_zone: Option<&str>,
        disambiguation: Disambiguation,
    ) -> Result<i64, LocaleError> {
        let zone = self.zone(time_zone)?;
        Ok(zone
            .to_ambiguous_timestamp(civil)
            .disambiguate(disambiguation)?
            .as_millisecond())
    }

    /// Apply calendar fields in local time, then clock fields as elapsed time.
    ///
    /// # Errors
    /// Rejects an unknown zone, out-of-range input/result, mixed duration signs,
    /// and a calendar result in a gap/fold under `Reject`.
    pub fn add_zoned_duration(
        &self,
        epoch_ms: i64,
        duration: ZonedDuration,
        time_zone: Option<&str>,
        disambiguation: Disambiguation,
    ) -> Result<i64, LocaleError> {
        let fields = [
            duration.years,
            duration.months,
            duration.weeks,
            duration.days,
            duration.hours,
            duration.minutes,
            duration.seconds,
            duration.milliseconds,
        ];
        if fields.iter().any(|&value| value < 0) && fields.iter().any(|&value| value > 0) {
            return Err(LocaleError::MixedDurationSigns);
        }
        let calendar = Span::new()
            .try_years(duration.years)?
            .try_months(duration.months)?
            .try_weeks(duration.weeks)?
            .try_days(duration.days)?;
        let clock = Span::new()
            .try_hours(duration.hours)?
            .try_minutes(duration.minutes)?
            .try_seconds(duration.seconds)?
            .try_milliseconds(duration.milliseconds)?;
        let zoned = self.to_zoned(epoch_ms, time_zone)?;
        // A time-only duration must not re-disambiguate an instant already in a fold.
        let timestamp = if calendar.is_zero() {
            zoned.timestamp()
        } else {
            let civil = zoned.datetime().checked_add(calendar)?;
            zoned
                .time_zone()
                .to_ambiguous_timestamp(civil)
                .disambiguate(disambiguation)?
        };
        Ok(timestamp.checked_add(clock)?.as_millisecond())
    }
}

/// IANA name for a known zone, with UTC as the fallback for an unnamed zone.
pub fn zone_name(zone: &TimeZone) -> &str {
    zone.iana_name().unwrap_or("UTC")
}

/// Whole calendar and clock units, all of the same sign.
#[derive(Clone, Copy, Debug, Default)]
pub struct ZonedDuration {
    pub years: i64,
    pub months: i64,
    pub weeks: i64,
    pub days: i64,
    pub hours: i64,
    pub minutes: i64,
    pub seconds: i64,
    pub milliseconds: i64,
}
