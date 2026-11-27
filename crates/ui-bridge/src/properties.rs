//! Native property vocabulary. Null and undefined both clear a property's value.
//! Field/editor `value` loads through the native host, with empty values clearing
//! to empty text. `defaultValue` initializes only once and is ignored after any
//! explicit value assignment; only initialization metadata is retained here.
//!
//! Application classes and inline declarations are separate from SDK ownership.
//! All inline updates parse before committing through the native DOM transaction.

use std::collections::{BTreeSet, HashMap, HashSet};
use rquickjs::{Ctx, Exception, FromJs, Value};
use zgui::{
    view::{AttrName, ClassName, CustomPropertyName, DomHandle, NodeId, PropKey, PropValue, Semantics, UiState},
    vocab::{AriaCurrent, HasPopup, Invalid, Live, Orientation, Role, SemanticFlags, SortDirection, Toggled},
};
use zgui::vocab::prop::drawing;

#[derive(Clone, Default, PartialEq)]
pub(crate) enum Style { #[default] Empty, Text(String), Object(Vec<(String, Option<String>)>) }

#[derive(Clone, Copy)]
pub(crate) enum StyleOwner { Application, Defaults, Invariants }

pub(crate) enum Mutation {
    Class(&'static str, Option<String>),
    TestId(Option<String>),
    Style(StyleOwner, Style),
    Attribute(&'static str, Option<String>),
    Disabled(bool),
    Inert(bool),
    ReadOnly(bool),
    Value(String),
    DefaultValue(String),
    Placeholder(Option<String>),
    Role(Role),
    Label(Option<String>),
    Description(Option<String>),
    Sort(Option<SortDirection>),
    Current(Option<AriaCurrent>),
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
    ScalarRelation(ScalarRelation, Option<zgui::vocab::NodeId>),
}

#[derive(Clone, Copy)]
pub(crate) enum NumericField { Min, Max, Value, Step }

#[derive(Clone, Copy)]
pub(crate) enum Relation { LabelledBy, DescribedBy, Controls, Owns, RadioGroup, PopupFor }

#[derive(Clone, Copy)]
pub(crate) enum ScalarRelation { ActiveDescendant }

impl ScalarRelation {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        match name {
            "ariaActiveDescendant" => Some(Self::ActiveDescendant),
            _ => None,
        }
    }
}

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

fn validate_svg(ctx: &Ctx<'_>, source: &str) -> rquickjs::Result<()> {
    const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
    if source.trim().is_empty() { return Ok(()); }
    // roxmltree's default disables DTDs. The painter's parser has an ambient image resolver.
    let document = roxmltree::Document::parse(source)
        .map_err(|_| Exception::throw_type(ctx, "svg must be well-formed XML without a DTD"))?;
    let root = document.root_element();
    let root_tag = root.tag_name();
    if root_tag.name() != "svg" || !matches!(root_tag.namespace(), None | Some(SVG_NAMESPACE)) {
        return Err(Exception::throw_type(ctx, "svg must have an SVG root element"));
    }
    if root.descendants().filter(|node| node.is_element()).any(|node| {
        let tag = node.tag_name();
        matches!(tag.namespace(), None | Some(SVG_NAMESPACE))
            && matches!(tag.name(), "image" | "feImage")
    }) {
        return Err(Exception::throw_type(ctx, "svg drawing does not support image elements"));
    }
    Ok(())
}

impl Mutation {
    // Parse all JS properties/getters before borrowing any native property state.
    pub(crate) fn parse<'js>(ctx: &Ctx<'js>, name: &str, value: Value<'js>) -> rquickjs::Result<Self> {
        let empty = value.is_null() || value.is_undefined();
        let string = |value| Option::<String>::from_js(ctx, value);
        match name {
            "class" => Ok(Self::Class("class", string(value)?)),
            "__zguiClass" => Ok(Self::Class("__zguiClass", string(value)?)),
            "testId" => {
                let value = string(value)?;
                if value.as_ref().is_some_and(|value| value.len() > 512) {
                    return Err(Exception::throw_type(ctx, "testId exceeds 512 UTF-8 bytes"));
                }
                Ok(Self::TestId(value))
            }
            "style" | "__zguiStyle" | "__zguiInvariantStyle" => {
                let owner = match name {
                    "__zguiStyle" => StyleOwner::Defaults,
                    "__zguiInvariantStyle" => StyleOwner::Invariants,
                    _ => StyleOwner::Application,
                };
                if !matches!(owner, StyleOwner::Application) && value.is_string() {
                    return Err(Exception::throw_type(ctx, "STYLE_VALUE_INVALID: SDK ownership requires a declaration map"));
                }
                let style = if empty { Style::Empty }
                else if value.is_string() { Style::Text(String::from_js(ctx, value)?) }
                else {
                    let object = value.as_object().filter(|_| !value.is_array() && !value.is_function())
                        .ok_or_else(|| Exception::throw_type(ctx, "STYLE_VALUE_INVALID: style must be a CSS string or object"))?;
                    let mut properties = Vec::new();
                    for property in object.props::<String, Value>() {
                        let (key, value) = property?;
                        let key = normalize_name(ctx, key)?;
                        let value = style_value(ctx, &key, value)?;
                        properties.push((key, value));
                    }
                    Style::Object(properties)
                };
                Ok(Self::Style(owner, style))
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
            "inert" => Ok(Self::Inert(if empty { false } else { bool::from_js(ctx, value)? })),
            "readOnly" => Ok(Self::ReadOnly(if empty { false } else { bool::from_js(ctx, value)? })),
            "value" => Ok(Self::Value(string(value)?.unwrap_or_default())),
            "defaultValue" => Ok(Self::DefaultValue(string(value)?.unwrap_or_default())),
            "placeholder" => Ok(Self::Placeholder(string(value)?)),
            "secure" => Ok(Self::Attribute("secure", if !empty && bool::from_js(ctx, value)? { Some(String::new()) } else { None })),
            "documentSelection" => Ok(Self::Attribute(
                "data-zgui-document-selection",
                if !empty && bool::from_js(ctx, value)? { Some("true".to_owned()) } else { None },
            )),
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
            drawing::PATHS => Ok(Self::Drawing(drawing::PATHS, string(value)?)),
            drawing::VIEW_BOX => Ok(Self::Drawing(drawing::VIEW_BOX, string(value)?)),
            drawing::DOCUMENT => {
                let source = string(value)?;
                if let Some(source) = &source { validate_svg(ctx, source)?; }
                Ok(Self::Drawing(drawing::DOCUMENT, source))
            }
            "ariaLabel" => Ok(Self::Label(string(value)?)),
            "ariaDescription" => Ok(Self::Description(string(value)?)),
            "ariaSort" => Ok(Self::Sort(match string(value)?.as_deref() {
                None | Some("none") => None,
                Some("ascending") => Some(SortDirection::Ascending),
                Some("descending") => Some(SortDirection::Descending),
                Some("other") => Some(SortDirection::Other),
                Some(_) => return Err(Exception::throw_type(ctx, "unsupported ariaSort value")),
            })),
            "ariaCurrent" => Ok(Self::Current(match string(value)?.as_deref() {
                None => None,
                Some("false") => Some(AriaCurrent::False),
                Some("true") => Some(AriaCurrent::True),
                Some("page") => Some(AriaCurrent::Page),
                Some("step") => Some(AriaCurrent::Step),
                Some("location") => Some(AriaCurrent::Location),
                Some("date") => Some(AriaCurrent::Date),
                Some("time") => Some(AriaCurrent::Time),
                Some(_) => return Err(Exception::throw_type(ctx, "unsupported ariaCurrent value")),
            })),
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
            custom if CustomPropertyName::parse(custom).is_some() => Ok(Self::Custom(custom.to_owned(), style_value(ctx, custom, value)?)),
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
struct ClassState {
    canonical: Option<String>,
    internal: Option<String>,
    native: Option<String>,
}

#[derive(Default)]
struct StyleState {
    application: Style,
    defaults: Style,
    invariants: Style,
    direct: Vec<(String, Option<String>)>,
}

#[derive(Default)]
pub(crate) struct Properties {
    styles: HashMap<u64, StyleState>,
    classes: HashMap<u64, ClassState>,
    class_dirty: BTreeSet<u64>,
    semantics: HashMap<u64, Semantics>,
    initialized_values: HashSet<u64>,
}
impl Properties {
    pub(crate) fn remove(&mut self, token: u64) { self.styles.remove(&token); self.classes.remove(&token); self.class_dirty.remove(&token); self.semantics.remove(&token); self.initialized_values.remove(&token); }
    pub(crate) fn clear(&mut self) { self.styles.clear(); self.classes.clear(); self.class_dirty.clear(); self.semantics.clear(); self.initialized_values.clear(); }

    /// Returns an owned native value command, to execute after releasing this borrow.
    /// Only initialization metadata is retained; the native editor owns the text.
    pub(crate) fn apply(&mut self, dom: &DomHandle, node: NodeId, token: u64, mutation: Mutation) -> Result<Option<String>, crate::StyleDiagnostic> {
        match mutation {
            Mutation::Value(value) => {
                self.initialized_values.insert(token);
                return Ok(Some(value));
            }
            Mutation::DefaultValue(value) => {
                return Ok(self.initialized_values.insert(token).then_some(value));
            }
            Mutation::Class(name, value) => {
                let state = self.classes.entry(token).or_default();
                let value = value.filter(|value| !value.trim().is_empty());
                match name {
                    "class" => state.canonical = value,
                    _ => state.internal = value,
                }
                self.class_dirty.insert(token);
            }
            Mutation::Attribute(name, value) => dom.set_attribute(node, AttrName::new(name), value.as_deref()),
            Mutation::DataAttribute(name, value) => dom.set_attribute(node, AttrName::new(&name), value.as_deref()),
            Mutation::Drawing(name, value) => dom.set_property(node, PropKey::new(name), value.as_deref().map_or(PropValue::Unset, PropValue::from)),
            Mutation::Inert(inert) => dom.set_ui_state(node, UiState::INERT, inert),
            Mutation::Open(open) => dom.set_ui_state(node, UiState::OPEN, open),
            Mutation::Custom(name, value) => {
                let state = self.styles.entry(token).or_default();
                let direct = state.direct.iter().filter(|(key, _)| key != &name)
                    .map(|(key, value)| (key.as_str(), value.as_deref()))
                    .chain(std::iter::once((name.as_str(), value.as_deref())));
                commit_style(dom, node, &state.defaults, &state.application, direct, &state.invariants)?;
                state.direct.retain(|(key, _)| key != &name);
                if value.is_some() { state.direct.push((name, value)); }
            }
            Mutation::Style(owner, next) => {
                let state = self.styles.entry(token).or_default();
                let (defaults, application, direct, invariants) = match owner {
                    StyleOwner::Application => (&state.defaults, &next, &[][..], &state.invariants),
                    StyleOwner::Defaults => (&next, &state.application, state.direct.as_slice(), &state.invariants),
                    StyleOwner::Invariants => (&state.defaults, &state.application, state.direct.as_slice(), &next),
                };
                // Equality skips native work, except direct application writes invalidate it.
                let unchanged = match owner {
                    StyleOwner::Application => state.application == next && state.direct.is_empty(),
                    StyleOwner::Defaults => state.defaults == next,
                    StyleOwner::Invariants => state.invariants == next,
                };
                if !unchanged { commit_style(dom, node, defaults, application, direct.iter().map(|(key, value)| (key.as_str(), value.as_deref())), invariants)?; }
                match owner {
                    StyleOwner::Application => { state.application = next; state.direct.clear(); }
                    StyleOwner::Defaults => state.defaults = next,
                    StyleOwner::Invariants => state.invariants = next,
                }
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
                    Mutation::ScalarRelation(relation, target) => match relation {
                        ScalarRelation::ActiveDescendant => semantics.relations.active_descendant = target,
                    },
                    Mutation::Role(role) => semantics.role = role,
                    Mutation::Level(level) => semantics.position.level = level,
                    Mutation::Label(label) => semantics.label = label.map(Into::into),
                    Mutation::Description(description) => semantics.description = description.map(Into::into),
                    Mutation::Sort(direction) => semantics.sort_direction = direction,
                    Mutation::Current(current) => semantics.current = current,
                    Mutation::KeyShortcuts(shortcuts) => semantics.keyboard_shortcut = shortcuts.map(Into::into),
                    _ => unreachable!(),
                }
                if *semantics == Semantics::default() { dom.set_semantics(node, None); self.semantics.remove(&token); }
                else { dom.set_semantics(node, Some(semantics)); }
            }
        }
        Ok(None)
    }
    pub(crate) fn dirty_tokens(&self) -> Vec<u64> { self.class_dirty.iter().copied().collect() }

    pub(crate) fn finish_turn(&mut self, dom: &DomHandle, targets: Vec<(u64, NodeId)>) -> Vec<crate::StyleDiagnostic> {
        let diagnostics = Vec::new();
        for (token, native) in targets {
            self.class_dirty.remove(&token);
            let Some(state) = self.classes.get_mut(&token) else { continue; };
            let mut classes = Vec::new();
            for value in [state.internal.as_deref(), state.canonical.as_deref()].into_iter().flatten() {
                for class in value.split_ascii_whitespace().map(ClassName::new) {
                    if !classes.contains(&class) { classes.push(class); }
                }
            }
            let fingerprint = classes.iter().map(|class| class.as_str()).collect::<Vec<_>>().join(" ");
            if state.native.as_deref() != Some(fingerprint.as_str()) {
                dom.set_classes(native, &classes);
                state.native = Some(fingerprint);
            }
        }
        diagnostics
    }
}
fn commit_style<'a>(dom: &DomHandle, node: NodeId, defaults: &Style, application: &Style, direct: impl Iterator<Item = (&'a str, Option<&'a str>)>, invariants: &Style) -> Result<(), crate::StyleDiagnostic> {
    use zgui::view::dom::InlineStyle;
    fn append<'a>(entries: &mut Vec<InlineStyle<'a>>, style: &'a Style, important: bool) {
        match style {
            Style::Empty => {}
            Style::Text(text) => entries.push(InlineStyle::Text(text)),
            Style::Object(properties) => {
                for (key, value) in properties {
                    if let Some(value) = value { entries.push(InlineStyle::Declaration(key, value, important)); }
                }
            }
        }
    }
    let count = |style: &Style| match style { Style::Empty => 0, Style::Text(_) => 1, Style::Object(properties) => properties.len() };
    let mut entries = Vec::with_capacity(count(defaults) + count(application) + count(invariants) + direct.size_hint().1.unwrap_or(0));
    append(&mut entries, defaults, false);
    append(&mut entries, application, false);
    for (key, value) in direct {
        if let Some(value) = value { entries.push(InlineStyle::Declaration(key, value, false)); }
    }
    append(&mut entries, invariants, true);
    dom.replace_inline_style(node, &entries)
        .map_err(|message| crate::StyleDiagnostic { code: "STYLE_PARSE_INVALID", message })
}

fn normalize_name<'js>(ctx: &Ctx<'js>, key: String) -> rquickjs::Result<String> {
    if key.starts_with("--") {
        return CustomPropertyName::parse(&key).map(|_| key.clone())
            .ok_or_else(|| Exception::throw_type(ctx, "STYLE_NAME_INVALID: invalid custom property"));
    }
    if let Some((_, native)) = CAMEL_PROPERTIES.iter().find(|(alias, _)| *alias == key) {
        return Ok((*native).into());
    }
    if !key.is_empty() && key.bytes().all(|byte| byte.is_ascii_lowercase() || byte == b'-') { return Ok(key); }
    Err(Exception::throw_type(ctx, &format!("STYLE_NAME_INVALID: unsupported native style name {key}")))
}

fn style_value<'js>(ctx: &Ctx<'js>, key: &str, value: Value<'js>) -> rquickjs::Result<Option<String>> {
    if value.is_null() || value.is_undefined() { return Ok(None); }
    if let Some(number) = value.as_number() {
        if number.is_finite() && (key.starts_with("--") || UNITLESS.contains(&key)) { return Ok(Some(number.to_string())); }
        return Err(Exception::throw_type(ctx, &format!("STYLE_VALUE_INVALID: numeric {key} requires a finite custom or unitless value")));
    }
    if !value.is_string() { return Err(Exception::throw_type(ctx, "STYLE_VALUE_INVALID: style values must be strings, finite unitless numbers, or empty")); }
    Ok(Some(String::from_js(ctx, value)?))
}

const UNITLESS: &[&str] = &["opacity", "flex", "flex-grow", "flex-shrink", "font-weight", "line-height", "z-index", "order", "animation-iteration-count", "scale"];

const CAMEL_PROPERTIES: &[(&str, &str)] = &[
    ("boxSizing", "box-sizing"),
    ("flexDirection", "flex-direction"), ("flexWrap", "flex-wrap"), ("flexGrow", "flex-grow"), ("flexShrink", "flex-shrink"), ("flexBasis", "flex-basis"),
    ("alignItems", "align-items"), ("alignSelf", "align-self"), ("justifyContent", "justify-content"), ("justifySelf", "justify-self"),
    ("rowGap", "row-gap"), ("columnGap", "column-gap"),
    ("gridTemplateColumns", "grid-template-columns"), ("gridTemplateRows", "grid-template-rows"),
    ("gridColumn", "grid-column"), ("gridRow", "grid-row"), ("gridArea", "grid-area"), ("gridAutoFlow", "grid-auto-flow"),
    ("paddingTop", "padding-top"), ("paddingRight", "padding-right"), ("paddingBottom", "padding-bottom"), ("paddingLeft", "padding-left"),
    ("marginTop", "margin-top"), ("marginRight", "margin-right"), ("marginBottom", "margin-bottom"), ("marginLeft", "margin-left"),
    ("minWidth", "min-width"), ("minHeight", "min-height"), ("maxWidth", "max-width"), ("maxHeight", "max-height"),
    ("objectFit", "object-fit"), ("objectPosition", "object-position"),
    ("backgroundColor", "background-color"),
    ("borderTop", "border-top"), ("borderRight", "border-right"), ("borderBottom", "border-bottom"), ("borderLeft", "border-left"),
    ("borderTopStyle", "border-top-style"), ("borderRightStyle", "border-right-style"), ("borderBottomStyle", "border-bottom-style"), ("borderLeftStyle", "border-left-style"),
    ("borderTopWidth", "border-top-width"), ("borderRightWidth", "border-right-width"), ("borderBottomWidth", "border-bottom-width"), ("borderLeftWidth", "border-left-width"),
    ("borderTopColor", "border-top-color"), ("borderRightColor", "border-right-color"), ("borderBottomColor", "border-bottom-color"), ("borderLeftColor", "border-left-color"),
    ("borderColor", "border-color"), ("borderStyle", "border-style"), ("borderWidth", "border-width"), ("borderRadius", "border-radius"),
    ("outlineColor", "outline-color"), ("outlineStyle", "outline-style"), ("outlineWidth", "outline-width"), ("outlineOffset", "outline-offset"),
    ("fontFamily", "font-family"), ("fontSize", "font-size"), ("fontWeight", "font-weight"), ("lineHeight", "line-height"),
    ("letterSpacing", "letter-spacing"), ("fontVariantNumeric", "font-variant-numeric"), ("fontFeatureSettings", "font-feature-settings"),
    ("fontVariationSettings", "font-variation-settings"), ("fontStyle", "font-style"), ("fontStretch", "font-stretch"),
    ("textAlign", "text-align"), ("textTransform", "text-transform"), ("textDecoration", "text-decoration"), ("textDecorationLine", "text-decoration-line"),
    ("whiteSpace", "white-space"), ("textOverflow", "text-overflow"),
    ("overflowX", "overflow-x"), ("overflowY", "overflow-y"),
    ("boxShadow", "box-shadow"), ("textShadow", "text-shadow"), ("backdropFilter", "backdrop-filter"), ("mixBlendMode", "mix-blend-mode"), ("clipPath", "clip-path"),
    ("pointerEvents", "pointer-events"), ("userSelect", "user-select"),
    ("zIndex", "z-index"), ("transformOrigin", "transform-origin"),
    ("animationName", "animation-name"), ("animationDuration", "animation-duration"), ("animationDelay", "animation-delay"),
    ("animationTimingFunction", "animation-timing-function"), ("animationIterationCount", "animation-iteration-count"),
    ("animationFillMode", "animation-fill-mode"), ("animationDirection", "animation-direction"), ("animationPlayState", "animation-play-state"),
    ("transitionProperty", "transition-property"), ("transitionDuration", "transition-duration"), ("transitionDelay", "transition-delay"), ("transitionTimingFunction", "transition-timing-function"),
];
