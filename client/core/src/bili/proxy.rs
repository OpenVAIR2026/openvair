//! OpenVAIR Desktop keeps the bili proxy running for the agents routed through
//! it. The proxy is started as its own process, so it outlives the app and
//! its restarts; the app only checks on it and starts it again when it stops.
use super::{bili_names, find, search_path};
use crate::{CoreError, Result};
use std::{
    ffi::OsString,
    io::{Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// bili's default proxy port, which Claude Code's bili hook also uses, so
/// both agents share one proxy.
pub const PORT: u16 = 18787;
const CHECK_TIMEOUT: Duration = Duration::from_secs(1);
const START_TIMEOUT: Duration = Duration::from_secs(15);
const ROUTE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    /// bili answers its health check.
    Running,
    /// Nothing listens on the port.
    Stopped,
    /// Something other than bili listens on the port.
    Taken,
}

/// `http://127.0.0.1:<port>`, where the proxy listens.
pub fn origin(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

/// The port of a loopback proxy origin such as `http://127.0.0.1:18787`.
pub fn port(origin: &str) -> Option<u16> {
    origin.strip_prefix("http://127.0.0.1:")?.parse().ok()
}

/// Asks `/__bili/health` on the loopback port with a plain HTTP/1.0 request,
/// so the check needs no async runtime.
pub fn health(port: u16) -> Health {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&address, CHECK_TIMEOUT) else {
        return Health::Stopped;
    };
    let _ = stream.set_read_timeout(Some(CHECK_TIMEOUT));
    let _ = stream.set_write_timeout(Some(CHECK_TIMEOUT));
    let request = format!("GET /__bili/health HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n");
    let mut head = [0; 64];
    let answered = stream.write_all(request.as_bytes()).is_ok()
        && stream
            .read(&mut head)
            .is_ok_and(|read| is_ok_status(&head[..read]));
    if answered {
        Health::Running
    } else {
        Health::Taken
    }
}

fn is_ok_status(head: &[u8]) -> bool {
    let line = String::from_utf8_lossy(head);
    let mut parts = line.split_whitespace();
    parts
        .next()
        .is_some_and(|version| version.starts_with("HTTP/"))
        && parts.next() == Some("200")
}

/// Starts the proxy on `port` unless bili already answers there, then waits
/// until it does.
pub fn ensure(home: &Path, port: u16) -> Result<()> {
    match health(port) {
        Health::Running => return Ok(()),
        Health::Taken => return Err(taken(port)),
        Health::Stopped => {}
    }
    spawn(home, port)?;
    let started = Instant::now();
    while started.elapsed() < START_TIMEOUT {
        std::thread::sleep(Duration::from_millis(300));
        match health(port) {
            Health::Running => return Ok(()),
            // bili binds before it answers; give it the rest of the wait.
            Health::Taken | Health::Stopped => {}
        }
    }
    Err(match health(port) {
        Health::Taken => taken(port),
        _ => CoreError::Other(format!(
            "bili 没有在 {} 秒内启动（端口 {port}）",
            START_TIMEOUT.as_secs()
        )),
    })
}

fn taken(port: u16) -> CoreError {
    CoreError::Other(format!(
        "端口 {port} 已被其他程序占用，bili 无法启动；请关闭占用该端口的程序"
    ))
}

/// Checks that an agent's request for bili on `port` reaches it. Codex sends
/// even loopback requests through the macOS system proxy, ignoring its
/// exception list, so `system` takes that proxy into account; Claude Code
/// only follows the proxy environment variables. Proxies like Clash forward
/// `127.0.0.1` to a remote node and answer an empty 502.
pub fn check_route(port: u16, system: bool) -> Result<()> {
    let Some((host, proxy_port)) = agent_proxy(system) else {
        return Ok(());
    };
    through(&host, proxy_port, port).map_err(|detail| {
        CoreError::Other(format!(
            "代理 {host}:{proxy_port} 连不上本机 bili（{detail}），请关闭代理或让 127.0.0.1 直连"
        ))
    })
}

/// Asks bili's health check through an HTTP proxy, the way an agent would.
fn through(host: &str, proxy_port: u16, port: u16) -> std::result::Result<(), String> {
    let address = (host, proxy_port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut addresses| addresses.next())
        .ok_or_else(|| "代理地址无效".to_string())?;
    let mut stream = TcpStream::connect_timeout(&address, ROUTE_TIMEOUT)
        .map_err(|_| "代理未运行".to_string())?;
    let _ = stream.set_read_timeout(Some(ROUTE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(ROUTE_TIMEOUT));
    let request = format!(
        "GET http://127.0.0.1:{port}/__bili/health HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|_| "代理无响应".to_string())?;
    let mut head = [0; 64];
    let read = stream
        .read(&mut head)
        .map_err(|_| "代理无响应".to_string())?;
    if is_ok_status(&head[..read]) {
        return Ok(());
    }
    let head = String::from_utf8_lossy(&head[..read]);
    match head.split_whitespace().nth(1) {
        Some(status) => Err(format!("返回 {status}")),
        None => Err("代理无响应".into()),
    }
}

/// The HTTP proxy an agent uses for `http://127.0.0.1`: `http_proxy` or
/// `all_proxy` unless `no_proxy` exempts loopback, otherwise the macOS
/// system proxy when `system` is set.
fn agent_proxy(system: bool) -> Option<(String, u16)> {
    let var = |name: &str| {
        std::env::var(name)
            .or_else(|_| std::env::var(name.to_uppercase()))
            .ok()
            .filter(|value| !value.trim().is_empty())
    };
    if var("no_proxy").is_some_and(|list| exempts_loopback(&list)) {
        return None;
    }
    if let Some(url) = var("http_proxy").or_else(|| var("all_proxy")) {
        return parse_proxy_url(&url);
    }
    system.then(os_proxy).flatten()
}

fn exempts_loopback(list: &str) -> bool {
    list.split(',').map(str::trim).any(|entry| {
        matches!(
            entry,
            "*" | "127.0.0.1" | "localhost" | "127.0.0.0/8" | "127.0.0.1/32"
        )
    })
}

/// `http://host:port` or `host:port`; SOCKS proxies are not checked.
fn parse_proxy_url(url: &str) -> Option<(String, u16)> {
    let url = url.trim();
    let rest = match url.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") => rest,
        Some(_) => return None,
        None => url,
    };
    let authority = rest.split('/').next()?;
    let authority = authority.rsplit('@').next()?;
    let (host, port) = authority.rsplit_once(':')?;
    Some((host.to_string(), port.parse().ok()?))
}

#[cfg(target_os = "macos")]
fn os_proxy() -> Option<(String, u16)> {
    let output = Command::new("/usr/sbin/scutil")
        .arg("--proxy")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    scutil_proxy(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(not(target_os = "macos"))]
fn os_proxy() -> Option<(String, u16)> {
    None
}

/// The HTTP proxy in `scutil --proxy` output. Its `ExceptionsList` is left
/// out on purpose, because Codex does not honour it.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn scutil_proxy(output: &str) -> Option<(String, u16)> {
    let value = |key: &str| {
        output.lines().find_map(|line| {
            let (name, value) = line.split_once(" : ")?;
            (name.trim() == key).then(|| value.trim().to_string())
        })
    };
    if value("HTTPEnable").as_deref() != Some("1") {
        return None;
    }
    Some((value("HTTPProxy")?, value("HTTPPort")?.parse().ok()?))
}

/// Runs `bili start` detached from the app, with no console window and its
/// output discarded; bili keeps its own log.
fn spawn(home: &Path, port: u16) -> Result<()> {
    let bili = find(home, bili_names()).ok_or_else(|| CoreError::Other("请先安装 bili".into()))?;
    // `bili` runs on the `node` found on PATH, which apps started from the
    // Dock do not inherit.
    let mut folders = search_path(home);
    if let Some(parent) = bili.parent() {
        folders.insert(0, parent.to_path_buf());
    }
    let path = std::env::join_paths(folders).unwrap_or_else(|_| OsString::new());
    let mut command = Command::new(&bili);
    command
        .args(["start", "--host", "127.0.0.1", "--port", &port.to_string()])
        .env("PATH", path)
        .current_dir(home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    detach(&mut command);
    command
        .spawn()
        .map(drop)
        .map_err(|error| CoreError::Other(format!("启动 bili 失败：{error}")))
}

#[cfg(unix)]
fn detach(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    // Its own process group, so signals sent to the app's group miss it.
    command.process_group(0);
}

#[cfg(windows)]
fn detach(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
}

#[cfg(not(any(unix, windows)))]
fn detach(_: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn serve_once(response: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0; 256];
                let _ = stream.read(&mut request);
                let _ = stream.write_all(response.as_bytes());
            }
        });
        port
    }

    #[test]
    fn tells_bili_from_other_programs_and_free_ports() {
        let bili = serve_once("HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}");
        assert_eq!(health(bili), Health::Running);
        let other = serve_once("HTTP/1.1 404 Not Found\r\n\r\n");
        assert_eq!(health(other), Health::Taken);
        let free = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert_eq!(health(free), Health::Stopped);
    }

    #[test]
    fn a_taken_port_fails_without_starting_anything() {
        let home = tempfile::tempdir().unwrap();
        let other = serve_once("HTTP/1.1 404 Not Found\r\n\r\n");
        let error = ensure(home.path(), other).unwrap_err().to_string();
        assert!(error.contains("已被其他程序占用"), "{error}");
    }

    #[test]
    fn reads_the_port_of_a_loopback_origin() {
        assert_eq!(port("http://127.0.0.1:18787"), Some(18787));
        assert_eq!(port("http://localhost:18787"), None);
        assert_eq!(origin(PORT), "http://127.0.0.1:18787");
    }

    #[test]
    fn a_proxy_that_cannot_reach_bili_fails_the_route() {
        let clash = serve_once("HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n");
        let error = through("127.0.0.1", clash, PORT).unwrap_err();
        assert!(error.contains("502"), "{error}");
        let direct = serve_once("HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}");
        assert_eq!(through("127.0.0.1", direct, PORT), Ok(()));
    }

    #[test]
    fn reads_the_http_proxy_from_scutil() {
        let enabled = "<dictionary> {\n  ExceptionsList : <array> {\n    0 : 127.0.0.1\n  }\n  HTTPEnable : 1\n  HTTPPort : 7897\n  HTTPProxy : 127.0.0.1\n}\n";
        assert_eq!(scutil_proxy(enabled), Some(("127.0.0.1".into(), 7897)));
        let disabled = enabled.replace("HTTPEnable : 1", "HTTPEnable : 0");
        assert_eq!(scutil_proxy(&disabled), None);
    }

    #[test]
    fn reads_proxy_urls_and_loopback_exemptions() {
        assert_eq!(
            parse_proxy_url("http://user:pw@127.0.0.1:7897/"),
            Some(("127.0.0.1".into(), 7897))
        );
        assert_eq!(
            parse_proxy_url("proxy.lan:3128"),
            Some(("proxy.lan".into(), 3128))
        );
        assert_eq!(parse_proxy_url("socks5://127.0.0.1:7897"), None);
        assert!(exempts_loopback("example.com, localhost"));
        assert!(!exempts_loopback("example.com,.internal"));
    }
}
