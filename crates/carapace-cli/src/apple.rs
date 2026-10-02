//! `cargo carapace build apple`: staticlibs per target -> lipo -> xcframework.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::Args;

fn run(cmd: &mut Command, what: &str) -> Result<(), String> {
    let status = cmd
        .status()
        .map_err(|e| format!("cannot run {what}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{what} failed ({status})"))
    }
}

/// (rust target, platform group, simulator?)
const MAC: [&str; 2] = ["aarch64-apple-darwin", "x86_64-apple-darwin"];
const IOS_DEVICE: &str = "aarch64-apple-ios";
const IOS_SIM: [&str; 2] = ["aarch64-apple-ios-sim", "x86_64-apple-ios"];

pub fn build(args: &Args) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("build apple needs macOS with Xcode".into());
    }
    let meta = cargo_metadata(args)?;
    let target_dir = PathBuf::from(
        meta["target_directory"]
            .as_str()
            .ok_or("cargo metadata has no target_directory")?,
    );
    let (pkg, lib_name) = staticlib_of(&meta, args)?;
    let profile = if args.debug { "debug" } else { "release" };
    let out_root = args.out.clone().unwrap_or_else(|| "apple".into());
    let work = target_dir.join("carapace-apple");
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;

    let mut groups: Vec<(&str, Vec<&str>)> = vec![("macos", MAC.to_vec())];
    if args.ios {
        groups.push(("ios", vec![IOS_DEVICE]));
        groups.push(("ios-sim", IOS_SIM.to_vec()));
    }

    let mut slices: Vec<PathBuf> = Vec::new();
    for (group, targets) in &groups {
        for t in targets {
            let mut cmd = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
            cmd.args(["build", "--lib", "--target", t, "-p", &pkg]);
            if !args.debug {
                cmd.arg("--release");
            }
            eprintln!("building {pkg} for {t}");
            run(
                &mut cmd,
                &format!("cargo build for {t} (is the target installed? `rustup target add {t}`)"),
            )?;
        }
        let inputs: Vec<PathBuf> = targets
            .iter()
            .map(|t| {
                target_dir
                    .join(t)
                    .join(profile)
                    .join(format!("lib{lib_name}.a"))
            })
            .collect();
        for i in &inputs {
            if !i.exists() {
                return Err(format!(
                    "expected {} after the build; does [lib] list crate-type \"staticlib\"?",
                    i.display()
                ));
            }
        }
        // Same file name in every slice: the linker looks for `-l<lib_name>`.
        std::fs::create_dir_all(work.join(group)).map_err(|e| e.to_string())?;
        let fat = work.join(group).join(format!("lib{lib_name}.a"));
        if inputs.len() == 1 {
            std::fs::copy(&inputs[0], &fat).map_err(|e| e.to_string())?;
        } else {
            let mut lipo = Command::new("lipo");
            lipo.arg("-create").args(&inputs).arg("-output").arg(&fat);
            run(&mut lipo, "lipo")?;
        }
        slices.push(fat);
    }

    let xcf = out_root.join(format!("{}.xcframework", pascal_lib(&lib_name)));
    if xcf.exists() {
        std::fs::remove_dir_all(&xcf)
            .map_err(|e| format!("cannot replace {}: {e}", xcf.display()))?;
    }
    std::fs::create_dir_all(&out_root).map_err(|e| e.to_string())?;
    // SwiftPM only links a static-library xcframework that ships a headers directory.
    // The real C declarations live in CarapaceFFI, so this header is deliberately empty.
    let headers = work.join("headers");
    std::fs::create_dir_all(&headers).map_err(|e| e.to_string())?;
    std::fs::write(
        headers.join(format!("{lib_name}.h")),
        "/* Intentionally empty: the Carapace C ABI is declared by the CarapaceFFI package target. */\n",
    )
    .map_err(|e| e.to_string())?;
    let mut cmd = Command::new("xcodebuild");
    cmd.arg("-create-xcframework");
    for s in &slices {
        cmd.arg("-library").arg(s).arg("-headers").arg(&headers);
    }
    cmd.arg("-output").arg(&xcf);
    run(&mut cmd, "xcodebuild -create-xcframework")?;
    println!("wrote {}", xcf.display());
    print_next_steps(&xcf, &pkg);
    Ok(())
}

fn pascal_lib(name: &str) -> String {
    name.split(['_', '-'])
        .map(|p| {
            let mut c = p.chars();
            c.next()
                .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                .unwrap_or_default()
        })
        .collect()
}

fn print_next_steps(xcf: &Path, pkg: &str) {
    println!(
        "\nIn your Package.swift:\n  .binaryTarget(name: \"{name}\", path: \"{path}\")\n  .executableTarget(name: \"App\", dependencies: [\n      .product(name: \"CarapaceKit\", package: \"carapace\"), .product(name: \"CarapaceFFI\", package: \"carapace\"), \"{name}\"])\n(package `{pkg}`)",
        name = xcf.file_stem().and_then(|s| s.to_str()).unwrap_or("Core"),
        path = xcf.display(),
    );
}

fn cargo_metadata(args: &Args) -> Result<Value, String> {
    let mut cmd = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cmd.args(["metadata", "--format-version", "1", "--no-deps"]);
    let out = cmd
        .output()
        .map_err(|e| format!("cannot run cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let _ = args;
    serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())
}

/// Resolve the package and its staticlib name from cargo metadata.
fn staticlib_of(meta: &Value, args: &Args) -> Result<(String, String), String> {
    let packages = meta["packages"]
        .as_array()
        .ok_or("cargo metadata has no packages")?;
    let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
    let pick = packages.iter().find(|p| match &args.package {
        Some(n) => p["name"] == n.as_str(),
        None => p["manifest_path"]
            .as_str()
            .is_some_and(|m| Path::new(m).parent() == Some(cwd.as_path())),
    });
    let pkg = pick.ok_or("no package selected; pass -p <package> or run inside the core crate")?;
    let lib = pkg["targets"]
        .as_array()
        .and_then(|ts| ts.iter().find(|t| t["crate_types"].as_array().is_some_and(|c| c.iter().any(|x| x == "staticlib"))))
        .ok_or_else(|| format!("{} has no staticlib; add crate-type = [\"lib\", \"staticlib\", \"cdylib\"] to [lib]", pkg["name"]))?;
    let name = lib["name"].as_str().unwrap_or_default().replace('-', "_");
    Ok((pkg["name"].as_str().unwrap_or_default().to_string(), name))
}
