mod calibrate;
mod capture;
mod inventory;
mod output;
mod profile;
mod supervisor;
pub(crate) mod driver;
pub(crate) mod evidence;
pub(crate) mod fixture;
mod inspect;
mod run;
mod scenario;
mod semantics;
mod selectors;
mod saved;
mod compare;
mod watch;
mod resources;

use std::path::PathBuf;
use serde_json::{Value, json};

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Error {
    pub code: &'static str,
    pub message: String,
    pub exit_code: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl Error {
    pub fn new(code: &'static str, exit_code: u8, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), exit_code, details:None }
    }
    pub fn with_details(mut self, details: Value) -> Self { self.details=Some(details); self }
}

impl std::fmt::Display for Error {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}

const MAX_STDOUT_BYTES: usize = 8 * 1024;

pub(crate) fn command(mut args: impl Iterator<Item = String>) -> u8 {
    let command = args.next().unwrap_or_default();
    if command == "_doctor-worker" || command == "_run-worker" {
        let report = if command == "_doctor-worker" { doctor_worker(args) } else { run::worker(args) };
        println!("{}", serde_json::to_string(&report).expect("worker report serializes"));
        return 0;
    }
    if command == "watch" { return watch::command(args.collect()); }
    let outcome = match command.as_str() {
        "doctor" => doctor(args.collect()),
        "run" => run::command(args.collect()),
        "inspect" => saved::command(args.collect()),
        "compare" => compare::command(args.collect()),
        _ => Err(Error::new("E_SCHEMA", 2, "expected preview doctor, run, inspect, compare or watch")),
    };
    let result = outcome.unwrap_or_else(|error| failure(&command, error));
    let code = result["exitCode"].as_u64().unwrap_or(9) as u8;
    println!("{}", serde_json::to_string(&result).expect("JSON values serialize"));
    code
}

fn failure(command: &str, mut error: Error) -> Value {
    error.message = error.message.chars().take(1024).collect();
    let command: String = command.chars().take(64).collect();
    json!({"schemaVersion":1,"command":command,"status":"failed","exitCode":error.exit_code,"firstError":error})
}

struct DoctorOptions {
    profile: PathBuf,
    out: Option<PathBuf>,
    overrides: profile::Overrides,
}

fn doctor_options(mut args: impl Iterator<Item = String>) -> Result<DoctorOptions, Error> {
    let mut profile = None;
    let mut out = None;
    let mut overrides = profile::Overrides::default();
    let mut seen = std::collections::BTreeSet::new();
    while let Some(arg) = args.next() {
        if !seen.insert(arg.clone()) {
            return Err(Error::new("E_SCHEMA", 2, format!("duplicate option: {arg}")));
        }
        let value = args.next().ok_or_else(|| Error::new("E_SCHEMA", 2, format!("missing value for {arg}")))?;
        let invalid = || Error::new("E_SCHEMA", 2, format!("invalid value for {arg}"));
        match arg.as_str() {
            "--profile" => profile = Some(PathBuf::from(value)),
            "--out" => out = Some(PathBuf::from(value)),
            "--width" => overrides.width = Some(value.parse().map_err(|_| invalid())?),
            "--height" => overrides.height = Some(value.parse().map_err(|_| invalid())?),
            "--scale" => overrides.scale = Some(value.parse().map_err(|_| invalid())?),
            "--theme" => overrides.theme = Some(value),
            "--timeout-ms" => overrides.timeout_ms = Some(value.parse().map_err(|_| invalid())?),
            "--format" if value == "json" => {}
            _ => return Err(Error::new("E_SCHEMA", 2, format!("unsupported option: {arg}"))),
        }
    }
    Ok(DoctorOptions {
        profile: profile.ok_or_else(|| Error::new("E_SCHEMA", 2, "--profile is required"))?,
        out,
        overrides,
    })
}

fn doctor(args: Vec<String>) -> Result<Value, Error> {
    let options = doctor_options(args.iter().cloned())?;
    let (_, profile) = profile::read_profile(&options.profile, &options.overrides)?;
    let mut output = options.out.as_deref().map(|path| {
        output::Output::create(path, profile.limits.max_output_bytes)
    }).transpose()?;
    let executable = std::env::current_exe()
        .map_err(|error| Error::new("E_WORKER_CRASH", 9, error.to_string()))?;
    let mut child = std::process::Command::new(executable);
    child.args(["preview", "_doctor-worker"]).args(args)
        .env("ZGUI_PREVIEW_PARENT_PID", std::process::id().to_string());
    let report = supervisor::run(&mut child, std::time::Duration::from_millis(profile.limits.max_run_wall_ms));
    let result = match report {
        Ok(report) => {
            if let (Some(output), Some(state)) = (output.as_mut(), report.output_state) {
                output.adopt(state)?;
            }
            report.result
        }
        Err(error) => failure("doctor", error),
    };
    if serde_json::to_vec(&result).expect("JSON value serializes").len() + 1 > MAX_STDOUT_BYTES {
        return Err(Error::new("E_OUTPUT_LIMIT", 8, "doctor summary exceeds 8 KiB"));
    }
    if let Some(output) = output.as_mut() {
        let publication = (|| {
            if result["status"] != "passed" { output.recover()?; }
            output.finish(&result)
        })();
        if let Err(error) = publication {
            let mut failed = failure("doctor", error);
            failed["originalError"] = result["firstError"].clone();
            return Ok(failed);
        }
    }
    Ok(result)
}

fn doctor_worker(args: impl Iterator<Item = String>) -> supervisor::Report {
    let mut output = None;
    let outcome = (|| {
        let options = doctor_options(args)?;
        let profile = profile::load(&options.profile, &options.overrides)?;
        output = options.out.as_deref().map(|path| {
            let parent_pid = std::env::var("ZGUI_PREVIEW_PARENT_PID").ok()
                .and_then(|value| value.parse().ok())
                .ok_or_else(|| Error::new("E_HOST_INVARIANT", 9, "worker has no controller identity"))?;
            output::Output::attach_worker(path, profile.profile.limits.max_output_bytes, parent_pid)
        }).transpose()?;
    let mut environment = inventory::snapshot()?;
    let calibration = calibrate::calibrate(&profile)?;
    let pixel_digest = output::digest(&calibration.png);
    environment["profileDigest"] = json!(profile.digest);
    environment["fonts"] = profile.font_evidence.clone();
    environment["calibration"] = calibration.metadata.clone();
    environment["capabilities"] = capabilities(true);
    let selected_dependencies: Vec<_> = environment["dependencies"]["resolvedCargoPackages"]
        .as_array().expect("inventory package array").iter()
        .filter(|package| matches!(package["name"].as_str(), Some("zgui" | "rquickjs" | "rquickjs-sys" | "fontique")))
        .cloned().collect();
    let result = json!({
        "schemaVersion":1, "command":"doctor", "status":"passed", "exitCode":0,
        "firstError":null, "capabilityRevision":"native-preview-v1-p0",
        "capabilities":capabilities(true),
        "profile":{"id":profile.profile.id,"digest":profile.digest,"physicalWidth":profile.physical_width,"physicalHeight":profile.physical_height},
        "host":environment["host"],
        "dependencies":{"cargoLockDigest":environment["dependencies"]["cargoLockDigest"],"pnpmLockDigest":environment["dependencies"]["pnpmLockDigest"],"selected":selected_dependencies},
        "graphics":calibration.metadata.get("renderer"),
        "calibration":{"pngDigest":pixel_digest,"path":output.as_ref().map(|_| "calibration.png")},
        "runDirectory":output.as_ref().map(|output| output.root()),
        "artifacts":output.as_ref().map(|_| json!({"manifest":"artifacts.json","environment":"environment.json","calibration":"calibration.png","frame":"calibration.frame.json"}))
    });
    if serde_json::to_vec(&result).expect("JSON value serializes").len() + 1 > MAX_STDOUT_BYTES {
        return Err(Error::new("E_OUTPUT_LIMIT", 8, "doctor summary exceeds 8 KiB"));
    }
    if let Some(output) = output.as_mut() {
        output.json("inputs/profile.json", "resolved-profile", &profile.resolved_json)?;
        output.json("environment.json", "environment", &environment)?;
        output.bytes("calibration.png", "calibration-pixels", &calibration.png)?;
        output.json("calibration.frame.json", "calibration-metadata", &calibration.metadata)?;
    }
        Ok(result)
    })();
    supervisor::Report {
        result: outcome.unwrap_or_else(|error| failure("doctor", error)),
        output_state: output.map(output::Output::into_state),
    }
}

fn capabilities(pixels: bool) -> Value {
    json!({
        "pixels":pixels, "virtual-time":true, "services.fixtures":true,
        "selectors.test-id":true, "selectors.semantic":true, "tree.inspect":true,
        "input.pointer":true, "input.keyboard":true, "input.text":true, "input.ime":false,
        "assets.native-readiness":true, "capture.transparent":false, "capture.sequence":false,
        "surface.resize":true, "surface.scale":true, "surface.recreate":true, "surface.multiple":false,
        "lifecycle.reload":false, "source.node-location":false
    })
}

const RUN_CAPABILITIES: &[&str] = &[
    "pixels", "selectors.test-id", "selectors.semantic", "tree.inspect",
    "input.pointer", "input.keyboard", "input.text",
    "virtual-time", "services.fixtures", "assets.native-readiness", "surface.resize", "surface.scale", "surface.recreate",
];
