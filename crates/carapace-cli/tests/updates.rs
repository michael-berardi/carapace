//! The real binary against a local fake release record and a fake `cargo`. No network.
#![cfg(unix)]

use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

const CURRENT: &str = env!("CARGO_PKG_VERSION");
const NEWER: &str = r#"{"tag_name": "v9.9.9", "draft": false, "prerelease": false,
  "body": "Faster startup\nFixed a rare lock race"}"#;

/// Records its arguments; creates build output in `--target-dir`; exits `$FAKE_CARGO_EXIT`.
const FAKE_CARGO: &str = r#"#!/bin/sh
echo "$*" >> "$FAKE_CARGO_ARGS"
while [ "$#" -gt 0 ]; do
  if [ "$1" = --target-dir ]; then mkdir -p "$2/release" && : > "$2/release/cargo-carapace"; fi
  shift
done
exit "${FAKE_CARGO_EXIT:-0}"
"#;

/// A temp folder holding a fake `cargo install` root with a copy of the binary, a fake
/// cargo, a private TMPDIR and cache, and a local release server that records requests.
struct Sandbox {
    root: PathBuf,
    api: String,
    requests: Arc<Mutex<Vec<String>>>,
}

impl Sandbox {
    fn new(label: &str, release: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "carapace-cli-updates-{}-{nanos}-{label}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("install/bin")).unwrap();
        fs::create_dir_all(root.join("tmp")).unwrap();
        let record = r#"{"installs": {"cargo-carapace 0.2.0 (git+https://example.invalid)": {}}}"#;
        fs::write(root.join("install/.crates2.json"), record).unwrap();
        fs::copy(
            env!("CARGO_BIN_EXE_cargo-carapace"),
            root.join("install/bin/cargo-carapace"),
        )
        .unwrap();
        fs::write(root.join("fake-cargo"), FAKE_CARGO).unwrap();
        fs::set_permissions(root.join("fake-cargo"), fs::Permissions::from_mode(0o755)).unwrap();
        let (api, requests) = serve(release.to_string());
        Sandbox {
            root,
            api,
            requests,
        }
    }

    fn installed(&self) -> PathBuf {
        self.root.join("install/bin/cargo-carapace")
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_bin(&self.installed(), args, &[])
    }

    fn run_bin(&self, bin: &PathBuf, args: &[&str], env: &[(&str, &str)]) -> Output {
        let mut command = Command::new(bin);
        command
            .args(args)
            .current_dir(&self.root)
            .env("CARAPACE_UPDATE_API", &self.api)
            .env("XDG_CACHE_HOME", self.root.join("cache"))
            .env("TMPDIR", self.root.join("tmp"))
            .env("CARGO", self.root.join("fake-cargo"))
            .env("FAKE_CARGO_ARGS", self.root.join("cargo-args"));
        for name in [
            "CI",
            "CARAPACE_AGENT",
            "CARAPACE_NO_UPDATE_CHECK",
            "CLAUDECODE",
            "CODEX_SANDBOX",
            "GEMINI_CLI",
            "FAKE_CARGO_EXIT",
        ] {
            command.env_remove(name);
        }
        command.envs(env.iter().copied());
        command.output().unwrap()
    }

    fn state(&self) -> Value {
        let path = self.root.join("cache/cargo-carapace/state.json");
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    fn cargo_calls(&self) -> Vec<String> {
        let text = fs::read_to_string(self.root.join("cargo-args")).unwrap_or_default();
        text.lines().map(str::to_string).collect()
    }

    fn temp_leftovers(&self) -> Vec<String> {
        fs::read_dir(self.root.join("tmp"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn serve(body: String) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/releases/latest", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(vec![]));
    let seen = requests.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0u8; 4096];
            let n = stream.read(&mut request).unwrap_or(0);
            seen.lock()
                .unwrap()
                .push(String::from_utf8_lossy(&request[..n]).into_owned());
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (url, requests)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn check_exit_codes_and_an_anonymous_request() {
    let newer = Sandbox::new("newer", NEWER);
    let out = newer.run(&["update", "--check"]);
    assert_eq!(out.status.code(), Some(10), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    assert!(stdout.contains(&format!("{CURRENT} → 9.9.9")), "{stdout}");
    assert!(stdout.contains("Faster startup"), "{stdout}");
    assert_eq!(newer.state()["latest"], "v9.9.9");

    // One plain GET: fixed User-Agent, no credentials, cookies, version or query string.
    let requests = newer.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 1);
    let mut lines = requests[0].lines();
    assert_eq!(lines.next(), Some("GET /releases/latest HTTP/1.1"));
    let mut headers: Vec<String> = lines
        .take_while(|l| !l.is_empty())
        .map(|l| l.to_ascii_lowercase())
        .collect();
    headers.sort();
    assert_eq!(headers.len(), 3, "{headers:?}");
    assert!(headers[0].starts_with("accept: application/vnd.github+json"));
    assert!(headers[1].starts_with("host: 127.0.0.1:"));
    assert_eq!(headers[2], "user-agent: cargo-carapace-update-check");

    let current = format!(r#"{{"tag_name": "v{CURRENT}", "body": ""}}"#);
    let same = Sandbox::new("current", &current);
    let out = same.run(&["update", "--check"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(text(&out.stdout).contains("is up to date"));

    let pre = Sandbox::new(
        "prerelease",
        r#"{"tag_name": "v9.9.9", "prerelease": true}"#,
    );
    assert_eq!(pre.run(&["update", "--check"]).status.code(), Some(0));

    let down = Sandbox::new("down", NEWER);
    let env = [("CARAPACE_UPDATE_API", "http://127.0.0.1:1/releases/latest")];
    let out = down.run_bin(&down.installed(), &["update", "--check"], &env);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("update check failed"));
    assert!(!down.state()["error"].as_str().unwrap().is_empty());
}

#[test]
fn skip_auto_checks_and_silence_without_a_terminal() {
    let sandbox = Sandbox::new("settings", NEWER);
    let out = sandbox.run(&["update", "--skip"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert_eq!(sandbox.state()["skipped"], "9.9.9");

    assert_eq!(sandbox.state()["auto"], false, "off by default");
    sandbox.run(&["update", "--auto", "on"]);
    assert_eq!(sandbox.state()["auto"], true);
    let status = text(&sandbox.run(&["update", "--auto", "status"]).stdout);
    assert!(status.contains("automatic updates: on"), "{status}");
    let log = sandbox.root.join("cache/cargo-carapace/update.log");
    fs::write(&log, "1 installed v9.9.9\n").unwrap();
    sandbox.run(&["update", "--auto", "off"]);
    assert_eq!(sandbox.state()["auto"], false);
    assert!(
        !log.exists(),
        "--auto off removes the background-install log"
    );
    assert_eq!(
        sandbox.run(&["update", "--auto", "maybe"]).status.code(),
        Some(1)
    );

    sandbox.run(&["update", "--checks", "off"]);
    assert_eq!(sandbox.state()["checks_off"], true);
    sandbox.run(&["update", "--checks", "on"]);
    assert_eq!(sandbox.state()["checks_off"], false);

    // Piped output: only the command's own line, and no request at all.
    let before = sandbox.requests.lock().unwrap().len();
    let out = sandbox.run(&["--version"]);
    assert_eq!(text(&out.stdout), format!("cargo carapace {CURRENT}\n"));
    assert_eq!(text(&out.stderr), "");
    assert_eq!(sandbox.requests.lock().unwrap().len(), before);

    let out = sandbox.run(&["gen", "--check"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("only work with `update`"));

    assert_eq!(sandbox.run(&["update", "--clean"]).status.code(), Some(0));
    assert!(!sandbox.root.join("cache/cargo-carapace").exists());
}

#[test]
fn update_installs_the_tag_with_cargo_and_rollback_reinstalls_the_previous_one() {
    let sandbox = Sandbox::new("install", NEWER);
    let out = sandbox.run(&["update"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    assert!(
        stdout.contains("9.9.9 installed; it applies on the next run"),
        "{stdout}"
    );

    let calls = sandbox.cargo_calls();
    assert_eq!(calls.len(), 1);
    let root = sandbox.root.join("install");
    let expected = format!(
        "install --git https://github.com/michael-berardi/carapace --tag v9.9.9 --locked --root {}",
        root.display()
    );
    assert!(calls[0].starts_with(&expected), "{}", calls[0]);
    assert!(calls[0].ends_with(" cargo-carapace"), "{}", calls[0]);
    assert_eq!(sandbox.temp_leftovers(), Vec::<String>::new());
    assert_eq!(sandbox.state()["previous"], CURRENT);
    assert_eq!(
        sandbox.state()["auto_asked"],
        false,
        "no terminal, no question"
    );

    fs::write(
        sandbox.root.join("cache/cargo-carapace/state.json"),
        r#"{"previous": "0.1.1"}"#,
    )
    .unwrap();
    let out = sandbox.run(&["update", "--rollback"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let calls = sandbox.cargo_calls();
    assert!(calls[1].contains("--tag v0.1.1 --locked"), "{}", calls[1]);
    assert_eq!(sandbox.state()["skipped"], CURRENT);
    assert_eq!(sandbox.temp_leftovers(), Vec::<String>::new());
}

#[test]
fn failed_install_keeps_the_current_binary_and_leaves_no_temp_files() {
    let sandbox = Sandbox::new("failure", NEWER);
    let before = fs::read(sandbox.installed()).unwrap();
    let out = sandbox.run_bin(
        &sandbox.installed(),
        &["update"],
        &[("FAKE_CARGO_EXIT", "101")],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(
        text(&out.stderr).contains("cargo install failed"),
        "{}",
        text(&out.stderr)
    );
    assert_eq!(fs::read(sandbox.installed()).unwrap(), before);
    assert_eq!(sandbox.temp_leftovers(), Vec::<String>::new());
    assert!(sandbox.state()["install_error"].is_string());
    assert!(sandbox.state()["previous"].is_null());

    let check = text(&sandbox.run(&["update", "--check"]).stdout);
    assert!(check.contains("last install failed"), "{check}");
}

#[test]
fn a_binary_cargo_does_not_own_is_never_replaced() {
    let sandbox = Sandbox::new("loose", NEWER);
    let loose = PathBuf::from(env!("CARGO_BIN_EXE_cargo-carapace"));
    let out = sandbox.run_bin(&loose, &["update"], &[]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = text(&out.stderr);
    assert!(
        stderr.contains("not installed with `cargo install`"),
        "{stderr}"
    );
    assert!(
        stderr.contains("--tag v9.9.9 --locked cargo-carapace"),
        "{stderr}"
    );
    assert!(sandbox.cargo_calls().is_empty());
}
