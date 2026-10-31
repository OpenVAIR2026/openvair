//! Codex has no native bili mode: `bili plugin install codex` only adds the
//! `[mcp_servers.bili]` block for bili's tools. OpenVAIR also points the `openvair`
//! provider at the proxy, which OpenVAIR Desktop keeps running (see
//! [`super::proxy`]). Earlier versions started the proxy from a SessionStart
//! hook instead, which Codex skips until the user trusts it; that hook is
//! removed wherever it is found.
use super::{AgentStatus, PLUGIN_TIMEOUT, bili_names, find, origin, proxy, run, wrapped_upstream};
use crate::{
    CoreError, Result,
    agents::{read_text, write_text},
};
use std::path::{Path, PathBuf};
use toml_edit::{DocumentMut, InlineTable, Item, Table, value};

const PROVIDER: &str = "openvair";
/// The script earlier versions' hook ran, which also marks that hook.
const SCRIPT_NAME: &str = "openvair-bili-start.mjs";

fn config_path(home: &Path) -> PathBuf {
    home.join(".codex/config.toml")
}

fn script_path(home: &Path) -> PathBuf {
    home.join(".codex").join(SCRIPT_NAME)
}

fn load(path: &Path) -> Result<DocumentMut> {
    read_text(path)?
        .unwrap_or_default()
        .parse::<DocumentMut>()
        .map_err(|_| CoreError::InvalidConfig(path.display().to_string()))
}

/// The `openvair` provider's base URL, while Codex uses that provider.
fn provider_url(doc: &DocumentMut) -> Option<&str> {
    if doc.get("model_provider").and_then(Item::as_str) != Some(PROVIDER) {
        return None;
    }
    doc.get("model_providers")?
        .get(PROVIDER)?
        .get("base_url")?
        .as_str()
}

fn is_ours(group: &Table) -> bool {
    group
        .get("hooks")
        .and_then(Item::as_array_of_tables)
        .into_iter()
        .flatten()
        .filter_map(|hook| hook.get("command").and_then(Item::as_str))
        .any(|command| command.contains(SCRIPT_NAME))
}

/// The status and the provider's base URL.
pub fn status(home: &Path, upstream: &str) -> Result<(AgentStatus, Option<String>)> {
    let doc = load(&config_path(home))?;
    let tools = doc
        .get("mcp_servers")
        .and_then(|servers| servers.get("bili"))
        .is_some();
    let url = provider_url(&doc).map(ToOwned::to_owned);
    let enabled = tools && url.as_deref().and_then(wrapped_upstream) == Some(upstream);
    Ok((
        AgentStatus {
            hooked: tools,
            enabled,
        },
        url,
    ))
}

/// Routes Codex through an installed bili to `upstream`, OpenVAIR's `/v1` URL,
/// once the proxy answers. The file is put back as it was if any step fails.
pub fn enable(home: &Path, upstream: &str) -> Result<()> {
    let bili = find(home, bili_names()).ok_or_else(|| CoreError::Other("请先安装 bili".into()))?;
    proxy::ensure(home, proxy::PORT)?;
    proxy::check_route(proxy::PORT, true)?;
    let path = config_path(home);
    let previous = read_text(&path)?;
    let outcome = run(
        home,
        &bili,
        &["plugin".into(), "install".into(), "codex".into()],
        &[],
        PLUGIN_TIMEOUT,
        "启用 bili",
    )
    .and_then(|_| configure(&path, upstream))
    .and_then(|_| forget_script(home));
    if outcome.is_err() {
        match previous {
            Some(text) => write_text(&path, &text)?,
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    outcome
}

/// Finishes what `bili plugin install codex` started.
fn configure(path: &Path, upstream: &str) -> Result<()> {
    let mut doc = load(path)?;
    if provider_url(&doc).is_none() {
        return Err(CoreError::Other("请先把 Codex 接入 OpenVAIR".into()));
    }
    let origin = proxy::origin(proxy::PORT);
    let server = doc
        .get_mut("mcp_servers")
        .and_then(|servers| servers.get_mut("bili"))
        .and_then(Item::as_table_like_mut)
        .ok_or_else(|| CoreError::Other("bili 没有在 Codex 中添加工具".into()))?;
    let mut env = InlineTable::new();
    env.insert("BILI_MCP_PROXY", origin.as_str().into());
    server.insert("env", value(env));
    // bili's tools only manage the conversation's context; without this,
    // Codex blocks them whenever it cannot ask, as in `codex exec`.
    server.insert("default_tools_approval_mode", value("approve"));
    remove_hook(&mut doc);
    doc["model_providers"][PROVIDER]["base_url"] = value(format!("{origin}/bili/{upstream}"));
    write_text(path, &doc.to_string())
}

/// Removes bili's tools and points Codex at OpenVAIR directly again. The npm
/// package stays installed, and the proxy keeps running for other agents.
pub fn disable(home: &Path, upstream: &str) -> Result<()> {
    if let Some(bili) = find(home, bili_names()) {
        run(
            home,
            &bili,
            &["plugin".into(), "remove".into(), "codex".into()],
            &[],
            PLUGIN_TIMEOUT,
            "移除 bili",
        )?;
    }
    let path = config_path(home);
    if !path.exists() {
        return forget_script(home);
    }
    let mut doc = load(&path)?;
    remove_hook(&mut doc);
    if provider_url(&doc).is_some_and(|url| origin(url).is_some()) {
        doc["model_providers"][PROVIDER]["base_url"] = value(upstream);
    }
    write_text(&path, &doc.to_string())?;
    forget_script(home)
}

/// Removes the hook and script earlier versions added, leaving the file
/// untouched when there is none.
pub fn forget_hook(home: &Path) -> Result<()> {
    let path = config_path(home);
    if let Some(text) = read_text(&path)?
        && let Ok(mut doc) = text.parse::<DocumentMut>()
        && remove_hook(&mut doc)
    {
        write_text(&path, &doc.to_string())?;
    }
    forget_script(home)
}

fn forget_script(home: &Path) -> Result<()> {
    match std::fs::remove_file(script_path(home)) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(CoreError::file(&script_path(home), error))
        }
        _ => Ok(()),
    }
}

/// Removes earlier versions' hook and any tables left empty; whether it
/// found one.
fn remove_hook(doc: &mut DocumentMut) -> bool {
    let Some(hooks) = doc.get_mut("hooks").and_then(Item::as_table_mut) else {
        return false;
    };
    let mut removed = false;
    if let Some(groups) = hooks
        .get_mut("SessionStart")
        .and_then(Item::as_array_of_tables_mut)
    {
        let before = groups.len();
        groups.retain(|group| !is_ours(group));
        removed = groups.len() != before;
        if groups.is_empty() {
            hooks.remove("SessionStart");
        }
    }
    if hooks.is_empty() {
        doc.remove("hooks");
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    const UPSTREAM: &str = "https://openvair.example.test/v1";
    const OLD_HOOK: &str = "[[hooks.SessionStart]]\n[[hooks.SessionStart.hooks]]\ntype = \"command\"\n\
         command = '\"/n/node\" \"/h/.codex/openvair-bili-start.mjs\" \"/b/dist/index.js\" 18787'\n\
         timeout = 30\n\n";

    fn write(home: &Path, text: &str) {
        let path = config_path(home);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn connected() -> String {
        format!(
            "# mine\nmodel = \"gpt\"\nmodel_provider = \"openvair\"\n\n\
             [model_providers.openvair]\nbase_url = \"{UPSTREAM}\"\n\n\
             [[hooks.SessionStart]]\n[[hooks.SessionStart.hooks]]\ntype = \"command\"\ncommand = \"echo mine\"\n\n\
             {OLD_HOOK}\
             [mcp_servers.bili]\ncommand = \"/n/node\"\nargs = [\"/b/dist/mcp.js\"]\n"
        )
    }

    #[test]
    fn routes_through_the_proxy_and_drops_the_old_hook() {
        let home = tempfile::tempdir().unwrap();
        write(home.path(), &connected());
        let path = config_path(home.path());
        configure(&path, UPSTREAM).unwrap();
        let (state, url) = status(home.path(), UPSTREAM).unwrap();
        assert!(state.hooked && state.enabled);
        assert_eq!(
            url.as_deref(),
            Some("http://127.0.0.1:18787/bili/https://openvair.example.test/v1")
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# mine\n"));
        assert!(text.contains("echo mine"));
        assert!(!text.contains(SCRIPT_NAME));
        assert!(text.contains(r#"BILI_MCP_PROXY = "http://127.0.0.1:18787""#));
        assert!(text.contains(r#"default_tools_approval_mode = "approve""#));
        assert!(
            !status(home.path(), "https://other.test/v1")
                .unwrap()
                .0
                .enabled
        );
    }

    #[test]
    fn needs_codex_on_vair_and_bili_tools() {
        let home = tempfile::tempdir().unwrap();
        let path = config_path(home.path());
        write(home.path(), "model_provider = \"openai\"\n");
        assert!(configure(&path, UPSTREAM).is_err());
        write(
            home.path(),
            &format!(
                "model_provider = \"openvair\"\n[model_providers.openvair]\nbase_url = \"{UPSTREAM}\"\n"
            ),
        );
        assert!(configure(&path, UPSTREAM).is_err());
        assert!(!status(home.path(), UPSTREAM).unwrap().0.hooked);
    }

    #[test]
    fn forgetting_the_hook_keeps_everything_else() {
        let home = tempfile::tempdir().unwrap();
        write(home.path(), &connected());
        std::fs::write(script_path(home.path()), "// old").unwrap();
        forget_hook(home.path()).unwrap();
        let text = std::fs::read_to_string(config_path(home.path())).unwrap();
        assert!(!text.contains(SCRIPT_NAME));
        assert!(text.contains("echo mine"));
        assert!(text.contains("[mcp_servers.bili]"));
        assert!(!script_path(home.path()).exists());

        // A file without the hook is left byte for byte.
        let mine = "model = \"gpt\"   # spacing kept\n";
        write(home.path(), mine);
        forget_hook(home.path()).unwrap();
        assert_eq!(
            std::fs::read_to_string(config_path(home.path())).unwrap(),
            mine
        );
    }

    #[test]
    fn removing_the_last_hook_drops_empty_tables() {
        let mut doc: DocumentMut = format!("model = \"gpt\"\n{OLD_HOOK}").parse().unwrap();
        assert!(remove_hook(&mut doc));
        assert_eq!(doc.to_string().trim_end(), "model = \"gpt\"");
        assert!(!remove_hook(&mut doc));
    }
}
