//! The catalog each Codex release ships, which `codex debug models --bundled`
//! prints. Codex's download from ChatGPT stops at the last sign-in, while the
//! shipped catalog follows every upgrade, so new models arrive with the
//! release that knows how to run them.
// Tests stand in for the installed Codex, so the lookup goes unused there.
#![cfg_attr(test, allow(dead_code))]
use super::detect;
use serde_json::{Map, Value};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime},
};

#[derive(Clone, Debug, PartialEq)]
pub struct Bundled {
    pub version: String,
    pub catalog: Map<String, Value>,
}

const TIMEOUT: Duration = Duration::from_secs(10);

/// What identifies an install: a new release replaces the file.
type Stamp = Vec<(PathBuf, Option<SystemTime>, u64)>;

/// The last lookup, reused until an install changes; the tray asks every
/// 30 seconds.
static LAST: Mutex<Option<(Stamp, Option<Bundled>)>> = Mutex::new(None);

/// The catalog of the newest Codex installed, if any can print one.
#[cfg(not(test))]
pub fn find(home: &Path) -> Option<Bundled> {
    lookup(home)
}

#[cfg(test)]
thread_local! {
    static FAKE: std::cell::RefCell<Option<Bundled>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub fn find(_home: &Path) -> Option<Bundled> {
    FAKE.with(|fake| fake.borrow().clone())
}

/// Stands in for the installed Codex in this test's thread.
#[cfg(test)]
pub fn fake(version: &str, models: Value) {
    let mut catalog = Map::new();
    catalog.insert("models".into(), models);
    FAKE.with(|fake| {
        *fake.borrow_mut() = Some(Bundled {
            version: version.to_owned(),
            catalog,
        })
    });
}

fn lookup(home: &Path) -> Option<Bundled> {
    let programs = programs(home);
    let stamp: Stamp = programs
        .iter()
        .map(|program| {
            let meta = std::fs::metadata(program).ok();
            (
                program.clone(),
                meta.as_ref().and_then(|meta| meta.modified().ok()),
                meta.map_or(0, |meta| meta.len()),
            )
        })
        .collect();
    let mut last = LAST.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((seen, found)) = last.as_ref()
        && *seen == stamp
    {
        return found.clone();
    }
    let found = programs
        .iter()
        .filter_map(|program| read(home, program))
        .max_by(|a, b| order(&a.version).cmp(&order(&b.version)));
    *last = Some((stamp, found.clone()));
    found
}

/// Every Codex command found, each install once: the CLI on the search path,
/// and on macOS the Codex app and the one inside the ChatGPT app.
fn programs(home: &Path) -> Vec<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["codex.cmd", "codex.exe"]
    } else {
        &["codex"]
    };
    let mut programs: Vec<PathBuf> = detect::folders(home, std::env::var_os("PATH"))
        .iter()
        .flat_map(|folder| names.iter().map(move |name| folder.join(name)))
        .collect();
    if cfg!(target_os = "macos") {
        for apps in [PathBuf::from("/Applications"), home.join("Applications")] {
            programs.push(apps.join("Codex.app/Contents/Resources/codex"));
            programs.push(apps.join(
                "ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex",
            ));
        }
    }
    // npm's command is a link to a script; it is run by its own name, so the
    // script finds its package, and counted once by where it leads.
    let mut seen = BTreeSet::new();
    programs.retain(|program| {
        program.is_file() && std::fs::canonicalize(program).is_ok_and(|real| seen.insert(real))
    });
    programs
}

/// Older releases without `--bundled` give nothing.
fn read(home: &Path, program: &Path) -> Option<Bundled> {
    let run = |args: &[&str], action: &str| {
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        crate::bili::run(home, program, &args, &[], TIMEOUT, action).ok()
    };
    // `codex-cli 0.164.0`
    let version = run(&["--version"], "读取 Codex 版本")?
        .split_whitespace()
        .last()?
        .to_owned();
    let text = run(
        &["debug", "models", "--bundled"],
        "读取 Codex 自带的模型列表",
    )?;
    match serde_json::from_str(&text).ok()? {
        Value::Object(catalog) => Some(Bundled { version, catalog }),
        _ => None,
    }
}

/// Whether Codex version `a` is newer than `b`.
pub fn newer(a: &str, b: &str) -> bool {
    order(a) > order(b)
}

/// Sorts versions such as `0.164.0`, `0.166.0-alpha.1` and
/// `0.164.0-alpha.11.1`; a release follows its own prereleases, and numbers
/// compare as numbers.
fn order(version: &str) -> (Vec<u64>, bool, Vec<Result<u64, String>>) {
    let (core, pre) = match version.split_once('-') {
        Some((core, pre)) => (core, Some(pre)),
        None => (version, None),
    };
    let core = core
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect();
    let pre: Vec<Result<u64, String>> = pre
        .map(|pre| {
            pre.split('.')
                .map(|part| part.parse().map_err(|_| part.to_owned()))
                .collect()
        })
        .unwrap_or_default();
    (core, pre.is_empty(), pre)
}

#[cfg(test)]
mod tests {
    use super::newer;

    #[test]
    fn orders_codex_versions() {
        assert!(newer("0.166.0-alpha.1", "0.164.0"));
        assert!(newer("0.164.0", "0.164.0-alpha.11.1"));
        assert!(newer("0.164.0-alpha.11.1", "0.164.0-alpha.2"));
        assert!(newer("0.164.0", "0.162.1"));
        assert!(newer("0.10.0", "0.9.9"));
        assert!(!newer("0.164.0", "0.164.0"));
        assert!(!newer("0.162.1", "0.164.0"));
    }
}
