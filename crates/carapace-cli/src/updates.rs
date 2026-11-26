//! Release notices and updates for `cargo carapace` (see docs/updates.md).
//!
//! Only this CLI checks for releases; the library crates and the Swift and npm clients make
//! no network requests. The check is one anonymous GET of the public latest-release record,
//! at most once a day, after a command has finished on an interactive terminal.

use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Hidden command that runs an opted-in automatic install in a detached process.
pub const BACKGROUND_COMMAND: &str = "__background-update";
const TOOL: &str = "cargo carapace";
const PACKAGE: &str = "cargo-carapace";
const USER_AGENT: &str = "cargo-carapace-update-check";
const DAY: u64 = 24 * 60 * 60;
const MAX_LOG_BYTES: usize = 256 * 1024;
const OPT_OUT: &str = "CARAPACE_NO_UPDATE_CHECK";
/// Set when a command runs for an agent (headless or print mode); the CLI then stays silent.
const AGENT_VARS: [&str; 4] = [
    "CARAPACE_AGENT",
    "CLAUDECODE",
    "CODEX_SANDBOX",
    "GEMINI_CLI",
];
/// For tests: read the release record from this URL instead of GitHub.
const API_OVERRIDE: &str = "CARAPACE_UPDATE_API";

/// `update` options from the command line.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Flags {
    pub check: bool,
    pub skip: bool,
    pub auto: Option<String>,
    pub checks: Option<String>,
    pub rollback: bool,
    pub clean: bool,
}

/// The only file the check writes. Missing or damaged state reads as the defaults
/// (daily check on, automatic installs off).
#[derive(Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct State {
    /// Last check: when, the latest stable tag, or why it failed.
    pub checked_at: u64,
    pub latest: Option<String>,
    pub error: Option<String>,
    /// Notices: the skipped version, and the version last shown and when.
    pub skipped: Option<String>,
    pub shown: Option<String>,
    pub shown_at: u64,
    pub checks_off: bool,
    pub auto: bool,
    pub auto_asked: bool,
    /// The version a background install was started for, and the last install failure.
    pub auto_tried: Option<String>,
    pub install_error: Option<String>,
    /// The version that ran before the last update, for `update --rollback`.
    pub previous: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub version: String,
    pub notes: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Check {
    Current,
    Available(Release),
    Failed(String),
}

/// What to print after a command, and the tag to install in the background when opted in.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Notice {
    pub lines: Vec<String>,
    pub install: Option<String>,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `$XDG_CACHE_HOME/cargo-carapace`, else the platform cache dir.
pub fn cache_dir() -> Result<PathBuf, String> {
    let var = |name: &str| {
        env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let base = var("XDG_CACHE_HOME").or_else(|| {
        if cfg!(windows) {
            var("LOCALAPPDATA")
        } else if cfg!(target_os = "macos") {
            var("HOME").map(|home| home.join("Library/Caches"))
        } else {
            var("HOME").map(|home| home.join(".cache"))
        }
    });
    Ok(base
        .ok_or("no cache directory; set XDG_CACHE_HOME")?
        .join(PACKAGE))
}

fn state_path(dir: &Path) -> PathBuf {
    dir.join("state.json")
}

fn log_path(dir: &Path) -> PathBuf {
    dir.join("update.log")
}

/// Opens a file readable only by its owner (0600 on Unix), creating it if needed.
fn open_private(path: &Path, append: bool) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true);
    if append {
        options.append(true);
    } else {
        options.write(true).truncate(true);
    }
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

pub fn load(dir: &Path) -> State {
    fs::read(state_path(dir))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Atomic: writes a private temp file next to the state, then renames it over.
pub fn save(dir: &Path, state: &State) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let tmp = dir.join(format!("state.json.{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec(state).map_err(|e| e.to_string())?;
    let written = open_private(&tmp, false)
        .and_then(|mut file| file.write_all(&bytes))
        .and_then(|()| fs::rename(&tmp, state_path(dir)));
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written.map_err(|e| format!("cannot save {}: {e}", state_path(dir).display()))
}

fn edit(dir: &Path, change: impl FnOnce(&mut State)) -> Result<(), String> {
    let mut state = load(dir);
    change(&mut state);
    save(dir, &state)
}

/// Stable `x.y.z`, with or without a leading `v`. Prereleases, build metadata and anything
/// else are not versions here.
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let text = text.strip_prefix('v').unwrap_or(text);
    let mut parts = text.split('.').map(|part| {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            None
        } else {
            part.parse::<u64>().ok()
        }
    });
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// Only a strictly newer stable version counts; anything unparseable is not newer.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    matches!(
        (parse_version(candidate), parse_version(current)),
        (Some(a), Some(b)) if a > b
    )
}

fn version_of(tag: &str) -> String {
    parse_version(tag).map_or_else(|| tag.to_string(), |(a, b, c)| format!("{a}.{b}.{c}"))
}

/// The repository declared in this package's own metadata.
fn repository() -> &'static str {
    env!("CARGO_PKG_REPOSITORY").trim_end_matches('/')
}

fn api_url() -> Result<String, String> {
    if let Some(url) = env::var(API_OVERRIDE).ok().filter(|u| !u.trim().is_empty()) {
        return Ok(url);
    }
    let path = repository()
        .strip_prefix("https://github.com/")
        .ok_or("package metadata names no GitHub repository")?;
    Ok(format!(
        "https://api.github.com/repos/{}/releases/latest",
        path.trim_end_matches(".git")
    ))
}

/// One anonymous GET with a fixed User-Agent: no token, cookie, version or machine data.
/// `--disable` stops a user's `.curlrc` from adding headers.
fn fetch(url: &str) -> Result<Option<Release>, String> {
    let out = Command::new("curl")
        .args([
            "--disable",
            "--silent",
            "--show-error",
            "--fail",
            "--location",
        ])
        .args(["--connect-timeout", "2", "--max-time", "3"])
        .args(["--user-agent", USER_AGENT])
        .args(["--header", "Accept: application/vnd.github+json", url])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run curl: {e}"))?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if why.is_empty() {
            format!("curl {}", out.status)
        } else {
            why
        });
    }
    parse_release(&out.stdout)
}

/// Drafts, prereleases and tags that are not `x.y.z` are never offered.
pub fn parse_release(body: &[u8]) -> Result<Option<Release>, String> {
    let value: Value =
        serde_json::from_slice(body).map_err(|e| format!("unexpected release record: {e}"))?;
    if value["draft"].as_bool() == Some(true) || value["prerelease"].as_bool() == Some(true) {
        return Ok(None);
    }
    let tag = value["tag_name"].as_str().unwrap_or_default().trim();
    if parse_version(tag).is_none() {
        return Ok(None);
    }
    let mut notes: Vec<String> = value["body"]
        .as_str()
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("```"))
        .take(3)
        .map(|line| match line.char_indices().nth(100) {
            Some((cut, _)) => format!("{}…", &line[..cut]),
            None => line.to_string(),
        })
        .collect();
    if let Some(url) = value["html_url"].as_str() {
        notes.push(url.to_string());
    }
    Ok(Some(Release {
        tag: tag.to_string(),
        version: version_of(tag),
        notes: notes.join("\n"),
    }))
}

fn cached(state: &State) -> Check {
    match (&state.error, &state.latest) {
        (Some(cause), _) => Check::Failed(cause.clone()),
        (None, Some(tag)) if is_newer(tag, CURRENT_VERSION) => Check::Available(Release {
            tag: tag.clone(),
            version: version_of(tag),
            notes: String::new(),
        }),
        _ => Check::Current,
    }
}

/// Automatic checks reuse a result younger than a day, a failure included, so nothing
/// retries in a loop. Explicit commands pass `force`.
pub fn check_with(
    dir: &Path,
    at: u64,
    force: bool,
    fetch: impl FnOnce() -> Result<Option<Release>, String>,
) -> Check {
    let mut state = load(dir);
    if !force && state.checked_at != 0 && at.saturating_sub(state.checked_at) < DAY {
        return cached(&state);
    }
    let result = fetch();
    state.checked_at = at;
    match &result {
        Ok(release) => {
            state.latest = release.as_ref().map(|r| r.tag.clone());
            state.error = None;
        }
        Err(cause) => state.error = Some(cause.clone()),
    }
    let _ = save(dir, &state);
    match result {
        Ok(Some(release)) if is_newer(&release.version, CURRENT_VERSION) => {
            Check::Available(release)
        }
        Ok(_) => Check::Current,
        Err(cause) => Check::Failed(cause),
    }
}

/// Automatic checks need an interactive terminal and stay off in CI, under agent hosts and
/// when opted out (`CARAPACE_NO_UPDATE_CHECK=1` or `update --checks off`).
pub fn silenced(state: &State, interactive: bool, var: impl Fn(&str) -> Option<String>) -> bool {
    let set = |name: &str| var(name).is_some_and(|v| !v.is_empty() && v != "0" && v != "false");
    !interactive
        || state.checks_off
        || set("CI")
        || set(OPT_OUT)
        || AGENT_VARS.iter().any(|name| set(name))
}

pub fn prompt_lines(version: &str, auto: bool) -> Vec<String> {
    let skip = format!("skip: {TOOL} update --skip");
    vec![
        format!("{TOOL} {version} available (you have {CURRENT_VERSION}) · run: {TOOL} update"),
        if auto {
            skip
        } else {
            format!("turn on automatic updates: {TOOL} update --auto on · {skip}")
        },
    ]
}

/// At most one notice per version per day; none for a skipped version. With automatic
/// updates on, a new version is installed in the background once, and a failed install
/// falls back to the normal notice.
pub fn notice_with(
    dir: &Path,
    at: u64,
    interactive: bool,
    var: impl Fn(&str) -> Option<String>,
    fetch: impl FnOnce() -> Result<Option<Release>, String>,
) -> Notice {
    if silenced(&load(dir), interactive, var) {
        return Notice::default();
    }
    let Check::Available(release) = check_with(dir, at, false, fetch) else {
        return Notice::default();
    };
    let version = &release.version;
    let mut state = load(dir);
    if state.skipped.as_ref() == Some(version) {
        return Notice::default();
    }
    let mut notice = if state.auto && state.auto_tried.as_ref() != Some(version) {
        state.auto_tried = Some(version.clone());
        Notice {
            lines: vec![format!(
                "{TOOL} {version} available (you have {CURRENT_VERSION}) · installing in the background"
            )],
            install: Some(release.tag.clone()),
        }
    } else if state.shown.as_ref() == Some(version) && at.saturating_sub(state.shown_at) < DAY {
        return Notice::default();
    } else {
        Notice {
            lines: prompt_lines(version, state.auto),
            install: None,
        }
    };
    state.shown = Some(version.clone());
    state.shown_at = at;
    if save(dir, &state).is_err() {
        // Without a record of the attempt, an install could repeat on every run.
        notice.install = None;
    }
    notice
}

/// Runs after a successful command. The check gets at most four seconds on a helper
/// thread; a slow or failed check prints nothing.
pub fn after_command() {
    if !(io::stdout().is_terminal() && io::stderr().is_terminal()) {
        return;
    }
    let Ok(dir) = cache_dir() else {
        return;
    };
    let (tx, rx) = mpsc::channel();
    let check_dir = dir.clone();
    thread::spawn(move || {
        let var = |name: &str| env::var(name).ok();
        let notice = notice_with(&check_dir, now(), true, var, || fetch(&api_url()?));
        let _ = tx.send(notice);
    });
    let Ok(notice) = rx.recv_timeout(Duration::from_secs(4)) else {
        return;
    };
    let dim = env::var_os("NO_COLOR").map_or(true, |v| v.is_empty());
    for (i, line) in notice.lines.iter().enumerate() {
        if i > 0 && dim {
            eprintln!("\x1b[2m{line}\x1b[0m");
        } else {
            eprintln!("{line}");
        }
    }
    if let Some(tag) = notice.install {
        if let Err(cause) = spawn_background(&dir, &tag) {
            log(&dir, &cause);
            let _ = edit(&dir, |s| s.install_error = Some(cause));
        }
    }
}

/// `cargo install` puts the binary in `<root>/bin` and records the package in
/// `<root>/.crates2.json` (older Cargo: `.crates.toml`). Anything else is not Cargo's to replace.
pub fn cargo_root(exe: &Path) -> Option<PathBuf> {
    let bin = exe.parent()?;
    let root = bin.parent()?;
    let entry = format!("\"{PACKAGE} ");
    let recorded = [".crates2.json", ".crates.toml"]
        .iter()
        .any(|name| fs::read_to_string(root.join(name)).is_ok_and(|text| text.contains(&entry)));
    (bin.file_name()? == "bin" && recorded).then(|| root.to_path_buf())
}

fn install_hint(tag: &str) -> String {
    format!(
        "cargo install --git {} --tag {tag} --locked {PACKAGE}",
        repository()
    )
}

fn not_cargo_installed(tag: &str) -> String {
    format!(
        "this binary was not installed with `cargo install`; update it with: {}",
        install_hint(tag)
    )
}

/// The CLI is a `cargo install` from a release tag, so Cargo owns the binary and updating
/// re-runs it for the new tag. Cargo checks the locked dependency checksums and replaces the
/// binary only after a successful build, so a failure leaves the current version working.
/// The build uses its own temp dir, removed afterwards whatever the outcome.
pub fn install_with(
    dir: &Path,
    exe: &Path,
    tag: &str,
    quiet: bool,
    run: impl FnOnce(&[String]) -> Result<(), String>,
) -> Result<(), String> {
    if parse_version(tag).is_none() {
        return Err(format!("{tag:?} is not a release tag"));
    }
    let Some(root) = cargo_root(exe) else {
        return Err(not_cargo_installed(tag));
    };
    let build = env::temp_dir().join(format!("{PACKAGE}-update-{}-{}", std::process::id(), now()));
    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let (root, target) = (root.to_string_lossy(), build.to_string_lossy());
    let mut argv: Vec<String> = [
        cargo.as_str(),
        "install",
        "--git",
        repository(),
        "--tag",
        tag,
        "--locked",
        "--root",
        &root,
        "--target-dir",
        &target,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if quiet {
        argv.push("--quiet".into());
    }
    argv.push(PACKAGE.into());
    let result = run(&argv);
    let _ = fs::remove_dir_all(&build);
    let _ = edit(dir, |s| match &result {
        Ok(()) => {
            s.previous = Some(CURRENT_VERSION.into());
            s.install_error = None;
        }
        Err(cause) => s.install_error = Some(cause.clone()),
    });
    result
}

fn run_cargo(argv: &[String]) -> Result<(), String> {
    let status = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(Stdio::null())
        .status()
        .map_err(|e| format!("cannot run {}: {e}", argv[0]))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("cargo install failed ({status})"))
    }
}

fn install(dir: &Path, tag: &str, quiet: bool) -> Result<(), String> {
    let exe = env::current_exe().map_err(|e| format!("cannot locate this binary: {e}"))?;
    install_with(dir, &exe, tag, quiet, run_cargo)
}

/// Appends one line to the update log, which stays under 256 KB.
fn log(dir: &Path, line: &str) {
    let _ = fs::create_dir_all(dir);
    if let Ok(mut file) = open_private(&log_path(dir), true) {
        let _ = writeln!(file, "{} {line}", now());
    }
    trim_log(dir);
}

/// Keeps the newest half once the log passes half the bound, so one install's output
/// always fits under 256 KB.
fn trim_log(dir: &Path) {
    let path = log_path(dir);
    let keep = MAX_LOG_BYTES / 2;
    if let Ok(body) = fs::read(&path) {
        if body.len() > keep {
            if let Ok(mut file) = open_private(&path, false) {
                let _ = file.write_all(&body[body.len() - keep..]);
            }
        }
    }
}

/// Starts the opted-in install as a detached, low-priority process logging to one file.
fn spawn_background(dir: &Path, tag: &str) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    trim_log(dir);
    let log = open_private(&log_path(dir), true)
        .map_err(|e| format!("cannot open the update log: {e}"))?;
    let exe = env::current_exe().map_err(|e| format!("cannot locate this binary: {e}"))?;
    let mut command = Command::new(exe);
    command
        .args([BACKGROUND_COMMAND, tag])
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log);
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    command
        .spawn()
        .map(drop)
        .map_err(|e| format!("cannot start the background install: {e}"))
}

/// The hidden background command: install, then record the outcome for `update --check`.
pub fn background_install(args: &[String]) -> Result<u8, String> {
    let tag = args.first().ok_or("missing release tag")?;
    #[cfg(unix)]
    // SAFETY: lowers this process's own scheduling priority; no memory is involved.
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, 10);
    }
    let dir = cache_dir()?;
    match install(&dir, tag, true) {
        Ok(()) => log(
            &dir,
            &format!("installed {tag}; it applies on the next run"),
        ),
        Err(cause) => log(&dir, &format!("install of {tag} failed: {cause}")),
    }
    Ok(0)
}

/// `None` when there is no terminal to ask on. End of input counts as "no".
fn ask(question: &str, default: bool) -> Option<bool> {
    if !(io::stdin().is_terminal() && io::stderr().is_terminal()) {
        return None;
    }
    eprint!("{question} ");
    let _ = io::stderr().flush();
    let mut line = String::new();
    if !matches!(io::stdin().read_line(&mut line), Ok(n) if n > 0) {
        return Some(false);
    }
    Some(match line.trim().to_ascii_lowercase().as_str() {
        "" => default,
        "y" | "yes" => true,
        _ => false,
    })
}

/// Asked once, after a successful install and only on a terminal; either answer is kept.
pub fn offer_auto(dir: &Path, answer: impl FnOnce() -> Option<bool>) {
    let state = load(dir);
    if state.auto || state.auto_asked {
        return;
    }
    if let Some(yes) = answer() {
        let _ = edit(dir, |s| {
            s.auto = yes;
            s.auto_asked = true;
        });
        if yes {
            println!("automatic updates are on (turn off: {TOOL} update --auto off)");
        }
    }
}

fn on_off(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}

fn status(dir: &Path) {
    let state = load(dir);
    println!("automatic updates: {}", on_off(state.auto));
    println!("daily release check: {}", on_off(!state.checks_off));
    if let Some(cause) = state.install_error {
        println!("last install failed: {cause}");
    }
}

fn switch(name: &str, value: &str) -> Result<Option<bool>, String> {
    match value {
        "on" => Ok(Some(true)),
        "off" => Ok(Some(false)),
        "status" => Ok(None),
        other => Err(format!("{name} takes on, off or status, not {other:?}")),
    }
}

/// Removes everything the updater wrote: the state file, the log and their folder.
fn clean(dir: &Path) -> Result<u8, String> {
    for path in [state_path(dir), log_path(dir)] {
        match fs::remove_file(&path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => {
                return Err(format!("cannot remove {}: {e}", path.display()))
            }
            _ => {}
        }
    }
    let _ = fs::remove_dir(dir);
    println!("update state and log removed");
    Ok(0)
}

fn skip(dir: &Path) -> Result<u8, String> {
    let check = match cached(&load(dir)) {
        Check::Available(release) => Check::Available(release),
        _ => {
            let url = api_url()?;
            check_with(dir, now(), true, || fetch(&url))
        }
    };
    match check {
        Check::Available(release) => {
            edit(dir, |s| s.skipped = Some(release.version.clone()))?;
            println!(
                "{TOOL}: no notice for {} until a newer release",
                release.version
            );
            Ok(0)
        }
        Check::Current => {
            println!("{TOOL} {CURRENT_VERSION} is up to date; nothing to skip");
            Ok(0)
        }
        Check::Failed(cause) => {
            eprintln!("{TOOL}: update check failed: {cause}");
            Ok(1)
        }
    }
}

/// Reinstalls the version that ran before the last update and skips the one left behind.
fn rollback(dir: &Path) -> Result<u8, String> {
    let previous = load(dir)
        .previous
        .ok_or("no earlier version recorded; nothing to roll back to")?;
    println!("{TOOL} {CURRENT_VERSION} → {previous}");
    install(dir, &format!("v{previous}"), false)?;
    edit(dir, |s| s.skipped = Some(CURRENT_VERSION.into()))?;
    println!("{TOOL} {previous} installed; it applies on the next run");
    Ok(0)
}

pub fn run(flags: &Flags) -> Result<u8, String> {
    let dir = cache_dir()?;
    if let Some(value) = &flags.auto {
        let Some(on) = switch("--auto", value)? else {
            status(&dir);
            return Ok(0);
        };
        edit(&dir, |s| {
            s.auto = on;
            s.auto_asked = true;
            s.auto_tried = None;
        })?;
        if !on {
            // Only background installs write the log.
            let _ = fs::remove_file(log_path(&dir));
        }
        println!("automatic updates are {}", on_off(on));
        return Ok(0);
    }
    if let Some(value) = &flags.checks {
        let Some(on) = switch("--checks", value)? else {
            status(&dir);
            return Ok(0);
        };
        edit(&dir, |s| s.checks_off = !on)?;
        println!("daily release check is {}", on_off(on));
        return Ok(0);
    }
    if flags.clean {
        return clean(&dir);
    }
    if flags.rollback {
        return rollback(&dir);
    }
    if flags.skip {
        return skip(&dir);
    }
    // An explicit command is consent: it checks even without a terminal or in CI.
    let url = api_url()?;
    let release = match check_with(&dir, now(), true, || fetch(&url)) {
        Check::Failed(cause) => {
            eprintln!("{TOOL}: update check failed: {cause}");
            return Ok(1);
        }
        Check::Current => {
            println!("{TOOL} {CURRENT_VERSION} is up to date");
            return Ok(0);
        }
        Check::Available(release) => release,
    };
    println!("{TOOL} {CURRENT_VERSION} → {}", release.version);
    if !release.notes.is_empty() {
        println!("{}", release.notes);
    }
    if let Some(cause) = load(&dir).install_error {
        println!("last install failed: {cause}");
    }
    if flags.check {
        return Ok(10);
    }
    let exe = env::current_exe().map_err(|e| format!("cannot locate this binary: {e}"))?;
    if cargo_root(&exe).is_none() {
        return Err(not_cargo_installed(&release.tag));
    }
    let question = format!("Install {} now? [Y/n]", release.version);
    if ask(&question, true) == Some(false) {
        println!("not installed");
        return Ok(0);
    }
    println!("installing with: {}", install_hint(&release.tag));
    install(&dir, &release.tag, false)?;
    println!(
        "{TOOL} {} installed; it applies on the next run",
        release.version
    );
    offer_auto(&dir, || {
        ask("Install future updates automatically? [y/N]", false)
    });
    Ok(0)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = env::temp_dir().join(format!(
            "carapace-update-test-{}-{nanos}-{name}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn release(version: &str) -> Release {
        Release {
            tag: format!("v{version}"),
            version: version.to_string(),
            notes: String::new(),
        }
    }

    fn no_env(_: &str) -> Option<String> {
        None
    }

    fn offline() -> Result<Option<Release>, String> {
        panic!("no request expected")
    }

    /// A fake cargo install root: `<root>/bin/cargo-carapace` plus Cargo's metadata file.
    fn cargo_install(dir: &Path) -> PathBuf {
        fs::create_dir_all(dir.join("root/bin")).unwrap();
        let record = r#"{"installs": {"cargo-carapace 0.2.0 (git+https://example.invalid)": {}}}"#;
        fs::write(dir.join("root/.crates2.json"), record).unwrap();
        let exe = dir.join("root/bin/cargo-carapace");
        fs::write(&exe, b"current").unwrap();
        exe
    }

    fn arg_after<'a>(argv: &'a [String], flag: &str) -> &'a str {
        let i = argv.iter().position(|a| a == flag).unwrap();
        &argv[i + 1]
    }

    #[test]
    fn only_strictly_newer_stable_versions_count() {
        assert!(is_newer("0.3.0", "0.2.0"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(is_newer("0.2.10", "0.2.9"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("v0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.9", "0.2.0"));
        for garbage in [
            "", "v", "1.2", "1.2.3.4", "banana", "1..2", " 9.9.9", "9.9.x",
        ] {
            assert!(!is_newer(garbage, "0.2.0"), "{garbage:?}");
        }
        for pre in ["9.9.9-rc.1", "v9.9.9-beta", "9.9.9+build.5"] {
            assert!(!is_newer(pre, "0.2.0"), "{pre:?}");
        }
        assert!(!is_newer("9.9.9", "garbage"));
    }

    #[test]
    fn release_records_ignore_drafts_prereleases_and_odd_tags() {
        let record =
            |extra: &str| format!(r#"{{"tag_name": "v9.9.9", "body": "One\n\nTwo"{extra}}}"#);
        let stable = parse_release(record("").as_bytes()).unwrap().unwrap();
        assert_eq!(stable.version, "9.9.9");
        assert_eq!(stable.tag, "v9.9.9");
        assert_eq!(stable.notes, "One\nTwo");
        assert_eq!(
            parse_release(record(r#", "draft": true"#).as_bytes()),
            Ok(None)
        );
        assert_eq!(
            parse_release(record(r#", "prerelease": true"#).as_bytes()),
            Ok(None)
        );
        assert_eq!(parse_release(br#"{"tag_name": "nightly"}"#), Ok(None));
        assert!(parse_release(b"<html>").is_err());

        let long = format!(
            r#"{{"tag_name": "v9.9.9", "html_url": "https://example.invalid/r", "body": "{}\n```sh\na\nb\nc"}}"#,
            "x".repeat(150)
        );
        let notes = parse_release(long.as_bytes()).unwrap().unwrap().notes;
        let lines: Vec<&str> = notes.lines().collect();
        assert_eq!(lines.len(), 4, "{notes}");
        assert_eq!(lines[0].chars().count(), 101);
        assert_eq!(lines[1], "a");
        assert_eq!(lines[3], "https://example.invalid/r");
    }

    #[test]
    fn checks_are_cached_for_a_day_and_failures_keep_their_cause() {
        let dir = temp_dir("cache");
        let calls = Cell::new(0);
        let fetch = || {
            calls.set(calls.get() + 1);
            Ok(Some(release("9.9.9")))
        };
        let day0 = 1_800_000_000;
        assert_eq!(
            check_with(&dir, day0, false, fetch),
            Check::Available(release("9.9.9"))
        );
        let cached = check_with(&dir, day0 + DAY - 1, false, fetch);
        assert!(matches!(cached, Check::Available(r) if r.version == "9.9.9"));
        assert_eq!(calls.get(), 1);
        check_with(&dir, day0 + DAY, false, fetch);
        assert_eq!(calls.get(), 2);

        let failing = || Err("Could not resolve host".to_string());
        assert_eq!(
            check_with(&dir, day0 + 2 * DAY, false, failing),
            Check::Failed("Could not resolve host".into())
        );
        assert_eq!(load(&dir).error.as_deref(), Some("Could not resolve host"));
        // A failed check is not retried until a day has passed.
        assert_eq!(
            check_with(&dir, day0 + 2 * DAY + 60, false, offline),
            Check::Failed("Could not resolve host".into())
        );
        // An explicit command checks again and clears the cause.
        assert_eq!(
            check_with(&dir, day0 + 2 * DAY + 61, true, || Ok(None)),
            Check::Current
        );
        assert_eq!(load(&dir).error, None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn state_is_one_private_file_written_atomically() {
        let dir = temp_dir("state");
        check_with(&dir, 1_800_000_000, true, || Ok(Some(release("9.9.9"))));
        let names: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["state.json"]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(state_path(&dir)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        fs::write(state_path(&dir), b"{ damaged").unwrap();
        assert_eq!(load(&dir), State::default());
        assert!(!load(&dir).auto, "automatic updates are off by default");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn silent_without_a_terminal_in_ci_for_agents_and_when_opted_out() {
        let state = State::default();
        assert!(!silenced(&state, true, no_env));
        assert!(silenced(&state, false, no_env));
        for name in [
            "CI",
            OPT_OUT,
            "CARAPACE_AGENT",
            "CLAUDECODE",
            "CODEX_SANDBOX",
        ] {
            let var = |n: &str| (n == name).then(|| "1".to_string());
            assert!(silenced(&state, true, var), "{name}");
        }
        let disabled = |n: &str| (n == "CI" || n == OPT_OUT).then(|| "0".to_string());
        assert!(!silenced(&state, true, disabled));
        let off = State {
            checks_off: true,
            ..State::default()
        };
        assert!(silenced(&off, true, no_env));

        // Silenced runs never make the request.
        let dir = temp_dir("silent");
        let ci = |n: &str| (n == "CI").then(|| "true".to_string());
        assert_eq!(
            notice_with(&dir, 1, false, no_env, offline),
            Notice::default()
        );
        assert_eq!(notice_with(&dir, 1, true, ci, offline), Notice::default());
        save(&dir, &off).unwrap();
        assert_eq!(
            notice_with(&dir, 1, true, no_env, offline),
            Notice::default()
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn notice_text_frequency_and_skip() {
        assert_eq!(
            prompt_lines("9.9.9", false),
            [
                format!("cargo carapace 9.9.9 available (you have {CURRENT_VERSION}) · run: cargo carapace update"),
                "turn on automatic updates: cargo carapace update --auto on · skip: cargo carapace update --skip".to_string(),
            ]
        );
        let dir = temp_dir("notice");
        let day0 = 1_800_000_000;
        let newer = |v: &'static str| move || Ok(Some(release(v)));
        let first = notice_with(&dir, day0, true, no_env, newer("9.9.9"));
        assert_eq!(first.lines, prompt_lines("9.9.9", false));
        assert_eq!(first.install, None);
        // Once per version per day.
        assert!(notice_with(&dir, day0 + 60, true, no_env, offline)
            .lines
            .is_empty());
        assert_eq!(
            notice_with(&dir, day0 + DAY, true, no_env, newer("9.9.9"))
                .lines
                .len(),
            2
        );
        // Skipped: silent for that version, a newer one notifies again.
        edit(&dir, |s| s.skipped = Some("9.9.9".into())).unwrap();
        assert!(
            notice_with(&dir, day0 + 2 * DAY, true, no_env, newer("9.9.9"))
                .lines
                .is_empty()
        );
        let next = notice_with(&dir, day0 + 3 * DAY, true, no_env, newer("10.0.0"));
        assert_eq!(next.lines, prompt_lines("10.0.0", false));
        // Not newer: nothing.
        let same = || Ok(Some(release(CURRENT_VERSION)));
        assert!(notice_with(&dir, day0 + 4 * DAY, true, no_env, same)
            .lines
            .is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn automatic_install_starts_once_per_version_and_falls_back_to_the_notice() {
        let dir = temp_dir("auto");
        let day0 = 1_800_000_000;
        edit(&dir, |s| s.auto = true).unwrap();
        let newer = || Ok(Some(release("9.9.9")));
        let started = notice_with(&dir, day0, true, no_env, newer);
        assert_eq!(started.install.as_deref(), Some("v9.9.9"));
        assert_eq!(started.lines.len(), 1);
        assert!(started.lines[0].ends_with("installing in the background"));
        assert_eq!(
            notice_with(&dir, day0 + 60, true, no_env, offline),
            Notice::default()
        );
        // Still on the old version a day later: the install failed, so show the prompt.
        let fallback = notice_with(&dir, day0 + DAY, true, no_env, newer);
        assert_eq!(fallback.install, None);
        assert_eq!(fallback.lines, prompt_lines("9.9.9", true));
        assert_eq!(fallback.lines[1], "skip: cargo carapace update --skip");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn auto_question_is_asked_once_and_no_is_remembered() {
        let dir = temp_dir("ask");
        offer_auto(&dir, || None); // no terminal: not asked, nothing stored
        assert!(!load(&dir).auto_asked);
        offer_auto(&dir, || Some(false));
        assert!(!load(&dir).auto);
        assert!(load(&dir).auto_asked);
        offer_auto(&dir, || panic!("asked twice"));
        fs::remove_dir_all(&dir).unwrap();

        let dir = temp_dir("ask-yes");
        offer_auto(&dir, || Some(true));
        assert!(load(&dir).auto);
        offer_auto(&dir, || panic!("asked while already on"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn install_runs_cargo_for_the_tag_and_always_removes_its_build_dir() {
        let dir = temp_dir("install");
        let exe = cargo_install(&dir);
        let mut seen = vec![];
        install_with(&dir, &exe, "v9.9.9", false, |argv| {
            seen = argv.to_vec();
            let build = Path::new(arg_after(argv, "--target-dir"));
            fs::create_dir_all(build.join("release")).unwrap();
            Ok(())
        })
        .unwrap();
        assert_eq!(
            &seen[1..6],
            ["install", "--git", repository(), "--tag", "v9.9.9"]
        );
        assert!(seen.contains(&"--locked".to_string()));
        assert_eq!(Path::new(arg_after(&seen, "--root")), dir.join("root"));
        assert_eq!(seen.last().map(String::as_str), Some("cargo-carapace"));
        assert!(!Path::new(arg_after(&seen, "--target-dir")).exists());
        assert_eq!(load(&dir).previous.as_deref(), Some(CURRENT_VERSION));
        assert_eq!(load(&dir).install_error, None);

        let mut build = PathBuf::new();
        let failed = install_with(&dir, &exe, "v9.9.10", true, |argv| {
            build = PathBuf::from(arg_after(argv, "--target-dir"));
            fs::create_dir_all(build.join("release")).unwrap();
            assert!(argv.contains(&"--quiet".to_string()));
            Err("cargo install failed (exit status: 101)".into())
        });
        assert!(failed.is_err());
        assert!(!build.exists(), "build dir left behind after a failure");
        assert_eq!(
            fs::read(&exe).unwrap(),
            b"current",
            "current binary untouched"
        );
        assert_eq!(
            load(&dir).install_error.as_deref(),
            Some("cargo install failed (exit status: 101)")
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn install_refuses_odd_tags_and_binaries_cargo_does_not_own() {
        let dir = temp_dir("refuse");
        let exe = cargo_install(&dir);
        let never = |_: &[String]| -> Result<(), String> { panic!("cargo must not run") };
        assert!(install_with(&dir, &exe, "--path=/tmp", false, never).is_err());
        assert!(install_with(&dir, &exe, "v9.9.9-rc.1", false, never).is_err());
        let loose = dir.join("target/debug/cargo-carapace");
        let err = install_with(&dir, &loose, "v9.9.9", false, never).unwrap_err();
        let untracked = dir.join("other/bin/cargo-carapace");
        fs::create_dir_all(untracked.parent().unwrap()).unwrap();
        fs::write(dir.join("other/.crates2.json"), r#"{"installs": {}}"#).unwrap();
        assert!(install_with(&dir, &untracked, "v9.9.9", false, never).is_err());
        assert!(err.contains("cargo install --git"), "{err}");
        assert!(
            err.contains("--tag v9.9.9 --locked cargo-carapace"),
            "{err}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn update_log_stays_under_256_kb() {
        let dir = temp_dir("log");
        fs::write(log_path(&dir), vec![b'x'; 3 * MAX_LOG_BYTES]).unwrap();
        log(&dir, "installed v9.9.9");
        let size = fs::metadata(log_path(&dir)).unwrap().len() as usize;
        assert!(size <= MAX_LOG_BYTES, "log is {size} bytes");
        assert!(fs::read_to_string(log_path(&dir))
            .unwrap()
            .ends_with("installed v9.9.9\n"));
        for _ in 0..4000 {
            log(&dir, &"y".repeat(100));
        }
        assert!(fs::metadata(log_path(&dir)).unwrap().len() as usize <= MAX_LOG_BYTES);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn clean_removes_every_updater_file() {
        let root = temp_dir("clean");
        let dir = root.join(PACKAGE);
        save(&dir, &State::default()).unwrap();
        log(&dir, "one line");
        clean(&dir).unwrap();
        assert!(!dir.exists());
        clean(&dir).unwrap(); // nothing left to remove is fine
        fs::remove_dir_all(&root).unwrap();
    }
}
