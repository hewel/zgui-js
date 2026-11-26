import { runBounded } from "./run-bounded.mjs";

const SUITES = [
  {
    crate: "zgui-runtime",
    args: [
      "--test",
      "controlled",
      "--test",
      "value",
      "--test",
      "editing",
      "--test",
      "caret",
      "--test",
      "caret_reveal",
      "--test",
      "cursor",
      "--test",
      "popup_auto_height",
      "--test",
      "geometry_observation",
      "--test",
      "observe_owed",
      "--test",
      "observed_place",
      "--test",
      "scale",
      "--test",
      "damage",
      "--test",
      "drag_over",
      "--test",
      "shared_text",
      "--test",
      "anim_lifecycle",
      "--test",
      "scroll_root",
      "--test",
      "scroll_touchpad",
      "--test",
      "scroll_owed",
      "--test",
      "overlay_scrollbar",
      "--test",
      "scrollbar",
      "--test",
      "scroll_hit",
      "--test",
      "style_revisions",
      "--test",
      "image_policy",
      "--test",
      "images_end_to_end",
      "--test",
      "held_frame",
      "--test",
      "frame_report",
      "--test",
      "focus_trap",
      "--test",
      "inert",
      "--test",
      "session_lock",
      "--test",
      "shell_lifecycle",
      "--test",
      "window_state",
      "--test",
      "windowless",
      "--test",
      "document_selection",
      "--test",
      "blur_region",
    ],
  },
  {
    crate: "zgui-runtime",
    args: ["--lib"],
  },
  {
    crate: "zgui-platform",
    args: ["--lib"],
  },
  {
    crate: "zgui-platform-wayland",
    args: ["--lib"],
  },
  {
    crate: "zgui-platform-wayland",
    args: [
      "--test",
      "layer_protocol",
      "--test",
      "session_lock_protocol",
      "--test",
      "popup_protocol",
      "--test",
      "dnd_source_protocol",
      "--test",
      "background_effect_protocol",
    ],
  },
  {
    crate: "zgui-platform-headless",
    args: ["--lib"],
  },
  {
    crate: "zgui-platform-headless",
    args: ["--test", "session_lock"],
  },
  {
    crate: "zgui-layout",
    args: ["--lib", "--test", "box_tree", "--test", "scroll_region"],
  },
  {
    crate: "zgui-paint",
    args: ["--lib", "--test", "vector", "--test", "scrollbar_style"],
  },
  {
    crate: "zgui-scroll",
    args: ["--lib"],
  },
  {
    crate: "zgui-style",
    args: ["--lib", "driver::animations"],
  },
  {
    crate: "zgui-style",
    args: ["--test", "stylex_profile"],
  },
  {
    crate: "zgui-dom",
    args: ["--lib", "mutate::edit::style"],
  },
  {
    crate: "zgui-dom",
    args: ["--test", "engine"],
  },
  {
    crate: "zgui-input",
    args: ["--lib", "--test", "dispatch"],
  },
  {
    crate: "zgui-a11y",
    args: ["--lib"],
  },
  {
    crate: "zgui-text-parley",
    args: ["--lib", "raster::glyphs"],
  },
  {
    crate: "zgui-text-parley",
    args: ["--lib", "shape::glyphs"],
  },
  {
    crate: "zgui-text-parley",
    args: ["--lib", "shape::style"],
  },
  {
    crate: "zgui-text-parley",
    args: ["--lib", "system::instances"],
  },
  {
    crate: "zgui-view-dom",
    args: ["--test", "observations"],
  },
  {
    crate: "zgui",
    args: ["--lib"],
  },
];

const argv = process.argv.slice(2);
const fetchOnly = argv.includes("--fetch");
const filters = argv.filter(arg => arg !== "--fetch");

function selectSuites(requested) {
  if (requested.length === 0) return SUITES;
  const chosen = [];
  const seen = new Set();
  for (const filter of requested) {
    const exact = SUITES.filter(suite => suite.crate === filter);
    const matches = exact.length > 0 ? exact : SUITES.filter(suite => suite.crate.includes(filter));
    for (const suite of matches) {
      if (!seen.has(suite)) {
        seen.add(suite);
        chosen.push(suite);
      }
    }
  }
  return chosen;
}

const selected = selectSuites(filters);
if (selected.length === 0) {
  console.error(`No vendored test suites matched: ${filters.join(", ")}`);
  process.exit(1);
}

function targetCompileArgs(args) {
  const compileArgs = [];
  for (let i = 0; i < args.length; i += 1) {
    if (args[i] === "--lib") {
      compileArgs.push("--lib");
    } else if (args[i] === "--test" && i + 1 < args.length) {
      compileArgs.push("--test", args[i + 1]);
      i += 1;
    }
  }
  return compileArgs;
}

const steps = fetchOnly
  ? [...new Set(selected.map(suite => suite.crate))].map(crate => ({
      phase: "build",
      args: ["fetch", "--manifest-path", `vendor/${crate}/Cargo.toml`],
    }))
  : (() => {
      const planned = [];
      const compiledKeys = new Set();
      for (const suite of selected) {
        const compileTargets = targetCompileArgs(suite.args);
        const compileKey = `${suite.crate}:${compileTargets.join(" ")}`;
        if (!compiledKeys.has(compileKey)) {
          compiledKeys.add(compileKey);
          planned.push({
            phase: "build",
            args: [
              "test",
              "-j",
              "4",
              "--manifest-path",
              `vendor/${suite.crate}/Cargo.toml`,
              ...compileTargets,
              "--offline",
              "--no-run",
            ],
          });
        }
        planned.push({
          phase: "test",
          args: [
            "test",
            "-j",
            "4",
            "--manifest-path",
            `vendor/${suite.crate}/Cargo.toml`,
            ...suite.args,
            "--offline",
          ],
        });
      }
      return planned;
    })();

for (const step of steps) {
  console.log(`\n==> cargo ${step.args.join(" ")}`);
  const status = await runBounded("cargo", step.args, { phase: step.phase });
  if (status !== 0) {
    process.exitCode = status;
    break;
  }
}
