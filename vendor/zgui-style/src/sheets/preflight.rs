//! Strict, installation-free parsing for the native StyleX v1 capability profile.

use cssparser::{Parser, ParserInput, SourceLocation, Token};
use style::properties::{PropertyDeclaration, PropertyDeclarationBlock};
use style::properties_and_values::rule::Inherits;
use style::shared_lock::SharedRwLockReadGuard;
use style::stylesheets::{CssRule, Origin};
use style::values::computed::font::SingleFontFamily;
use style::values::specified::FontFamily;

use crate::sheets::errors::{CssDiagnostics, DiagnosticSink, DropKind};
use crate::sheets::parse::{ParseContext, base_url};

/// A serializable description of a sheet accepted by the native profile.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct StylesheetSummary {
    /// Number of top-level parsed rules, including keyframes and grouping rules.
    pub rule_count: usize,
    /// Statically parsed named font families in first-use order, with duplicates and generics omitted.
    ///
    /// Includes shorthand declarations, nested rules, and keyframes even in inactive conditions.
    /// Variable-resolved families are not static and require validation after the native cascade.
    pub font_families: Vec<String>,
}

/// Parses a candidate with the installation parser, rejecting every drop and unsupported v1 feature.
///
/// No rules are installed, imports are never fetched, and selectors are never rewritten. Success
/// is not a transaction or an installation token: callers must preflight the exact bytes before
/// queuing a replacement, and retain their previous sheet when this returns diagnostics.
///
/// # Errors
///
/// Returns all parser complaints and native-profile rejections with generated CSS locations.
pub fn preflight_stylesheet(css: &str) -> Result<StylesheetSummary, CssDiagnostics> {
    zgui_css::enable_css_features();
    let document = zgui_dom::Document::new();
    let lock = document.store().lock();
    let url = base_url();
    let sink = DiagnosticSink::new();
    let sheet = ParseContext {
        lock,
        url: &url,
        loader: document.store().host().sheets(),
        sink: &sink,
    }.parse(css, Origin::Author);
    let mut input = ParserInput::new(css);
    scan(&mut Parser::new(&mut input), &sink, false, false);
    let guard = lock.read();
    let contents = sheet.0.contents.read_with(&guard);
    let rules = contents.rules.read_with(&guard);
    validate_registrations(&rules.0, &guard, &sink);
    let diagnostics = sink.take();
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    let mut font_families = Vec::new();
    collect_font_families(&rules.0, &guard, &mut font_families);
    Ok(StylesheetSummary { rule_count: rules.0.len(), font_families })
}

fn validate_registrations(rules: &[CssRule], guard: &SharedRwLockReadGuard, sink: &DiagnosticSink) {
    for rule in rules {
        if let CssRule::Property(registration) = rule {
            let descriptors = &registration.descriptors;
            if !descriptors.is_universal()
                || descriptors.inherits != Some(Inherits::False)
                || descriptors.initial_value.is_some()
            {
                reject(sink, registration.source_location, "@property outside syntax: \"*\"; inherits: false; without initial-value");
            }
        }
        validate_registrations(rule.children(guard), guard, sink);
    }
}

fn collect_font_families(rules: &[CssRule], guard: &SharedRwLockReadGuard, families: &mut Vec<String>) {
    for rule in rules {
        match rule {
            CssRule::Style(rule) => collect_block_fonts(rule.read_with(guard).block.read_with(guard), families),
            CssRule::NestedDeclarations(rule) => collect_block_fonts(rule.read_with(guard).block.read_with(guard), families),
            CssRule::Keyframes(rule) => {
                for frame in &rule.read_with(guard).keyframes {
                    collect_block_fonts(frame.read_with(guard).block.read_with(guard), families);
                }
            }
            _ => {}
        }
        collect_font_families(rule.children(guard), guard, families);
    }
}

fn collect_block_fonts(block: &PropertyDeclarationBlock, families: &mut Vec<String>) {
    for declaration in block.declarations() {
        if let PropertyDeclaration::FontFamily(FontFamily::Values(list)) = declaration {
            for family in list.iter() {
                if let SingleFontFamily::FamilyName(family) = family {
                    let name: &str = family.name.as_ref();
                    if !families.iter().any(|existing| existing == name) {
                        families.push(name.to_owned());
                    }
                }
            }
        }
    }
}

fn reject(sink: &DiagnosticSink, location: SourceLocation, feature: &str) {
    sink.record(location, DropKind::Other, format!("{feature} is unsupported by zgui-stylex-v1"));
}

// CSS tokens decode escapes and distinguish comments/strings from actual functions and selectors.
// Descending into blocks also catches forbidden values in custom properties and inactive rules.
fn scan(input: &mut Parser<'_, '_>, sink: &DiagnosticSink, html_context: bool, media_context: bool) {
    let mut html = html_context;
    let mut class_prefix = false;
    let mut media = media_context;
    loop {
        let location = input.current_source_location();
        let Ok(token) = input.next_including_whitespace_and_comments().cloned() else { break };
        match token {
            Token::AtKeyword(name) => {
                if ["container", "import", "font-face"].iter().any(|s| name.eq_ignore_ascii_case(s)) {
                    reject(sink, location, &format!("@{name}"));
                }
                html = false;
                class_prefix = false;
                media = name.eq_ignore_ascii_case("media");
            }
            Token::Ident(name) => {
                if media && name.eq_ignore_ascii_case("prefers-reduced-motion") {
                    reject(sink, location, "prefers-reduced-motion");
                }
                html = !class_prefix && name.eq_ignore_ascii_case("html");
                class_prefix = false;
            }
            Token::UnquotedUrl(_) | Token::BadUrl(_) => reject(sink, location, "CSS url()"),
            Token::Function(name) => {
                if ["url", "image-set", "-webkit-image-set"].iter().any(|s| name.eq_ignore_ascii_case(s)) {
                    reject(sink, location, &format!("CSS {name}()"));
                }
                let nested_html = html && name.eq_ignore_ascii_case("not");
                let _: Result<(), cssparser::ParseError<'_, ()>> = input.parse_nested_block(|nested| {
                    scan(nested, sink, nested_html, media);
                    Ok(())
                });
                html = false;
                class_prefix = false;
            }
            Token::SquareBracketBlock => {
                let _: Result<(), cssparser::ParseError<'_, ()>> = input.parse_nested_block(|nested| {
                    if html {
                        let location = nested.current_source_location();
                        if nested.try_parse(|p| p.expect_ident_matching("dir")).is_ok() {
                            reject(sink, location, "browser html[dir]/html:not([dir]) RTL selectors");
                        }
                    }
                    scan(nested, sink, false, false);
                    Ok(())
                });
                html = false;
                class_prefix = false;
            }
            Token::ParenthesisBlock => {
                let _: Result<(), cssparser::ParseError<'_, ()>> = input.parse_nested_block(|nested| {
                    scan(nested, sink, false, media);
                    Ok(())
                });
                html = false;
                class_prefix = false;
            }
            Token::CurlyBracketBlock => {
                let _: Result<(), cssparser::ParseError<'_, ()>> = input.parse_nested_block(|nested| {
                    scan(nested, sink, false, false);
                    Ok(())
                });
                html = false;
                media = false;
                class_prefix = false;
            }
            Token::WhiteSpace(_) | Token::Comment(_) | Token::Colon => {}
            Token::Delim('.') => { class_prefix = true; html = false; }
            _ => { html = false; class_prefix = false; }
        }
    }
}
