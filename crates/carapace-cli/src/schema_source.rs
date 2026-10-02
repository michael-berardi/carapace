//! Find the schema: a file, or the core's own built library.

use std::ffi::{c_char, CStr, CString};
use std::process::{Command, Stdio};

use serde_json::Value;

use crate::Args;

pub fn load(args: &Args) -> Result<String, String> {
    if let Some(path) = &args.schema {
        return std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()));
    }
    let lib = build_cdylib(args)?;
    read_from_library(&lib)
}

/// Build the package and return the path of its cdylib.
pub fn build_cdylib(args: &Args) -> Result<String, String> {
    let mut cmd = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    cmd.args(["build", "--lib", "--message-format=json-render-diagnostics"]);
    if let Some(p) = &args.package {
        cmd.args(["-p", p]);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::inherit());
    let out = cmd.output().map_err(|e| format!("cannot run cargo: {e}"))?;
    if !out.status.success() {
        return Err("cargo build failed (see the output above)".into());
    }
    let wanted = ["dylib", "so", "dll"];
    let mut found = None;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v["reason"] != "compiler-artifact" {
            continue;
        }
        let is_cdylib = v["target"]["crate_types"]
            .as_array()
            .is_some_and(|t| t.iter().any(|c| c == "cdylib"));
        if !is_cdylib {
            continue;
        }
        if let Some(files) = v["filenames"].as_array() {
            for f in files.iter().filter_map(Value::as_str) {
                if wanted.iter().any(|ext| f.ends_with(&format!(".{ext}"))) {
                    found = Some(f.to_string());
                }
            }
        }
    }
    found.ok_or_else(|| {
        "the package builds no cdylib. Add `crate-type = [\"lib\", \"staticlib\", \"cdylib\"]` to its [lib] section, \
         or pass --schema <file>"
            .into()
    })
}

#[cfg(unix)]
fn read_from_library(path: &str) -> Result<String, String> {
    let c_path = CString::new(path).map_err(|e| e.to_string())?;
    // SAFETY: loading the user's own freshly built core library and calling two C functions with known signatures.
    unsafe {
        let lib = libc::dlopen(c_path.as_ptr(), libc::RTLD_NOW);
        if lib.is_null() {
            let why = CStr::from_ptr(libc::dlerror())
                .to_string_lossy()
                .into_owned();
            return Err(format!("cannot load {path}: {why}"));
        }
        let sym = |name: &str| -> Result<*mut libc::c_void, String> {
            let n = CString::new(name).unwrap();
            let p = libc::dlsym(lib, n.as_ptr());
            if p.is_null() {
                Err(format!("{path} does not export {name}; call `carapace::export!(YourApp)` in the library"))
            } else {
                Ok(p)
            }
        };
        let schema: extern "C" fn() -> *mut c_char = std::mem::transmute(sym("carapace_schema")?);
        let free: extern "C" fn(*mut c_char) = std::mem::transmute(sym("carapace_string_free")?);
        let raw = schema();
        let text = CStr::from_ptr(raw).to_string_lossy().into_owned();
        free(raw);
        Ok(text)
    }
}

#[cfg(not(unix))]
fn read_from_library(path: &str) -> Result<String, String> {
    Err(format!(
        "reading the schema from {path} is only implemented on macOS and Linux. Run the core's schema example and pass \
         the file with --schema <file>"
    ))
}
