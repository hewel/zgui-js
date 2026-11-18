# zgui-style

Style engine for zgui documents, built on Stylo.

This crate is part of [zgui](https://github.com/zortax/zgui), a native Rust UI framework.
It is an internal part of the framework. Applications depend on
[`zgui`](https://crates.io/crates/zgui).

## Native StyleX profile

`preflight_stylesheet(&str)` runs the installation parser without installing anything and rejects
every parser drop plus unsupported `zgui-stylex-v1` features. Its serializable `StylesheetSummary`
contains the top-level rule count and statically parsed named `font_families` (including nested
rules, shorthand declarations, and keyframes); rejected `CssDiagnostics` serialize generated locations.
Families resolved through custom properties still need validation after the native cascade.
Priority selectors, native tag-agnostic `:root`, themes, and keyframes remain unmodified.
The only admitted `@property` form is the official dynamic-variable registration:
`@property --name { syntax: "*"; inherits: false; }`. Native Stylo registrations supply
non-inheritance and follow their owning stylesheet through replacement and removal. Typed syntax,
`inherits: true`, and any `initial-value` remain rejected; no generated CSS is rewritten.
Callers must preflight the exact candidate bytes before queuing a replacement; the ordinary
installation API still installs the parseable subset and is not transactional.
