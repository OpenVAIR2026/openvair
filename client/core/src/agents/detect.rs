//! Whether an agent is installed: its command can be found, or it has left
//! files that only running it creates. A configuration folder alone proves
//! nothing, since other tools (and OpenVAIR itself) create those.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
    time::{Duration, Instant},
};

pub(super) fn installed(home: &Path, binaries: &[&str], traces: &[&str]) -> bool {
    traces.iter().any(|trace| home.join(trace).exists())
        || found(&folders(home, std::env::var_os("PATH")), binaries)
}

/// Whether one of `binaries` is in one of `folders`.
pub(super) fn found(folders: &[PathBuf], binaries: &[&str]) -> bool {
    let names: Vec<String> = binaries
        .iter()
        .flat_map(|binary| {
            if cfg!(windows) {
                vec![format!("{binary}.cmd"), format!("{binary}.exe")]
            } else {
                vec![(*binary).to_owned()]
            }
        })
        .collect();
    folders
        .iter()
        .any(|folder| names.iter().any(|name| folder.join(name).is_file()))
}

/// Where commands may live. An app opened from the Dock or a desktop menu
/// does not get the PATH set up in the user's shell, so the usual install
/// folders are added, and on macOS and Linux the shell's own PATH as well.
pub(super) fn folders(home: &Path, path: Option<OsString>) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = path
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    folders.extend(shell_path().iter().cloned());
    folders.extend(
        [
            ".local/bin",
            ".npm-global/bin",
            ".bun/bin",
            ".volta/bin",
            ".claude/local",
            ".opencode/bin",
            ".local/share/pnpm",
            "Library/pnpm",
            "scoop/shims",
        ]
        .map(|folder| home.join(folder)),
    );
    folders.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    for variable in ["APPDATA", "LOCALAPPDATA"] {
        if let Some(base) = std::env::var_os(variable) {
            let base = PathBuf::from(base);
            folders.extend([base.join("npm"), base.join("pnpm")]);
        }
    }
    // Every Node version that nvm or fnm keeps; a global npm install lands in one.
    folders.extend(versions(&home.join(".nvm/versions/node"), "bin"));
    for fnm in [
        ".local/share/fnm/node-versions",
        "Library/Application Support/fnm/node-versions",
    ] {
        folders.extend(versions(&home.join(fnm), "installation/bin"));
    }
    folders
}

fn versions(root: &Path, bin: &str) -> Vec<PathBuf> {
    std::fs::read_dir(root)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path().join(bin))
                .collect()
        })
        .unwrap_or_default()
}

/// The PATH an interactive login shell sets up, read once. Empty on Windows,
/// where apps already get the user's PATH, or when the shell does not answer
/// within two seconds.
fn shell_path() -> &'static [PathBuf] {
    static PATH: OnceLock<Vec<PathBuf>> = OnceLock::new();
    PATH.get_or_init(|| {
        if cfg!(windows) {
            return Vec::new();
        }
        let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into());
        ask_shell(&shell).unwrap_or_default()
    })
}

const MARK: &str = "__VAIR_PATH__";

fn ask_shell(shell: &OsString) -> Option<Vec<PathBuf>> {
    let mut child = Command::new(shell)
        .args(["-ilc", &format!("printf '{MARK}%s{MARK}' \"$PATH\"")])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        if child.try_wait().ok()?.is_some() {
            break;
        }
        if started.elapsed() > Duration::from_secs(2) {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    // Startup files may print their own lines; the PATH is between the marks.
    let path = text.split(MARK).nth(1)?;
    Some(std::env::split_paths(path).collect())
}
