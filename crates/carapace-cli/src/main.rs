//! `cargo carapace`: generate typed shells from a core and package it for Apple platforms.

mod apple;
mod schema_source;
mod updates;

use std::path::PathBuf;
use std::process::ExitCode;

const HELP: &str = "\
cargo carapace - generate shells for a Carapace core

USAGE
    cargo carapace gen [swift|ts|all] [-p <package>] [--schema <file>] [--out <dir>]
    cargo carapace schema [-p <package>]
    cargo carapace build apple [-p <package>] [--debug] [--ios] [--out <dir>]
    cargo carapace doctor
    cargo carapace update [--check | --skip | --auto on|off|status | --checks on|off | --rollback | --clean]

COMMANDS
    gen       Write typed Swift and/or TypeScript bindings (default: all) to --out (default ./generated).
              The schema comes from the core's built library, or from --schema <file>.
    schema    Print the core's JSON Schema bundle.
    build     Build a universal xcframework (macOS arm64+x86_64, plus iOS with --ios) for SwiftPM.
    doctor    Check the toolchain needed for each shell.
    update    Install the latest release with `cargo install`. See docs/updates.md.

OPTIONS
    -p, --package <name>   Cargo package that exports the core (default: the package in the current directory).
    --schema <file>        Read the schema bundle from a file instead of the built library.
    --out <dir>            Output directory.
    --debug                Build in debug mode (build apple defaults to release).
    --ios                  Also build iOS device and simulator slices.
    --check                update: report only (exit 0 current, 10 update available, 1 check failed).
    --skip                 update: no notice for this version until a newer one.
    --auto <on|off|status> update: install future updates automatically (off by default).
    --checks <on|off>      update: turn the daily release check on or off.
    --rollback             update: reinstall the version that ran before the last update.
    --clean                update: remove the update state and log (before uninstalling).
    -V, --version          Show the version.
    -h, --help             Show this help.
";

pub struct Args {
    pub package: Option<String>,
    pub schema: Option<PathBuf>,
    pub out: Option<PathBuf>,
    pub debug: bool,
    pub ios: bool,
    pub update: updates::Flags,
    pub rest: Vec<String>,
}

fn parse_args(raw: Vec<String>) -> Result<Args, String> {
    let mut a = Args {
        package: None,
        schema: None,
        out: None,
        debug: false,
        ios: false,
        update: updates::Flags::default(),
        rest: vec![],
    };
    let mut it = raw.into_iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "-p" | "--package" => a.package = Some(value("--package")?),
            "--schema" => a.schema = Some(value("--schema")?.into()),
            "--out" => a.out = Some(value("--out")?.into()),
            "--debug" => a.debug = true,
            "--ios" => a.ios = true,
            "--check" => a.update.check = true,
            "--skip" => a.update.skip = true,
            "--auto" => a.update.auto = Some(value("--auto")?),
            "--checks" => a.update.checks = Some(value("--checks")?),
            "--rollback" => a.update.rollback = true,
            "--clean" => a.update.clean = true,
            "-h" | "--help" => a.rest.push("help".into()),
            "-V" | "--version" => a.rest.push("version".into()),
            s if s.starts_with('-') => return Err(format!("unknown option {s}\n\n{HELP}")),
            _ => a.rest.push(arg),
        }
    }
    let command = a.rest.first().map(String::as_str);
    if a.update != updates::Flags::default() && command != Some("update") {
        return Err(format!(
            "--check, --skip, --auto, --checks, --rollback and --clean only work with `update`\n\n{HELP}"
        ));
    }
    Ok(a)
}

fn run() -> Result<u8, String> {
    let mut raw: Vec<String> = std::env::args().skip(1).collect();
    // Invoked as `cargo carapace ...`: cargo passes "carapace" first.
    if raw.first().map(String::as_str) == Some("carapace") {
        raw.remove(0);
    }
    let args = parse_args(raw)?;
    let result = match args.rest.first().map(String::as_str) {
        None | Some("help") => {
            print!("{HELP}");
            Ok(())
        }
        Some("version") => {
            println!("cargo carapace {}", updates::CURRENT_VERSION);
            Ok(())
        }
        Some("update") => return updates::run(&args.update),
        Some(updates::BACKGROUND_COMMAND) => return updates::background_install(&args.rest[1..]),
        Some("schema") => {
            let text = schema_source::load(&args)?;
            println!("{text}");
            Ok(())
        }
        Some("gen") => generate(&args),
        Some("build") => match args.rest.get(1).map(String::as_str) {
            Some("apple") => apple::build(&args),
            _ => Err(format!(
                "build needs a target: `cargo carapace build apple`\n\n{HELP}"
            )),
        },
        Some("doctor") => doctor(),
        Some(other) => Err(format!("unknown command {other:?}\n\n{HELP}")),
    };
    result?;
    // After the command's own output: at most one short notice on stderr.
    updates::after_command();
    Ok(0)
}

fn generate(args: &Args) -> Result<(), String> {
    let which = args.rest.get(1).map(String::as_str).unwrap_or("all");
    if !matches!(which, "swift" | "ts" | "all") {
        return Err(format!(
            "gen target must be swift, ts or all, not {which:?}"
        ));
    }
    let text = schema_source::load(args)?;
    let files =
        carapace_codegen::generate(&text).map_err(|e| format!("cannot generate bindings: {e}"))?;
    let out = args.out.clone().unwrap_or_else(|| "generated".into());
    std::fs::create_dir_all(&out).map_err(|e| format!("cannot create {}: {e}", out.display()))?;
    let write = |name: String, body: &str| -> Result<(), String> {
        let path = out.join(name);
        std::fs::write(&path, body).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        println!("wrote {}", path.display());
        Ok(())
    };
    if which != "ts" {
        write(format!("{}.swift", files.app), &files.swift)?;
    }
    if which != "swift" {
        write(format!("{}.ts", files.app), &files.typescript)?;
    }
    Ok(())
}

fn doctor() -> Result<(), String> {
    let checks: [(&str, &[&str], &str); 7] = [
        ("rustc", &["--version"], "required for every core"),
        ("cargo", &["--version"], "required for every core"),
        ("swift", &["--version"], "SwiftUI shell on macOS/iOS"),
        (
            "xcodebuild",
            &["-version"],
            "xcframework packaging (build apple)",
        ),
        (
            "lipo",
            &["-info", "/bin/ls"],
            "universal macOS libraries (build apple)",
        ),
        (
            "node",
            &["--version"],
            "TypeScript shell (Tauri, Electron, Node)",
        ),
        (
            "python3",
            &["--version"],
            "ABI smoke test (any language that can call C)",
        ),
    ];
    for (tool, args, why) in checks {
        match std::process::Command::new(tool).args(args).output() {
            Ok(o) if o.status.success() => {
                let line = String::from_utf8_lossy(if o.stdout.is_empty() {
                    &o.stderr
                } else {
                    &o.stdout
                });
                println!(
                    "ok       {tool:<11} {:<40} {why}",
                    line.lines().next().unwrap_or("").trim()
                );
            }
            _ => println!("missing  {tool:<11} {:<40} {why}", ""),
        }
    }
    let installed = std::process::Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output();
    if let Ok(o) = installed {
        let list = String::from_utf8_lossy(&o.stdout);
        for t in [
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
            "aarch64-apple-ios",
            "aarch64-apple-ios-sim",
            "x86_64-apple-ios",
        ] {
            let state = if list.lines().any(|l| l.trim() == t) {
                "ok      "
            } else {
                "missing "
            };
            println!("{state} rust target {t} (`rustup target add {t}`)");
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
