//! Native property vocabulary. Null and undefined both clear a property's value.
//! Field/editor `value` loads through the native host, with empty values clearing
//! to empty text. `defaultValue` initializes only once and is ignored after any
//! explicit value assignment; only initialization metadata is retained here.
//!
//! Style objects accept only the kebab-case properties in `STYLE_PROPERTIES`, with
//! string values (including explicit units), or null/undefined to remove a key.
//! CSS strings use zgui's error-recovering CSS parser, not a browser CSS contract.
//! Custom `--name` declarations share the native inline block: replacing/clearing
//! style text also clears custom declarations, including directly assigned ones.

use std::collections::{HashMap, HashSet};
use rquickjs::{Ctx, Exception, FromJs, Value};
use zgui::{view::{AttrName, ClassName, CustomPropertyName, DomHandle, NodeId, Semantics, UiState}, vocab::{Role, SemanticFlags}};

#[derive(Clone, PartialEq)]
pub(crate) enum Style { Empty, Text(String), Object(Vec<(String, Option<String>)>) }

pub(crate) enum Mutation {
    Class(Option<String>),
    TestId(Option<String>),
    Style(Style),
    Attribute(&'static str, Option<String>),
    Disabled(bool),
    ReadOnly(bool),
    Value(String),
    DefaultValue(String),
    Role(Role),
    Label(Option<String>),
    Description(Option<String>),
    Custom(String, Option<String>),
}

impl Mutation {
    // Parse all JS properties/getters before borrowing any native property state.
    pub(crate) fn parse<'js>(ctx: &Ctx<'js>, name: &str, value: Value<'js>) -> rquickjs::Result<Self> {
        let empty = value.is_null() || value.is_undefined();
        let string = |value| Option::<String>::from_js(ctx, value);
        match name {
            "class" => Ok(Self::Class(string(value)?)),
            "testId" => {
                let value = string(value)?;
                if value.as_ref().is_some_and(|value| value.len() > 512) {
                    return Err(Exception::throw_type(ctx, "testId exceeds 512 UTF-8 bytes"));
                }
                Ok(Self::TestId(value))
            }
            "style" if empty => Ok(Self::Style(Style::Empty)),
            "style" if value.is_string() => Ok(Self::Style(Style::Text(String::from_js(ctx, value)?))),
            "style" => {
                let object = value.as_object().filter(|_| !value.is_array() && !value.is_function())
                    .ok_or_else(|| Exception::throw_type(ctx, "style must be a CSS string or string-valued object"))?;
                let mut properties = Vec::new();
                for property in object.props::<String, Value>() {
                    let (key, value) = property?;
                    if !STYLE_PROPERTIES.contains(&key.as_str()) && CustomPropertyName::parse(&key).is_none() {
                        return Err(Exception::throw_type(ctx, &format!("unsupported native style property: {key}")));
                    }
                    properties.push((key, string(value)?));
                }
                Ok(Self::Style(Style::Object(properties)))
            }
            "id" => Ok(Self::Attribute("id", string(value)?)),
            "src" => {
                let source = string(value)?;
                if source.as_ref().is_some_and(|source| source.len() > 4096) {
                    return Err(Exception::throw_type(ctx, "src exceeds 4096 UTF-8 bytes"));
                }
                Ok(Self::Attribute("src", source))
            }
            "title" => Ok(Self::Attribute("title", string(value)?)),
            "tabIndex" if empty => Ok(Self::Attribute("tabindex", None)),
            "tabIndex" => {
                let number = value.as_number().ok_or_else(|| Exception::throw_type(ctx, "tabIndex must be an integer"))?;
                if !number.is_finite() || number.fract() != 0.0 || number < i32::MIN as f64 || number > i32::MAX as f64 { return Err(Exception::throw_type(ctx, "tabIndex is outside the integer range")); }
                Ok(Self::Attribute("tabindex", Some((number as i32).to_string())))
            }
            "disabled" => Ok(Self::Disabled(if empty { false } else { bool::from_js(ctx, value)? })),
            "readOnly" => Ok(Self::ReadOnly(if empty { false } else { bool::from_js(ctx, value)? })),
            "value" => Ok(Self::Value(string(value)?.unwrap_or_default())),
            "defaultValue" => Ok(Self::DefaultValue(string(value)?.unwrap_or_default())),
            "ariaLabel" => Ok(Self::Label(string(value)?)),
            "ariaDescription" => Ok(Self::Description(string(value)?)),
            "role" => {
                let role = match string(value)?.as_deref() {
                    None => Role::GenericContainer,
                    Some("button") => Role::Button,
                    Some("group") => Role::Group,
                    Some("label") => Role::Label,
                    Some("textInput") => Role::TextInput,
                    Some("multilineTextInput") => Role::MultilineTextInput,
                    _ => return Err(Exception::throw_type(ctx, "unsupported accessibility role")),
                };
                Ok(Self::Role(role))
            }
            custom if CustomPropertyName::parse(custom).is_some() => Ok(Self::Custom(custom.to_owned(), string(value)?)),
            _ => Err(Exception::throw_type(ctx, &format!("unsupported native property: {name}"))),
        }
    }

    pub(crate) fn requires_editable(&self) -> bool {
        matches!(self, Self::Value(_) | Self::DefaultValue(_) | Self::ReadOnly(_))
    }
}

#[derive(Default)]
pub(crate) struct Properties {
    styles: HashMap<u64, Style>,
    semantics: HashMap<u64, Semantics>,
    initialized_values: HashSet<u64>,
}
impl Properties {
    pub(crate) fn remove(&mut self, token: u64) { self.styles.remove(&token); self.semantics.remove(&token); self.initialized_values.remove(&token); }
    pub(crate) fn clear(&mut self) { self.styles.clear(); self.semantics.clear(); self.initialized_values.clear(); }

    /// Returns an owned native value command, to execute after releasing this borrow.
    /// Only initialization metadata is retained; the native editor owns the text.
    pub(crate) fn apply(&mut self, dom: &DomHandle, node: NodeId, token: u64, mutation: Mutation) -> Option<String> {
        match mutation {
            Mutation::Value(value) => {
                self.initialized_values.insert(token);
                return Some(value);
            }
            Mutation::DefaultValue(value) => {
                return self.initialized_values.insert(token).then_some(value);
            }
            Mutation::Class(value) => {
                let mut classes = Vec::new();
                for class in value.as_deref().unwrap_or("").split_ascii_whitespace().map(ClassName::new) {
                    if !classes.contains(&class) { classes.push(class); }
                }
                dom.set_classes(node, &classes);
            }
            Mutation::Attribute(name, value) => dom.set_attribute(node, AttrName::new(name), value.as_deref()),
            Mutation::Custom(name, value) => dom.set_custom_property(node, CustomPropertyName::parse(&name).expect("parsed custom property"), value.as_deref()),
            Mutation::Style(next) => {
                let previous = self.styles.remove(&token).unwrap_or(Style::Empty);
                match (&previous, &next) {
                    (_, Style::Empty) => dom.set_style_text(node, None),
                    (_, Style::Text(text)) => dom.set_style_text(node, Some(text)),
                    (Style::Object(old), Style::Object(new)) => {
                        for (key, _) in old.iter().filter(|(key, _)| !new.iter().any(|(next, _)| next == key)) {
                            set_style(dom, node, key, None);
                        }
                        // Reapply in JS enumeration order. Removing a shorthand also
                        // removes its longhands, and custom props may have been written
                        // directly since the previous style-object assignment.
                        for (key, value) in new { set_style(dom, node, key, value.as_deref()); }
                    }
                    (_, Style::Object(properties)) => {
                        dom.set_style_text(node, None);
                        for (key, value) in properties { set_style(dom, node, key, value.as_deref()); }
                    }
                }
                if next != Style::Empty { self.styles.insert(token, next); }
            }
            semantic => {
                let semantics = self.semantics.entry(token).or_default();
                match semantic {
                    Mutation::Disabled(disabled) => {
                        dom.set_ui_state(node, UiState::DISABLED, disabled);
                        if disabled { semantics.flags |= SemanticFlags::DISABLED; }
                        else { semantics.flags = semantics.flags & !SemanticFlags::DISABLED; }
                    }
                    Mutation::ReadOnly(read_only) => {
                        dom.set_ui_state(node, UiState::READ_ONLY, read_only);
                        if read_only { semantics.flags |= SemanticFlags::READ_ONLY; }
                        else { semantics.flags = semantics.flags & !SemanticFlags::READ_ONLY; }
                    }
                    Mutation::Role(role) => semantics.role = role,
                    Mutation::Label(label) => semantics.label = label.map(Into::into),
                    Mutation::Description(description) => semantics.description = description.map(Into::into),
                    _ => unreachable!(),
                }
                if *semantics == Semantics::default() { dom.set_semantics(node, None); self.semantics.remove(&token); }
                else { dom.set_semantics(node, Some(semantics)); }
            }
        }
        None
    }
}
fn set_style(dom: &DomHandle, node: NodeId, name: &str, value: Option<&str>) {
    if let Some(property) = CustomPropertyName::parse(name) { dom.set_custom_property(node, property, value); }
    else { dom.set_style_property(node, name, value); }
}

// Native Servo CSS names, deliberately narrower than the complete upstream CSS
// parser. Numeric unit coercion and browser-style camelCase names are not supported.
const STYLE_PROPERTIES: &[&str] = &[
    "display", "flex-direction", "flex-wrap", "flex-grow", "flex-shrink", "flex-basis",
    "align-items", "justify-content", "gap", "row-gap", "column-gap",
    "padding", "padding-top", "padding-right", "padding-bottom", "padding-left",
    "margin", "margin-top", "margin-right", "margin-bottom", "margin-left",
    "width", "height", "min-width", "min-height", "max-width", "max-height",
    "color", "background", "background-color", "border", "border-color",
    "border-width", "border-radius", "font-family", "font-size", "font-weight",
    "line-height", "opacity", "overflow",
];
