//! OpenVAIR Desktop stays in the tray after its window closes, so it can keep
//! the bili proxy running for the agents routed through it. Codex has no
//! other way to start the proxy; see `openvair_core::bili::proxy`.
use crate::App;
use openvair_core::{CoreError, Result, agents, bili};
use std::{
    path::{Path, PathBuf},
    sync::{Mutex, mpsc},
    time::{Duration, Instant},
};
use tauri::{
    AppHandle, Manager, Window,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
};

/// The argument autostart launches the app with, to start in the tray only.
pub const HIDDEN: &str = "--hidden";
const TRAY: &str = "openvair";
const CHECK_EVERY: Duration = Duration::from_secs(30);
/// The longest wait between attempts while the proxy keeps failing to start.
const BACKOFF_LIMIT: Duration = Duration::from_secs(300);
/// Model lists rarely change, so the server is asked less often.
const SYNC_EVERY: Duration = Duration::from_secs(15 * 60);

/// What the supervisor last saw, shared with the commands.
pub struct Watch {
    error: Mutex<Option<String>>,
    wake: Mutex<mpsc::Sender<()>>,
}

impl Watch {
    /// Adds why the proxy last failed to start.
    pub fn report(&self, mut status: bili::Status) -> bili::Status {
        status.error = self.error.lock().ok().and_then(|error| error.clone());
        status
    }

    /// Checks again now, e.g. after bili was turned on or off.
    pub fn check(&self) {
        if let Ok(wake) = self.wake.lock() {
            let _ = wake.send(());
        }
    }
}

struct Items {
    status: MenuItem<tauri::Wry>,
    dashboard: MenuItem<tauri::Wry>,
}

pub fn setup(app: &tauri::App) -> tauri::Result<()> {
    let handle = app.handle();
    let open = MenuItem::with_id(handle, "open", "打开 OpenVAIR", true, None::<&str>)?;
    let status = MenuItem::with_id(handle, "status", "bili：未开启", false, None::<&str>)?;
    let dashboard = MenuItem::with_id(handle, "dashboard", "打开 bili 面板", false, None::<&str>)?;
    let quit = MenuItem::with_id(handle, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(
        handle,
        &[
            &open,
            &PredefinedMenuItem::separator(handle)?,
            &status,
            &dashboard,
            &PredefinedMenuItem::separator(handle)?,
            &quit,
        ],
    )?;
    let mut tray = TrayIconBuilder::with_id(TRAY)
        .tooltip("OpenVAIR Desktop")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show(app),
            "dashboard" => {
                let (home, server) = places(app);
                let _ = open_dashboard(&home, &server);
            }
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;

    let (wake, sleep) = mpsc::channel();
    app.manage(Watch {
        error: Mutex::new(None),
        wake: Mutex::new(wake),
    });
    let handle = handle.clone();
    std::thread::spawn(move || supervise(&handle, &sleep, &Items { status, dashboard }));
    Ok(())
}

pub fn show(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Hides the window instead of quitting; on macOS the Dock icon goes too.
pub fn hide(window: &Window) {
    let _ = window.hide();
    #[cfg(target_os = "macos")]
    let _ = window
        .app_handle()
        .set_activation_policy(tauri::ActivationPolicy::Accessory);
}

/// Opens bili's web interface at the loopback origin the agents use, once
/// the proxy answers there.
pub fn open_dashboard(home: &Path, server: &url::Url) -> Result<()> {
    let status = bili::status(home, server)?;
    let origin = status
        .dashboard
        .ok_or_else(|| CoreError::Other("长对话压缩未开启".into()))?;
    if status.running != Some(true) {
        return Err(CoreError::Other(
            "bili 未运行，请在 OpenVAIR 托盘中检查 bili 状态".into(),
        ));
    }
    open::that_detached(&origin).map_err(|error| CoreError::Other(error.to_string()))
}

fn places(app: &AppHandle) -> (PathBuf, url::Url) {
    tauri::async_runtime::block_on(async {
        let state = app.state::<App>();
        let inner = state.0.lock().await;
        (inner.home.clone(), inner.server())
    })
}

/// Checks every 30 seconds, or when woken, that the proxy runs on each port
/// an agent uses, and starts it where it does not. Failures wait longer
/// each time, up to five minutes.
fn supervise(app: &AppHandle, sleep: &mpsc::Receiver<()>, items: &Items) {
    let (home, _) = places(app);
    // Earlier versions started the proxy from a Codex hook.
    let _ = bili::codex::forget_hook(&home);
    let mut wait = CHECK_EVERY;
    let mut synced: Option<Instant> = None;
    loop {
        let (home, server) = places(app);
        // Codex's newer catalog download is carried into the app's copy.
        let _ = agents::AgentId::Codex.refresh(&home);
        let started = bili::ports(&home, &server).and_then(|ports| {
            ports
                .iter()
                .try_for_each(|&port| bili::proxy::ensure(&home, port))
                .map(|()| ports)
        });
        wait = if started.is_ok() {
            CHECK_EVERY
        } else {
            (wait * 2).min(BACKOFF_LIMIT)
        };
        // A proxy that cannot reach bili is the user's to fix, so it is
        // reported without backing off.
        let outcome = started.and_then(|ports| route(&home, &server, &ports));
        let watch = app.state::<Watch>();
        if let Ok(mut error) = watch.error.lock() {
            *error = outcome.err().map(|error| error.to_string());
        }
        show_status(app, items, &home, &server, &watch);
        // After the proxy, which the agents need first.
        if synced.is_none_or(|at| at.elapsed() >= SYNC_EVERY) {
            tauri::async_runtime::block_on(crate::sync_models(&app.state::<App>()));
            synced = Some(Instant::now());
        }
        match sleep.recv_timeout(wait) {
            Ok(()) => wait = CHECK_EVERY,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Checks the route to bili for each port, through the system proxy too
/// when Codex, which follows it, is routed through bili.
fn route(home: &Path, server: &url::Url, ports: &[u16]) -> Result<()> {
    let system = bili::status(home, server).is_ok_and(|status| status.codex.enabled);
    ports
        .iter()
        .try_for_each(|&port| bili::proxy::check_route(port, system))
}

fn show_status(app: &AppHandle, items: &Items, home: &Path, server: &url::Url, watch: &Watch) {
    let Ok(status) = bili::status(home, server) else {
        return;
    };
    let status = watch.report(status);
    let label = match (status.running, &status.error) {
        (None, _) => "bili：未开启",
        (Some(_), Some(_)) => "bili：异常",
        (Some(true), None) => "bili：运行中",
        (Some(false), None) => "bili：未运行",
    };
    let _ = items.status.set_text(label);
    let _ = items.dashboard.set_enabled(status.running == Some(true));
    if let Some(tray) = app.tray_by_id(TRAY) {
        let tooltip = match &status.error {
            Some(error) => format!("OpenVAIR Desktop · {error}"),
            None => format!("OpenVAIR Desktop · {label}"),
        };
        let _ = tray.set_tooltip(Some(tooltip));
    }
}
