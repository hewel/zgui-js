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
use zgui::{view::{AttrName, ClassName, CustomPropertyName, DomHandle, NodeId, PropKey, PropValue, Semantics, UiState}, vocab::{HasPopup, Invalid, Live, Orientation, Role, SemanticFlags, Toggled}};

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
    Placeholder(Option<String>),
    Role(Role),
    Label(Option<String>),
    Description(Option<String>),
    KeyShortcuts(Option<String>),
    Custom(String, Option<String>),
    DataAttribute(String, Option<String>),
    Drawing(&'static str, Option<String>),
    Checked(Option<Toggled>),
    Expanded(Option<bool>),
    Selected(Option<bool>),
    Open(bool),
    Invalid(bool),
    Required(bool),
    Hidden(bool),
    Modal(bool),
    HasPopup(Option<HasPopup>),
    Orientation(Option<Orientation>),
    Level(Option<usize>),
    Busy(bool),
    Live(Option<Live>),
    Numeric(NumericField, Option<f64>),
    Relation(Relation, Vec<zgui::vocab::NodeId>),
}

#[derive(Clone, Copy)]
pub(crate) enum NumericField { Min, Max, Value, Step }

#[derive(Clone, Copy)]
pub(crate) enum Relation { LabelledBy, DescribedBy, Controls, Owns, RadioGroup, PopupFor }

impl Relation {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "ariaLabelledBy" => Self::LabelledBy,
            "ariaDescribedBy" => Self::DescribedBy,
            "ariaControls" => Self::Controls,
            "ariaOwns" => Self::Owns,
            "ariaRadioGroup" => Self::RadioGroup,
            "ariaPopupFor" => Self::PopupFor,
            _ => return None,
        })
    }
}

fn numeric<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<Option<f64>> {
    if value.is_null() || value.is_undefined() { return Ok(None); }
    if !value.is_number() { return Err(Exception::throw_type(ctx, "aria value properties must be finite numbers")); }
    let number = f64::from_js(ctx, value)?;
    if !number.is_finite() { return Err(Exception::throw_type(ctx, "aria value properties must be finite numbers")); }
    Ok(Some(number))
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
            "placeholder" => Ok(Self::Placeholder(string(value)?)),
            "secure" => Ok(Self::Attribute("secure", if !empty && bool::from_js(ctx, value)? { Some(String::new()) } else { None })),
            "ariaBusy" => Ok(Self::Busy(if empty { false } else { bool::from_js(ctx, value)? })),
            "ariaLive" => {
                let live = match string(value)?.as_deref() {
                    None => None,
                    Some("off") => Some(Live::Off),
                    Some("polite") => Some(Live::Polite),
                    Some("assertive") => Some(Live::Assertive),
                    Some(_) => return Err(Exception::throw_type(ctx, "unsupported aria-live value")),
                };
                Ok(Self::Live(live))
            }
            "ariaValueMin" => Ok(Self::Numeric(NumericField::Min, numeric(ctx, value)?)),
            "ariaValueMax" => Ok(Self::Numeric(NumericField::Max, numeric(ctx, value)?)),
            "ariaValueNow" => Ok(Self::Numeric(NumericField::Value, numeric(ctx, value)?)),
            "ariaValueStep" => Ok(Self::Numeric(NumericField::Step, numeric(ctx, value)?)),
            "checked" => {
                let toggled = if empty {
                    None
                } else if value.is_bool() {
                    Some(Toggled::from(bool::from_js(ctx, value)?))
                } else if string(value.clone())?.as_deref() == Some("mixed") {
                    Some(Toggled::Mixed)
                } else {
                    return Err(Exception::throw_type(ctx, "checked must be a boolean, \"mixed\", null, or undefined"));
                };
                Ok(Self::Checked(toggled))
            }
            "expanded" => Ok(Self::Expanded(Option::<bool>::from_js(ctx, value)?)),
            "selected" => Ok(Self::Selected(Option::<bool>::from_js(ctx, value)?)),
            "open" => Ok(Self::Open(if empty { false } else { bool::from_js(ctx, value)? })),
            "invalid" => Ok(Self::Invalid(if empty { false } else { bool::from_js(ctx, value)? })),
            "required" => Ok(Self::Required(if empty { false } else { bool::from_js(ctx, value)? })),
            "ariaHidden" => Ok(Self::Hidden(if empty { false } else { bool::from_js(ctx, value)? })),
            "ariaModal" => Ok(Self::Modal(if empty { false } else { bool::from_js(ctx, value)? })),
            "ariaHasPopup" => {
                let popup = match string(value)?.as_deref() {
                    None => None,
                    Some("menu") => Some(HasPopup::Menu),
                    Some("listbox") => Some(HasPopup::Listbox),
                    Some("tree") => Some(HasPopup::Tree),
                    Some("grid") => Some(HasPopup::Grid),
                    Some("dialog") => Some(HasPopup::Dialog),
                    Some(_) => return Err(Exception::throw_type(ctx, "unsupported ariaHasPopup value")),
                };
                Ok(Self::HasPopup(popup))
            }
            "ariaLevel" if empty => Ok(Self::Level(None)),
            "ariaLevel" => {
                let level = value.as_number().ok_or_else(|| Exception::throw_type(ctx, "ariaLevel must be a number"))?;
                if !level.is_finite() || level.fract() != 0.0 || level < 1.0 || level > u32::MAX as f64 {
                    return Err(Exception::throw_range(ctx, "ariaLevel must be an integer between 1 and 4294967295"));
                }
                Ok(Self::Level(Some(level as usize)))
            }
            "orientation" => {
                let orientation = match string(value)?.as_deref() {
                    None => None,
                    Some("horizontal") => Some(Orientation::Horizontal),
                    Some("vertical") => Some(Orientation::Vertical),
                    Some(_) => return Err(Exception::throw_type(ctx, "unsupported orientation")),
                };
                Ok(Self::Orientation(orientation))
            }
            "d" => Ok(Self::Drawing("d", string(value)?)),
            "viewBox" => Ok(Self::Drawing("viewBox", string(value)?)),
            "ariaLabel" => Ok(Self::Label(string(value)?)),
            "ariaDescription" => Ok(Self::Description(string(value)?)),
            "ariaKeyShortcuts" => Ok(Self::KeyShortcuts(string(value)?)),
            "role" => {
                let role = match string(value)?.as_deref() {
                    None => Role::GenericContainer,
                    Some("alert") => Role::Alert,
                    Some("alertDialog") => Role::AlertDialog,
                    Some("button") => Role::Button,
                    Some("caption") => Role::Caption,
                    Some("cell") => Role::Cell,
                    Some("checkBox") => Role::CheckBox,
                    Some("columnHeader") => Role::ColumnHeader,
                    Some("comboBox") => Role::ComboBox,
                    Some("complementary") => Role::Complementary,
                    Some("dialog") => Role::Dialog,
                    Some("figure") => Role::Figure,
                    Some("form") => Role::Form,
                    Some("genericContainer") => Role::GenericContainer,
                    Some("grid") => Role::Grid,
                    Some("gridCell") => Role::GridCell,
                    Some("group") => Role::Group,
                    Some("heading") => Role::Heading,
                    Some("image") => Role::Image,
                    Some("label") => Role::Label,
                    Some("link") => Role::Link,
                    Some("list") => Role::List,
                    Some("listBox") => Role::ListBox,
                    Some("listBoxOption") => Role::ListBoxOption,
                    Some("listItem") => Role::ListItem,
                    Some("main") => Role::Main,
                    Some("menu") => Role::Menu,
                    Some("menuBar") => Role::MenuBar,
                    Some("menuItem") => Role::MenuItem,
                    Some("menuItemCheckBox") => Role::MenuItemCheckBox,
                    Some("menuItemRadio") => Role::MenuItemRadio,
                    Some("multilineTextInput") => Role::MultilineTextInput,
                    Some("navigation") => Role::Navigation,
                    Some("passwordInput") => Role::PasswordInput,
                    Some("progressIndicator") => Role::ProgressIndicator,
                    Some("radioButton") => Role::RadioButton,
                    Some("radioGroup") => Role::RadioGroup,
                    Some("region") => Role::Region,
                    Some("row") => Role::Row,
                    Some("rowGroup") => Role::RowGroup,
                    Some("rowHeader") => Role::RowHeader,
                    Some("scrollBar") => Role::ScrollBar,
                    Some("scrollView") => Role::ScrollView,
                    Some("slider") => Role::Slider,
                    Some("splitter") => Role::Splitter,
                    Some("status") => Role::Status,
                    Some("switch") => Role::Switch,
                    Some("tab") => Role::Tab,
                    Some("tabList") => Role::TabList,
                    Some("tabPanel") => Role::TabPanel,
                    Some("table") => Role::Table,
                    Some("textInput") => Role::TextInput,
                    Some("toolbar") => Role::Toolbar,
                    Some("tooltip") => Role::Tooltip,
                    _ => return Err(Exception::throw_type(ctx, "unsupported accessibility role")),
                };
                Ok(Self::Role(role))
            }
            custom if CustomPropertyName::parse(custom).is_some() => Ok(Self::Custom(custom.to_owned(), string(value)?)),
            data if data.starts_with("data-") => {
                let suffix = &data[5..];
                let valid = !suffix.is_empty() && suffix.bytes().all(|byte|
                    byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_');
                if !valid {
                    return Err(Exception::throw_type(ctx, "unsupported data attribute name"));
                }
                Ok(Self::DataAttribute(data.to_owned(), string(value)?))
            }
            _ => Err(Exception::throw_type(ctx, &format!("unsupported native property: {name}"))),
        }
    }

    pub(crate) fn requires_editable(&self) -> bool {
        matches!(self, Self::Value(_) | Self::DefaultValue(_) | Self::ReadOnly(_) | Self::Placeholder(_))
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
            Mutation::DataAttribute(name, value) => dom.set_attribute(node, AttrName::new(&name), value.as_deref()),
            Mutation::Drawing(name, value) => dom.set_property(node, PropKey::new(name), value.as_deref().map_or(PropValue::Unset, PropValue::from)),
            Mutation::Open(open) => dom.set_ui_state(node, UiState::OPEN, open),
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
                    Mutation::Checked(toggled) => {
                        dom.set_ui_state(node, UiState::CHECKED, toggled == Some(Toggled::True));
                        dom.set_ui_state(node, UiState::INDETERMINATE, toggled == Some(Toggled::Mixed));
                        semantics.toggled = toggled;
                    }
                    Mutation::Expanded(expanded) => semantics.expanded = expanded,
                    Mutation::Selected(selected) => semantics.selected = selected,
                    Mutation::Invalid(invalid) => {
                        dom.set_ui_state(node, UiState::INVALID, invalid);
                        semantics.invalid = invalid.then_some(Invalid::True);
                    }
                    Mutation::Required(required) => {
                        dom.set_ui_state(node, UiState::REQUIRED, required);
                        if required { semantics.flags |= SemanticFlags::REQUIRED; }
                        else { semantics.flags = semantics.flags & !SemanticFlags::REQUIRED; }
                    }
                    Mutation::Hidden(hidden) => {
                        if hidden { semantics.flags |= SemanticFlags::HIDDEN; }
                        else { semantics.flags = semantics.flags & !SemanticFlags::HIDDEN; }
                    }
                    Mutation::Modal(modal) => {
                        if modal { semantics.flags |= SemanticFlags::MODAL; }
                        else { semantics.flags = semantics.flags & !SemanticFlags::MODAL; }
                    }
                    Mutation::HasPopup(popup) => semantics.has_popup = popup,
                    Mutation::Orientation(orientation) => semantics.orientation = orientation,
                    Mutation::Placeholder(placeholder) => semantics.placeholder = placeholder.map(Into::into),
                    Mutation::Busy(busy) => {
                        if busy { semantics.flags |= SemanticFlags::BUSY; }
                        else { semantics.flags = semantics.flags & !SemanticFlags::BUSY; }
                    }
                    Mutation::Live(live) => semantics.live = live,
                    Mutation::Numeric(field, number) => match field {
                        NumericField::Min => semantics.numeric.min = number,
                        NumericField::Max => semantics.numeric.max = number,
                        NumericField::Value => semantics.numeric.value = number,
                        NumericField::Step => semantics.numeric.step = number,
                    },
                    Mutation::Relation(relation, targets) => match relation {
                        Relation::LabelledBy => semantics.relations.labelled_by = targets,
                        Relation::DescribedBy => semantics.relations.described_by = targets,
                        Relation::Controls => semantics.relations.controls = targets,
                        Relation::Owns => semantics.relations.owns = targets,
                        Relation::RadioGroup => semantics.relations.radio_group = targets,
                        Relation::PopupFor => semantics.relations.popup_for = targets.first().copied(),
                    },
                    Mutation::Role(role) => semantics.role = role,
                    Mutation::Level(level) => semantics.position.level = level,
                    Mutation::Label(label) => semantics.label = label.map(Into::into),
                    Mutation::Description(description) => semantics.description = description.map(Into::into),
                    Mutation::KeyShortcuts(shortcuts) => semantics.keyboard_shortcut = shortcuts.map(Into::into),
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
    "display", "position", "inset", "top", "right", "bottom", "left", "visibility",
    "flex-direction", "flex-wrap", "flex-grow", "flex-shrink", "flex-basis",
    "align-items", "justify-content", "gap", "row-gap", "column-gap",
    "padding", "padding-top", "padding-right", "padding-bottom", "padding-left",
    "margin", "margin-top", "margin-right", "margin-bottom", "margin-left",
    "width", "height", "min-width", "min-height", "max-width", "max-height",
    "color", "background", "background-color", "border", "border-color",
    "border-width", "border-radius", "font-family", "font-size", "font-weight",
    "line-height", "opacity", "overflow",
];
