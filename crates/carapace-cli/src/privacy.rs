//! Privacy helpers used by every command the CLI runs.
//!
//! Generated bindings and xcframework inputs can mirror a user's own types and
//! source, so the CLI creates them with owner-only permissions. Subprocesses get
//! telemetry opt-outs even when the user's own shell does not set them.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::process::Command;

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};

/// Telemetry opt-outs for local build tools the CLI invokes.
const TELEMETRY_OFF: [(&str, &str); 9] = [
    ("DO_NOT_TRACK", "1"),
    ("NEXT_TELEMETRY_DISABLED", "1"),
    ("ASTRO_TELEMETRY_DISABLED", "1"),
    ("HF_HUB_DISABLE_TELEMETRY", "1"),
    ("HOMEBREW_NO_ANALYTICS", "1"),
    ("npm_config_update_notifier", "false"),
    ("npm_config_audit", "false"),
    ("npm_config_fund", "false"),
    ("npm_config_telemetry_enabled", "false"),
];

/// Create a private directory. A directory that already exists (for example a
/// project folder passed as `--out`) keeps the permissions its owner chose.
pub(crate) fn private_dir(path: &Path) -> Result<(), String> {
    if path.is_dir() {
        return Ok(());
    }
    let created = {
        #[cfg(unix)]
        {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true).mode(0o700);
            builder.create(path)
        }
        #[cfg(not(unix))]
        {
            fs::create_dir_all(path)
        }
    };
    created.map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("cannot protect {}: {e}", path.display()))?;
    Ok(())
}

/// Write a file that only its owner can read, even if it existed with looser modes.
pub(crate) fn private_write(path: &Path, body: &str) -> Result<(), String> {
    private_write_bytes(path, body.as_bytes())
}

/// Binary-safe version used for copied static libraries.
pub(crate) fn private_write_bytes(path: &Path, body: &[u8]) -> Result<(), String> {
    let opened = {
        #[cfg(unix)]
        {
            let mut options = OpenOptions::new();
            options.write(true).create(true).truncate(true).mode(0o600);
            options.open(path)
        }
        #[cfg(not(unix))]
        {
            OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(path)
        }
    };
    let mut file = opened.map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    file.write_all(body)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("cannot protect {}: {e}", path.display()))?;
    Ok(())
}

/// Apply telemetry opt-outs to a subprocess launched by Carapace.
pub(crate) fn telemetry_off(cmd: &mut Command) {
    for (key, value) in TELEMETRY_OFF {
        cmd.env(key, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Names are assembled at run time so this check does not flag its own source.
    fn banned_terms() -> Vec<String> {
        vec![
            ["sen", "try"].concat(),
            ["post", "hog"].concat(),
            ["seg", "ment"].concat(),
            ["mix", "panel"].concat(),
            ["am", "plitude"].concat(),
            ["bugs", "nag"].concat(),
            ["hot", "jar"].concat(),
            ["data", "dog"].concat(),
            ["open", "tele", "metry"].concat(),
            ["google", "analytics"].concat(),
            ["app", "leak", "s"].concat(),
        ]
    }

    /// Broad names are checked only in dependency manifests: opt-out environment
    /// variables in scripts and CLI code legitimately spell these words.
    fn manifest_only_terms() -> Vec<String> {
        vec![["analy", "tics"].concat(), ["tele", "metry"].concat()]
    }

    fn banned_hosts() -> Vec<String> {
        vec![
            ["sen", "try.io"].concat(),
            ["post", "hog.com"].concat(),
            ["seg", "ment.io"].concat(),
            ["am", "plitude.com"].concat(),
            ["mix", "panel.com"].concat(),
            ["bugs", "nag.com"].concat(),
            ["hot", "jar.com"].concat(),
            ["data", "dog", "hq.com"].concat(),
            ["google", "apis.com"].concat(),
            ["googletag", "manager.com"].concat(),
            ["google-analytics", ".com"].concat(),
            ["js", "delivr.net"].concat(),
            ["un", "pkg.com"].concat(),
            ["cdn", "js.cloudflare.com"].concat(),
        ]
    }

    fn should_scan(path: &Path) -> bool {
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if matches!(name, "Cargo.lock" | "package-lock.json") {
            return false;
        }
        if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
            matches!(
                ext,
                "rs" | "toml" | "json" | "ts" | "tsx" | "js" | "swift" | "c" | "h" | "sh" | "html"
            )
        } else {
            false
        }
    }

    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                ".git" | "target" | "node_modules" | "dist" | ".build" | ".swiftpm" | "gen"
            ) {
                continue;
            }
            if path.is_dir() {
                walk(&path, out);
            } else if should_scan(&path) {
                out.push(path);
            }
        }
    }

    #[test]
    fn generated_output_is_owner_only() {
        let dir = std::env::temp_dir().join(format!(
            "carapace-privacy-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&dir);
        let nested = dir.join("nested");
        private_dir(&nested).expect("private directory");
        let file = nested.join("Binding.swift");
        // Pre-create with loose permissions: private_write must tighten them.
        fs::write(&file, "stale").expect("seed file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
        }
        private_write(&file, "public enum Binding {}").expect("private file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&nested).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(&file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(&dir).expect("cleanup");
    }

    /// The README keeps a short Privacy section; the detail and fine print live in PRIVACY.md.
    #[test]
    fn readme_privacy_is_short_and_privacy_md_has_the_details() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let readme = fs::read_to_string(root.join("README.md")).expect("read README.md");
        let section = readme
            .split("\n## Privacy\n")
            .nth(1)
            .expect("README.md has a Privacy section");
        let section = section.split("\n## ").next().unwrap_or(section);
        let lines = section.lines().filter(|l| !l.trim().is_empty()).count();
        assert!(
            lines <= 6,
            "README Privacy section has {lines} lines; keep it to 6 and move detail to PRIVACY.md"
        );
        assert_eq!(
            readme.matches("\n## Privacy\n").count(),
            1,
            "README.md has more than one Privacy section"
        );
        for needed in ["PRIVACY.md", "CARAPACE_NO_UPDATE_CHECK=1", "update --clean"] {
            assert!(
                section.contains(needed),
                "README Privacy section lacks {needed:?}"
            );
        }
        assert!(
            !section.to_ascii_lowercase().contains("no update check"),
            "README Privacy section must describe the daily update check, not deny it"
        );
        let privacy = fs::read_to_string(root.join("PRIVACY.md")).expect("read PRIVACY.md");
        for detail in [
            "0700",
            "0600",
            "--out",
            "newest 50",
            "payloads",
            "DO_NOT_TRACK=1",
            "no remote fonts or CDNs",
            "state.json",
            "256 KB",
            "update --clean",
        ] {
            assert!(
                privacy.contains(detail),
                "PRIVACY.md lost the detail {detail:?}"
            );
        }
    }

    #[test]
    fn no_telemetry_packages_or_endpoints_are_declared() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut files = Vec::new();
        walk(&root, &mut files);
        assert!(files.len() > 20, "workspace scan found too few files");
        let terms = banned_terms();
        let hosts = banned_hosts();
        let manifest_terms = manifest_only_terms();
        for path in files {
            let text = fs::read_to_string(&path)
                .unwrap_or_default()
                .to_ascii_lowercase();
            let is_manifest = matches!(
                path.extension().and_then(|s| s.to_str()),
                Some("toml" | "json")
            );
            let all: Vec<&String> = if is_manifest {
                terms
                    .iter()
                    .chain(hosts.iter())
                    .chain(manifest_terms.iter())
                    .collect()
            } else {
                terms.iter().chain(hosts.iter()).collect()
            };
            for term in all {
                assert!(
                    !text.contains(term),
                    "{} contains a telemetry/analytics token in shipped code or a manifest",
                    path.display()
                );
            }
        }
    }
}
