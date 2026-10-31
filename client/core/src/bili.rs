//! bili (the `billion-context` npm package) compresses long Claude Code
//! conversations through a local proxy. Its native Claude Code mode adds a
//! SessionStart hook that starts the proxy and, each session, repins
//! `ANTHROPIC_BASE_URL` to `http://127.0.0.1:<port>/bili/<upstream>` while
//! keeping the upstream. bili leaves any other base URL alone, so OpenVAIR asks it
//! to wrap OpenVAIR's address on install. On removal bili deletes its URL instead
//! of restoring one, so OpenVAIR writes its own address back. Codex is set up
//! in [`codex`]; OpenVAIR Desktop keeps the proxy running, see [`proxy`].
pub mod codex;
pub mod proxy;

use crate::{CoreError, Result};
use serde::Serialize;
use serde_json::Value;
use std::{
    ffi::OsString,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// The version OpenVAIR has been checked against; bili's tag format matters to
/// the gateway, so it is not left to float.
pub const VERSION: &str = "0.1.197";
pub const MIRROR: &str = "https://registry.npmmirror.com";
const NPM_REGISTRY: &str = "https://registry.npmjs.org/";
const PING_TIMEOUT: Duration = Duration::from_secs(8);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(180);
const PLUGIN_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Status {
    /// npm was found, so bili can be installed from here.
    pub npm: bool,
    /// The `bili` command was found.
    pub installed: bool,
    pub claude_code: AgentStatus,
    pub codex: AgentStatus,
    /// bili's web interface, at the proxy origin an enabled agent uses.
    pub dashboard: Option<String>,
    /// Whether the proxy at `dashboard` answers; `None` while no agent is
    /// routed through bili.
    pub running: Option<bool>,
    /// Why OpenVAIR Desktop last failed to start the proxy, filled in by the app.
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
pub struct AgentStatus {
    /// bili is set up in the agent's settings: Claude Code's SessionStart
    /// hook, or Codex's bili tools.
    pub hooked: bool,
    /// The agent is routed through bili to this OpenVAIR server.
    pub enabled: bool,
}

/// The proxy origin of a bili-managed base URL.
fn origin(url: &str) -> Option<&str> {
    wrapped_upstream(url).map(|upstream| &url[..url.len() - upstream.len() - "/bili/".len()])
}

/// The upstream inside a bili-managed base URL, or the URL itself.
pub fn unwrap(url: &str) -> &str {
    wrapped_upstream(url).unwrap_or(url)
}

/// The upstream of `http://127.0.0.1:<port>/bili/<http(s) upstream>`, the only
/// form bili's hook repins.
pub fn wrapped_upstream(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("http://127.0.0.1:")?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || digits > 5 {
        return None;
    }
    let upstream = rest[digits..].strip_prefix("/bili/")?;
    (upstream.starts_with("https://") || upstream.starts_with("http://")).then_some(upstream)
}

/// `upstream` behind the same bili proxy as `current`, when `current` is
/// bili-managed; otherwise `upstream` itself.
pub fn keep_wrapping(current: Option<&str>, upstream: &str) -> String {
    match current.and_then(origin) {
        Some(origin) => format!("{origin}/bili/{upstream}"),
        None => upstream.to_owned(),
    }
}

fn settings_path(home: &Path) -> PathBuf {
    home.join(".claude/settings.json")
}

fn base_url(home: &Path) -> Result<Option<String>> {
    let settings = crate::agents::read_json(&settings_path(home))?;
    Ok(settings
        .get("env")
        .and_then(|env| env.get("ANTHROPIC_BASE_URL"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned))
}

fn hooked(home: &Path) -> Result<bool> {
    let settings = crate::agents::read_json(&settings_path(home))?;
    let entries = settings
        .get("hooks")
        .and_then(|hooks| hooks.get("SessionStart"))
        .and_then(Value::as_array);
    Ok(entries.into_iter().flatten().any(|entry| {
        entry
            .get("hooks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|hook| hook.get("command").and_then(Value::as_str))
            .any(|command| command.contains("claude-native-bootstrap"))
    }))
}

/// Sets or, with `None`, removes `env.ANTHROPIC_BASE_URL`.
fn set_base_url(home: &Path, url: Option<&str>) -> Result<()> {
    let path = settings_path(home);
    let mut settings = crate::agents::read_json(&path)?;
    let env = settings
        .entry("env")
        .or_insert_with(|| Value::Object(Default::default()));
    if !env.is_object() {
        *env = Value::Object(Default::default());
    }
    let env = env.as_object_mut().expect("env was just made an object");
    match url {
        Some(url) => {
            env.insert("ANTHROPIC_BASE_URL".into(), Value::String(url.into()));
        }
        None => {
            env.remove("ANTHROPIC_BASE_URL");
        }
    }
    if env.is_empty() {
        settings.remove("env");
    }
    crate::agents::write_json(&path, settings)
}

/// `server` is the OpenVAIR server; each agent's upstream is its OpenVAIR base URL.
pub fn status(home: &Path, server: &url::Url) -> Result<Status> {
    let root = crate::agents::base_url(crate::agents::AgentId::ClaudeCode, server);
    let (claude_code, claude_url) = claude_status(home, &root)?;
    let (codex, codex_url) = codex::status(
        home,
        &crate::agents::base_url(crate::agents::AgentId::Codex, server),
    )?;
    let dashboard = claude_url
        .as_deref()
        .filter(|_| claude_code.enabled)
        .or(codex_url.as_deref().filter(|_| codex.enabled))
        .and_then(origin)
        .map(ToOwned::to_owned);
    let running = dashboard
        .as_deref()
        .and_then(proxy::port)
        .map(|port| proxy::health(port) == proxy::Health::Running);
    Ok(Status {
        npm: find(home, npm_names()).is_some(),
        installed: find(home, bili_names()).is_some(),
        claude_code,
        codex,
        dashboard,
        running,
        error: None,
    })
}

/// The proxy ports the agents routed through bili use, which OpenVAIR Desktop
/// keeps running.
pub fn ports(home: &Path, server: &url::Url) -> Result<Vec<u16>> {
    let root = crate::agents::base_url(crate::agents::AgentId::ClaudeCode, server);
    let (claude_code, claude_url) = claude_status(home, &root)?;
    let (codex, codex_url) = codex::status(
        home,
        &crate::agents::base_url(crate::agents::AgentId::Codex, server),
    )?;
    let mut ports: Vec<u16> = [(claude_code, claude_url), (codex, codex_url)]
        .into_iter()
        .filter(|(status, _)| status.enabled)
        .filter_map(|(_, url)| url.as_deref().and_then(origin).and_then(proxy::port))
        .collect();
    ports.dedup();
    Ok(ports)
}

fn claude_status(home: &Path, server_root: &str) -> Result<(AgentStatus, Option<String>)> {
    let url = base_url(home)?;
    let hooked = hooked(home)?;
    let enabled = hooked && url.as_deref().and_then(wrapped_upstream) == Some(server_root);
    Ok((AgentStatus { hooked, enabled }, url))
}

/// Installs the bili npm package unless the `bili` command is already there.
/// Claude Code's settings are not touched; see [`enable`].
pub fn install(home: &Path) -> Result<()> {
    if find(home, bili_names()).is_some() {
        return Ok(());
    }
    {
        {
            let npm = find(home, npm_names()).ok_or_else(|| {
                CoreError::Other("没有找到 npm，请先安装 Node.js 18 或更新版本".into())
            })?;
            let prefix = home.join(".local");
            let package = format!("billion-context@{VERSION}");
            let args: Vec<String> = vec![
                "install".into(),
                "-g".into(),
                package,
                "--prefix".into(),
                prefix.display().to_string(),
            ];
            let mut failure = None;
            for registry in registries(home, &npm) {
                let mut attempt = args.clone();
                if let Some(registry) = registry {
                    attempt.extend(["--registry".into(), registry.into()]);
                }
                match run(home, &npm, &attempt, &[], INSTALL_TIMEOUT, "安装 bili") {
                    Ok(_) => {
                        failure = None;
                        break;
                    }
                    Err(error) => failure = Some(error),
                }
            }
            if let Some(error) = failure {
                return Err(error);
            }
            find(home, bili_names())
                .ok_or_else(|| CoreError::Other("bili 已安装，但没有找到 bili 命令".into()))?;
        }
    }
    Ok(())
}

/// Routes Claude Code through an installed bili to `server_root`.
pub fn enable(home: &Path, server_root: &str) -> Result<()> {
    let bili = find(home, bili_names()).ok_or_else(|| CoreError::Other("请先安装 bili".into()))?;
    proxy::ensure(home, proxy::PORT)?;
    proxy::check_route(proxy::PORT, false)?;
    // bili wraps only an unset or bili-managed URL, taking the upstream from
    // BILI_CLAUDE_UPSTREAM; the previous value comes back on failure.
    let previous = base_url(home)?;
    if previous
        .as_deref()
        .is_some_and(|url| wrapped_upstream(url).is_none())
    {
        set_base_url(home, None)?;
    }
    let outcome = run(
        home,
        &bili,
        &["plugin".into(), "install".into(), "claude".into()],
        &[("BILI_CLAUDE_UPSTREAM", server_root)],
        PLUGIN_TIMEOUT,
        "启用 bili",
    )
    .and_then(|_| {
        let url = base_url(home)?;
        if url.as_deref().and_then(wrapped_upstream) == Some(server_root) {
            Ok(())
        } else {
            Err(CoreError::Other(format!(
                "bili 没有把 Claude Code 指向 OpenVAIR（当前为 {}）",
                url.as_deref().unwrap_or("未设置")
            )))
        }
    });
    if outcome.is_err() {
        set_base_url(home, previous.as_deref())?;
    }
    outcome
}

/// Removes bili's Claude Code hook and points Claude Code at `server_root`
/// directly again. The npm package stays installed.
pub fn disable(home: &Path, server_root: &str) -> Result<()> {
    if let Some(bili) = find(home, bili_names()) {
        run(
            home,
            &bili,
            &["plugin".into(), "remove".into(), "claude".into()],
            &[],
            PLUGIN_TIMEOUT,
            "移除 bili",
        )?;
    }
    restore_address(home, server_root)
}

/// bili deletes its URL on removal; without one Claude Code would send the
/// OpenVAIR key to Anthropic.
fn restore_address(home: &Path, server_root: &str) -> Result<()> {
    let url = base_url(home)?;
    if url
        .as_deref()
        .is_none_or(|url| wrapped_upstream(url).is_some())
    {
        set_base_url(home, Some(server_root))?;
    }
    Ok(())
}

/// Registries to try in order; `None` is npm's own setting. A registry the
/// user configured is kept. Otherwise whichever of npmjs.org and the mirror
/// answers faster goes first, and the other is the fallback.
fn registries(home: &Path, npm: &Path) -> Vec<Option<&'static str>> {
    let get = ["config".into(), "get".into(), "registry".into()];
    let configured = run(home, npm, &get, &[], PING_TIMEOUT, "读取 npm 源").unwrap_or_default();
    if !is_default_registry(&configured) {
        return vec![None];
    }
    let ping = |registry: &str| {
        let started = Instant::now();
        let args = ["ping".into(), "--registry".into(), registry.into()];
        run(home, npm, &args, &[], PING_TIMEOUT, "检查 npm 源")
            .ok()
            .map(|_| started.elapsed())
    };
    let (npmjs, mirror) = std::thread::scope(|scope| {
        let npmjs = scope.spawn(|| ping(NPM_REGISTRY));
        let mirror = ping(MIRROR);
        (npmjs.join().ok().flatten(), mirror)
    });
    order(npmjs, mirror)
}

fn is_default_registry(configured: &str) -> bool {
    let configured = configured.trim().trim_end_matches('/');
    configured.is_empty() || configured == NPM_REGISTRY.trim_end_matches('/')
}

/// npmjs.org first, unless only the mirror answered or it answered more than
/// twice as fast.
fn order(npmjs: Option<Duration>, mirror: Option<Duration>) -> Vec<Option<&'static str>> {
    let mirror_first = match (npmjs, mirror) {
        (None, Some(_)) => true,
        (Some(npmjs), Some(mirror)) => mirror * 2 < npmjs,
        _ => false,
    };
    if mirror_first {
        vec![Some(MIRROR), None]
    } else {
        vec![None, Some(MIRROR)]
    }
}

fn npm_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["npm.cmd"]
    } else {
        &["npm"]
    }
}

fn bili_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["bili.cmd", "bili.exe"]
    } else {
        &["bili"]
    }
}

/// Folders searched for commands. Apps started from the Dock do not inherit
/// the shell's PATH, so common Node install locations are added.
fn search_path(home: &Path) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    // npm --prefix puts commands in <prefix>/bin, or the prefix itself on Windows.
    folders.push(if cfg!(windows) {
        home.join(".local")
    } else {
        home.join(".local/bin")
    });
    folders.extend([".volta/bin", ".npm-global/bin", ".bun/bin"].map(|folder| home.join(folder)));
    if let Ok(entries) = std::fs::read_dir(home.join(".nvm/versions/node")) {
        let mut versions: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path().join("bin"))
            .collect();
        versions.sort();
        folders.extend(versions.into_iter().rev());
    }
    folders.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    folders
}

fn find(home: &Path, names: &[&str]) -> Option<PathBuf> {
    search_path(home).into_iter().find_map(|folder| {
        names
            .iter()
            .map(|name| folder.join(name))
            .find(|path| path.is_file())
    })
}

/// Runs a command with the command's own folder first on PATH, so the
/// `node` next to npm or bili is the one used.
pub(crate) fn run(
    home: &Path,
    program: &Path,
    args: &[String],
    envs: &[(&str, &str)],
    timeout: Duration,
    action: &str,
) -> Result<String> {
    let mut folders = search_path(home);
    if let Some(parent) = program.parent() {
        folders.insert(0, parent.to_path_buf());
    }
    let path = std::env::join_paths(folders).unwrap_or_else(|_| OsString::new());
    let mut child = Command::new(program)
        .args(args)
        .envs(envs.iter().copied())
        .env("PATH", path)
        // bili edits the settings file under the home directory it sees.
        .env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }, home)
        .current_dir(home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| CoreError::Other(format!("{action}失败：{error}")))?;
    let drain = |stream: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(mut stream) = stream {
                let _ = stream.read_to_string(&mut text);
            }
            text
        })
    };
    let stdout = drain(
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let stderr = drain(
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(200))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    match status {
        Some(status) if status.success() => Ok(stdout),
        Some(_) => Err(CoreError::Other(format!(
            "{action}失败：{}",
            tail(&format!("{stdout}{stderr}"))
        ))),
        None => Err(CoreError::Other(format!(
            "{action}超时（{} 秒）",
            timeout.as_secs()
        ))),
    }
}

/// The last lines of a command's output, enough to see why it failed.
fn tail(output: &str) -> String {
    let lines: Vec<&str> = output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    lines[lines.len().saturating_sub(6)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "https://openvair.example.test";

    fn write(home: &Path, text: &str) {
        let path = settings_path(home);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn recognizes_only_the_url_form_bili_repins() {
        let url = format!("http://127.0.0.1:41234/bili/{ROOT}");
        assert_eq!(wrapped_upstream(&url), Some(ROOT));
        assert_eq!(unwrap(&url), ROOT);
        assert_eq!(unwrap(ROOT), ROOT);
        for other in [
            format!("http://localhost:8787/bili/{ROOT}"),
            "http://127.0.0.1:8787/bili/openvair.example.test".into(),
            format!("http://127.0.0.1:/bili/{ROOT}"),
            format!("http://127.0.0.1:123456/bili/{ROOT}"),
        ] {
            assert_eq!(wrapped_upstream(&other), None, "{other}");
        }
        assert_eq!(
            keep_wrapping(Some(&url), "https://other.test"),
            "http://127.0.0.1:41234/bili/https://other.test"
        );
        assert_eq!(
            keep_wrapping(Some(ROOT), "https://other.test"),
            "https://other.test"
        );
        assert_eq!(keep_wrapping(None, ROOT), ROOT);
    }

    #[test]
    fn status_requires_the_hook_and_a_url_to_this_server() {
        let home = tempfile::tempdir().unwrap();
        let hook = r#""hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"node /x/dist/claude-native-bootstrap.js"}]}]}"#;
        write(
            home.path(),
            &format!(r#"{{"env":{{"ANTHROPIC_BASE_URL":"{ROOT}"}},{hook}}}"#),
        );
        let (status, _) = claude_status(home.path(), ROOT).unwrap();
        assert!(status.hooked && !status.enabled);
        write(
            home.path(),
            &format!(
                r#"{{"env":{{"ANTHROPIC_BASE_URL":"http://127.0.0.1:9/bili/{ROOT}"}},{hook}}}"#
            ),
        );
        let enabled = super::status(home.path(), &ROOT.parse().unwrap()).unwrap();
        assert!(enabled.claude_code.enabled && !enabled.codex.enabled);
        assert_eq!(enabled.dashboard.as_deref(), Some("http://127.0.0.1:9"));
        assert!(
            !claude_status(home.path(), "https://other.test")
                .unwrap()
                .0
                .enabled
        );
        write(
            home.path(),
            &format!(r#"{{"env":{{"ANTHROPIC_BASE_URL":"http://127.0.0.1:9/bili/{ROOT}"}}}}"#),
        );
        assert!(!claude_status(home.path(), ROOT).unwrap().0.enabled);
    }

    #[test]
    fn picks_the_faster_registry_and_keeps_a_users_own() {
        let ms = Duration::from_millis;
        assert_eq!(order(Some(ms(300)), Some(ms(200))), [None, Some(MIRROR)]);
        assert_eq!(order(Some(ms(900)), Some(ms(200))), [Some(MIRROR), None]);
        assert_eq!(order(None, Some(ms(200))), [Some(MIRROR), None]);
        assert_eq!(order(Some(ms(900)), None), [None, Some(MIRROR)]);
        assert_eq!(order(None, None), [None, Some(MIRROR)]);
        assert!(is_default_registry("https://registry.npmjs.org/\n"));
        assert!(is_default_registry(""));
        assert!(!is_default_registry("https://npm.example.corp/"));
    }

    #[test]
    fn removing_puts_vair_back_when_bili_left_no_address() {
        let home = tempfile::tempdir().unwrap();
        write(home.path(), r#"{"env":{"DEBUG":"1"},"permissions":{}}"#);
        restore_address(home.path(), ROOT).unwrap();
        assert_eq!(base_url(home.path()).unwrap().as_deref(), Some(ROOT));
        write(
            home.path(),
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://mine.test"}}"#,
        );
        restore_address(home.path(), ROOT).unwrap();
        assert_eq!(
            base_url(home.path()).unwrap().as_deref(),
            Some("https://mine.test")
        );
    }
}
