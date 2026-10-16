//! Build automation: `cargo xtask ci | layers`.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

mod samples;

/// Layer of each workspace crate (by package name). Apps and xtask are unrestricted.
const LAYERS: &[(&str, u8)] = &[
    ("polyroom-geom", 0),
    ("polyroom-mesh", 0),
    ("polyroom-testkit", 0),
    ("polyroom-formats", 1),
    ("polyroom-render", 2),
    ("polyroom-catalog", 3),
    ("polyroom-preview", 3),
    ("polyroom-engine", 4),
    ("polyroom-ui-egui", 5),
];

/// Dependencies that only the UI layer may use.
const UI_ONLY: &[&str] = &["egui", "eframe", "egui-wgpu", "egui_extras", "winit", "rfd"];

fn main() -> ExitCode {
    let task = std::env::args().nth(1).unwrap_or_default();
    let result = match task.as_str() {
        "ci" => ci(),
        "layers" => layers(),
        "samples" => match std::env::args().nth(2) {
            Some(dir) => samples::write_samples(Path::new(&dir)),
            None => Err("usage: cargo xtask samples <dir>".to_owned()),
        },
        _ => Err("usage: cargo xtask <ci|layers|samples <dir>>".to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::FAILURE
        }
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn cargo(args: &[&str]) -> Result<(), String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    eprintln!("$ cargo {}", args.join(" "));
    let status = Command::new(cargo)
        .args(args)
        .current_dir(root())
        .status()
        .map_err(|e| format!("failed to run cargo: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`cargo {}` failed", args.join(" ")))
    }
}

fn ci() -> Result<(), String> {
    cargo(&["fmt", "--all", "--check"])?;
    cargo(&[
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ])?;
    cargo(&["test", "--workspace"])?;
    layers()?;
    eprintln!("ci: all green");
    Ok(())
}

fn layer_of(name: &str) -> Option<u8> {
    LAYERS.iter().find(|(n, _)| *n == name).map(|(_, l)| *l)
}

fn manifests() -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    let dir = root().join("crates");
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries.flatten() {
        let manifest = entry.path().join("Cargo.toml");
        if manifest.is_file() {
            out.push(manifest);
        }
    }
    out.sort();
    Ok(out)
}

fn layers() -> Result<(), String> {
    let mut errors = Vec::new();
    for manifest in manifests()? {
        let text = std::fs::read_to_string(&manifest)
            .map_err(|e| format!("{}: {e}", manifest.display()))?;
        let doc: toml::Table = text
            .parse()
            .map_err(|e| format!("{}: {e}", manifest.display()))?;
        let Some(name) = doc
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
        else {
            errors.push(format!("{}: missing package.name", manifest.display()));
            continue;
        };
        let Some(layer) = layer_of(name) else {
            errors.push(format!("{name}: not assigned a layer in xtask LAYERS"));
            continue;
        };
        // Dev-dependencies are exempt: tests may pull in anything.
        let Some(deps) = doc.get("dependencies").and_then(|d| d.as_table()) else {
            continue;
        };
        for dep in deps.keys() {
            if layer < 5 && UI_ONLY.contains(&dep.as_str()) {
                errors.push(format!("{name} (L{layer}) depends on UI crate `{dep}`"));
            }
            if let Some(dep_layer) = layer_of(dep)
                && dep_layer >= layer
                && dep != "polyroom-testkit"
            {
                errors.push(format!("{name} (L{layer}) depends on {dep} (L{dep_layer})"));
            }
        }
    }
    if errors.is_empty() {
        eprintln!("layers: ok");
        Ok(())
    } else {
        Err(format!("layer violations:\n  {}", errors.join("\n  ")))
    }
}
