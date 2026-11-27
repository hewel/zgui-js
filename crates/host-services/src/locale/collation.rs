use std::cmp::Ordering;

use icu_collator::{
    CollatorBorrowed, CollatorPreferences,
    options::{CaseLevel, CollatorOptions, Strength},
    preferences::CollationNumericOrdering,
};

use super::{LocaleContext, LocaleError};

/// Case and accent strength, following ECMA-402 sensitivity semantics.
#[derive(Clone, Copy, Debug, Default)]
pub enum CollationSensitivity {
    Base,
    Accent,
    Case,
    #[default]
    Variant,
}

/// A single ICU collator shared by all comparisons in one operation.
#[derive(Debug)]
pub struct LocaleCollator(CollatorBorrowed<'static>);

impl LocaleContext {
    /// Configure a collator using the requested locale or the session default.
    ///
    /// # Errors
    /// Rejects malformed locale tags and ICU data errors.
    pub fn collator(
        &self,
        locale: Option<&str>,
        numeric: Option<bool>,
        sensitivity: CollationSensitivity,
    ) -> Result<LocaleCollator, LocaleError> {
        let locale = self.resolve_locale(locale)?;
        let mut prefs: CollatorPreferences = locale.as_ref().into();
        prefs.numeric_ordering = Some(match numeric.unwrap_or(true) {
            true => CollationNumericOrdering::True,
            false => CollationNumericOrdering::False,
        });
        let mut options = CollatorOptions::default();
        options.strength = Some(match sensitivity {
            CollationSensitivity::Base | CollationSensitivity::Case => Strength::Primary,
            CollationSensitivity::Accent => Strength::Secondary,
            CollationSensitivity::Variant => Strength::Tertiary,
        });
        options.case_level = Some(match sensitivity {
            CollationSensitivity::Case => CaseLevel::On,
            _ => CaseLevel::Off,
        });
        CollatorBorrowed::try_new(prefs, options)
            .map(LocaleCollator)
            .map_err(LocaleError::icu)
    }
}

impl LocaleCollator {
    /// Compare two strings without allocating sort keys.
    pub fn compare(&self, left: &str, right: &str) -> Ordering {
        self.0.compare(left, right)
    }

    /// Read each input once, then sort its original index by ICU key and input order.
    ///
    /// # Errors
    /// Propagates the first error from the input reader.
    pub fn sort_indices<T: AsRef<str>, E>(
        &self,
        count: u32,
        mut read: impl FnMut(u32) -> Result<T, E>,
    ) -> Result<Vec<u32>, E> {
        // One packed arena instead of a heap allocation per key. Original strings
        // need not survive marshaling and sorting never invokes the reader.
        let mut keys = Vec::new();
        let mut boundaries = Vec::with_capacity(count as usize + 1);
        boundaries.push(0);
        for index in 0..count {
            let value = read(index)?;
            let Ok(()) = self.0.write_sort_key_to(value.as_ref(), &mut keys);
            boundaries.push(keys.len());
        }
        let key = |index: u32| {
            let i = index as usize;
            &keys[boundaries[i]..boundaries[i + 1]]
        };
        let mut indices: Vec<_> = (0..count).collect();
        indices.sort_unstable_by(|&left, &right| {
            key(left).cmp(key(right)).then_with(|| left.cmp(&right))
        });
        Ok(indices)
    }
}
