//! Alias leases prevent a retired session from withdrawing its successor's font.
use std::{collections::BTreeMap, sync::{Arc, Mutex}};
use zgui_text::{FontData, FontError, FontSource};
use super::Fonts;
#[derive(Default)]
pub(super) struct Registrations {
    next: u64,
    aliases: BTreeMap<String, Vec<Registration>>,
    unnamed: Vec<FontData>,
}
struct Registration {
    token: u64,
    data: Vec<FontData>,
    family: String,
}

/// A registered font alias, withdrawn or restored to its predecessor when dropped.
#[derive(Debug)]
pub struct FontLease {
    fonts: Fonts,
    alias: String,
    token: u64,
}


impl std::fmt::Debug for Registrations {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registrations").field("aliases", &self.aliases.len()).finish()
    }
}
impl Fonts {
    /// Registers verified bytes under a session-owned family alias.
    ///
    /// Dropping an older lease never changes a newer registration. Dropping the
    /// newest lease restores a still-live predecessor, including permanent fonts.
    ///
    /// # Errors
    /// Returns a font error for malformed bytes or an alias of an untracked system family.
    pub fn register_scoped(&self, data: FontData, family: &str) -> Result<FontLease, FontError> {
        let alias = family.to_lowercase();
        let mut state = self.registrations.lock().expect("font registrations lock poisoned");
        if !state.aliases.contains_key(&alias) && self.contains_family(family) {
            return Err(FontError::Malformed("scoped font alias conflicts with a system family"));
        }
        let token = state.next.checked_add(1).ok_or(FontError::Malformed("font lease identity exhausted"))?;
        let previous = state.aliases.get(&alias).and_then(|entries| entries.last());
        let data = if let Some(previous) = previous.filter(|entry| entry.data.len() == 1
            && entry.data[0].as_ref().as_ref() == data.as_ref().as_ref()) {
            previous.data[0].clone()
        } else {
            self.replace_alias(family, std::slice::from_ref(&data), previous)?;
            data
        };
        state.next = token;
        state.aliases.entry(alias.clone()).or_default().push(Registration { token, data: vec![data], family: family.into() });
        Ok(FontLease { fonts: self.clone(), alias, token })
    }

    /// Whether the collection contains faces reachable by this exact family alias.
    pub fn contains_family(&self, family: &str) -> bool {
        self.system.get().contains_family(family)
    }

    /// Snapshot of whether system fonts are enabled plus all currently active registered font data
    /// (unnamed, permanent aliases, and active scoped leases) for SVG `<text>` rendering.
    pub fn svg_font_snapshot(&self) -> (bool, Vec<FontData>) {
        let state = self.registrations.lock().expect("font registrations lock poisoned");
        let mut faces = state.unnamed.clone();
        for entries in state.aliases.values() {
            if let Some(active) = entries.last() {
                faces.extend(active.data.iter().cloned());
            }
        }
        (self.uses_system_fonts, faces)
    }

    pub(super) fn record_unnamed(&self, data: FontData) {
        let mut state = self.registrations.lock().expect("font registrations lock poisoned");
        state.unnamed.push(data);
    }

    pub(super) fn register_permanent(&self, data: FontData, family: &str) -> Result<(), FontError> {
        let mut state = self.registrations.lock().expect("font registrations lock poisoned");
        let alias = family.to_lowercase();
        let entries = state.aliases.entry(alias).or_default();
        if entries.last().is_none_or(|entry| entry.token == 0) {
            self.system.get().register(data.clone(), Some(zgui_view::Ident::new(family)))?;
            if let Some(previous) = entries.last_mut() {
                previous.data.push(data);
            } else {
                entries.push(Registration { token: 0, data: vec![data], family: family.into() });
            }
        } else {
            self.replace_alias(family, std::slice::from_ref(&data), entries.last())?;
            entries.push(Registration { token: 0, data: vec![data], family: family.into() });
        }
        Ok(())
    }

    fn replace_alias(&self, family: &str, data: &[FontData], previous: Option<&Registration>) -> Result<(), FontError> {
        let system = self.system.get();
        system.unregister(zgui_view::Ident::new(family));
        for bytes in data {
            if let Err(error) = system.register(bytes.clone(), Some(zgui_view::Ident::new(family))) {
                system.unregister(zgui_view::Ident::new(family));
                if let Some(previous) = previous {
                    for bytes in &previous.data {
                        let _ = system.register(bytes.clone(), Some(zgui_view::Ident::new(&previous.family)));
                    }
                }
                return Err(error);
            }
        }
        Ok(())
    }
}

impl Drop for FontLease {
    fn drop(&mut self) {
        let Ok(mut state) = self.fonts.registrations.lock() else { return; };
        let Some(entries) = state.aliases.get_mut(&self.alias) else { return; };
        let Some(index) = entries.iter().position(|entry| entry.token == self.token) else { return; };
        let active = index + 1 == entries.len();
        let removed = entries.remove(index);
        let same_predecessor = entries.last().is_some_and(|previous| previous.data.len() == removed.data.len()
            && previous.data.iter().zip(&removed.data).all(|(left, right)| Arc::ptr_eq(left, right)));
        if active && !same_predecessor {
            let system = self.fonts.system.get();
            system.unregister(zgui_view::Ident::new(&removed.family));
            if let Some(previous) = entries.last() {
                for bytes in &previous.data {
                    let _ = system.register(bytes.clone(), Some(zgui_view::Ident::new(&previous.family)));
                }
            }
        }
        if entries.is_empty() { state.aliases.remove(&self.alias); }
    }
}

pub(super) type SharedRegistrations = Arc<Mutex<Registrations>>;
