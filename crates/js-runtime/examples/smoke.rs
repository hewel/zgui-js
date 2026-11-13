use js_runtime::{JsRuntime, RuntimeLimits};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("usage: cargo run -p js-runtime --example smoke -- <bundle.mjs>")?;
    let mut runtime = JsRuntime::new(RuntimeLimits::default())?;
    println!("{}", runtime.run_smoke_module_file(path)?);
    Ok(())
}
