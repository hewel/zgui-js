use std::{fs, path::Path};

#[test]
fn generated_sdk_styles_and_themes_pass_strict_native_preflight() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packages/ui/src/components/generated");
    let mut failures = Vec::new();
    let mut paths: Vec<_> = fs::read_dir(&root).unwrap().map(|entry| entry.unwrap().path()).collect();
    paths.sort();
    for path in paths {
        let source = fs::read_to_string(&path).unwrap();
        if path.file_name().unwrap().to_string_lossy().ends_with("Style.ts") {
            let (_, css) = source.split_once(", css: ").expect("generated stylesheet CSS field");
            let css = serde_json::Deserializer::from_str(css).into_iter::<String>().next().unwrap().unwrap();
            if let Err(diagnostics) = zgui_style::preflight_stylesheet(&format!("@layer zgui.components {{ {css} }}")) {
                failures.push(format!("{}: {diagnostics:#?}", path.file_name().unwrap().to_string_lossy()));
            }
        } else if path.file_name().unwrap() == "themes.ts" {
            let (_, themes) = source.split_once("export const themes = ").unwrap();
            let themes = serde_json::Deserializer::from_str(themes).into_iter::<serde_json::Value>().next().unwrap().unwrap();
            for (name, schemes) in themes.as_object().unwrap() {
                for (scheme, css) in schemes.as_object().unwrap() {
                    if let Err(diagnostics) = zgui_style::preflight_stylesheet(&format!("@layer zgui.tokens {{ :root {{ color-scheme: {scheme}; {} }} }}", css.as_str().unwrap())) {
                        failures.push(format!("theme {name}/{scheme}: {diagnostics:#?}"));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "SDK resources must not rely on native error recovery:\n{}", failures.join("\n"));
}
