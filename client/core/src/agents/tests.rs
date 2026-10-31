use super::*;
use crate::state::{back_up, restore};
use serde_json::json;

const SECRET: &str = "openvair_test-secret-value";
const OTHER: &str = "openvair_other-secret-value";

fn server() -> Url {
    Url::parse("https://openvair.example.test").unwrap()
}

fn settings<'a>(server: &'a Url, secret: &'a str, model: &'a str) -> Settings<'a> {
    Settings {
        server,
        secret,
        model,
        models: &[],
        background_model: None,
        protocol: None,
    }
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn codex_keeps_user_config_and_restores_the_previous_provider() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(".codex/config.toml");
    write(
        &path,
        "# my settings\nmodel = \"o3\"\napproval_policy = \"on-request\"\n\n[model_providers.local]\nbase_url = \"http://localhost:1234/v1\"\n",
    );
    let server = server();
    let prior = AgentId::Codex
        .apply(home.path(), &settings(&server, SECRET, "gpt-5.1-codex"))
        .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("# my settings"));
    assert!(text.contains("approval_policy = \"on-request\""));
    assert!(text.contains("[model_providers.local]"));
    let parsed: toml_edit::DocumentMut = text.parse().unwrap();
    assert_eq!(parsed["model_provider"].as_str(), Some("openvair"));
    assert_eq!(
        parsed["model_providers"]["openvair"]["wire_api"].as_str(),
        Some("responses")
    );
    assert_eq!(
        parsed["model_providers"]["openvair"]["base_url"].as_str(),
        Some("https://openvair.example.test/v1")
    );

    let current = AgentId::Codex.read(home.path()).unwrap();
    assert_eq!(current.secret.as_deref(), Some(SECRET));
    assert_eq!(current.model.as_deref(), Some("gpt-5.1-codex"));

    AgentId::Codex.remove(home.path(), &prior).unwrap();
    let parsed: toml_edit::DocumentMut = std::fs::read_to_string(&path).unwrap().parse().unwrap();
    assert_eq!(parsed["model"].as_str(), Some("o3"));
    assert!(parsed.get("model_provider").is_none());
    assert!(parsed["model_providers"].get("openvair").is_none());
    assert!(parsed["model_providers"].get("local").is_some());
    assert!(AgentId::Codex.read(home.path()).unwrap().secret.is_none());
}

#[test]
fn claude_code_switches_keys_and_puts_back_earlier_env_values() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(".claude/settings.json");
    write(
        &path,
        r#"{"permissions":{"allow":["Bash(ls)"]},"env":{"ANTHROPIC_MODEL":"claude-opus","DEBUG":"1"}}"#,
    );
    let server = server();
    let first = AgentId::ClaudeCode
        .apply(home.path(), &settings(&server, SECRET, "claude-sonnet-4-5"))
        .unwrap();
    AgentId::ClaudeCode
        .apply(home.path(), &settings(&server, OTHER, "claude-sonnet-4-5"))
        .unwrap();
    let current = AgentId::ClaudeCode.read(home.path()).unwrap();
    assert_eq!(current.secret.as_deref(), Some(OTHER));
    assert_eq!(
        current.base_url.as_deref(),
        Some("https://openvair.example.test")
    );

    // The first connect's prior holds the user's own values.
    AgentId::ClaudeCode.remove(home.path(), &first).unwrap();
    let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(value["env"]["ANTHROPIC_MODEL"], "claude-opus");
    assert_eq!(value["env"]["DEBUG"], "1");
    assert!(value["env"].get("ANTHROPIC_AUTH_TOKEN").is_none());
    assert_eq!(value["permissions"]["allow"][0], "Bash(ls)");
}

#[test]
fn gemini_cli_sets_env_and_api_key_sign_in() {
    let home = tempfile::tempdir().unwrap();
    let env = home.path().join(".gemini/.env");
    write(&env, "# keep\nexport GEMINI_API_KEY='old'\nOTHER=1\n");
    let server = server();
    let prior = AgentId::GeminiCli
        .apply(home.path(), &settings(&server, SECRET, "gemini-2.5-pro"))
        .unwrap();
    let text = std::fs::read_to_string(&env).unwrap();
    assert!(text.contains("# keep") && text.contains("OTHER=1"));
    assert!(text.contains(&format!("GEMINI_API_KEY=\"{SECRET}\"")));
    assert_eq!(text.matches("GEMINI_API_KEY").count(), 1);
    let settings: Value = serde_json::from_str(
        &std::fs::read_to_string(home.path().join(".gemini/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        settings["security"]["auth"]["selectedType"],
        "gemini-api-key"
    );

    AgentId::GeminiCli.remove(home.path(), &prior).unwrap();
    let text = std::fs::read_to_string(&env).unwrap();
    assert!(text.contains("GEMINI_API_KEY=\"old\""));
    assert!(!text.contains("GOOGLE_GEMINI_BASE_URL"));
}

#[test]
fn invalid_config_is_reported_not_overwritten() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(".claude/settings.json");
    write(&path, "{ not json");
    let server = server();
    assert!(matches!(
        AgentId::ClaudeCode.apply(home.path(), &settings(&server, SECRET, "m")),
        Err(crate::CoreError::InvalidConfig(_))
    ));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
}

#[test]
fn backups_restore_files_and_remove_new_ones() {
    let home = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let files = AgentId::GeminiCli.files(home.path());
    write(&files[1], "{\"theme\":\"dark\"}\n");
    let backups = back_up(&files, data.path(), "gemini-cli").unwrap();
    let server = server();
    AgentId::GeminiCli
        .apply(home.path(), &settings(&server, SECRET, "gemini-2.5-pro"))
        .unwrap();
    restore(&backups).unwrap();
    assert!(!files[0].exists());
    assert_eq!(
        std::fs::read_to_string(&files[1]).unwrap(),
        "{\"theme\":\"dark\"}\n"
    );
}

#[test]
fn preview_masks_secrets() {
    let home = tempfile::tempdir().unwrap();
    let server = server();
    AgentId::ClaudeCode
        .apply(home.path(), &settings(&server, SECRET, "a"))
        .unwrap();
    let changes = AgentId::ClaudeCode
        .preview(home.path(), &settings(&server, "openvair_K9x", "b"))
        .unwrap();
    let text = serde_json::to_string(&changes).unwrap();
    assert!(!text.contains(SECRET));
    assert!(text.contains("openvair_K9x…••••"));
}

#[test]
fn codex_notices_chatgpt_sign_in() {
    let home = tempfile::tempdir().unwrap();
    assert_eq!(AgentId::Codex.notice(home.path()), None);
    let path = home.path().join(".codex/auth.json");
    write(
        &path,
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-test"}"#,
    );
    assert_eq!(AgentId::Codex.notice(home.path()), None);
    write(&path, r#"{"auth_mode":"chatgpt","tokens":{}}"#);
    assert_eq!(
        AgentId::Codex.notice(home.path()),
        Some("codex-chatgpt-login")
    );
    write(&path, "not json");
    assert_eq!(AgentId::Codex.notice(home.path()), None);
}

#[test]
fn claude_code_keeps_routing_through_bili_when_switching_keys() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(".claude/settings.json");
    write(
        &path,
        r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:41234/bili/https://old.example.test"}}"#,
    );
    let server = server();
    let changes = AgentId::ClaudeCode
        .preview(
            home.path(),
            &settings(&server, "openvair_test", "claude-sonnet-4-5"),
        )
        .unwrap();
    let wrapped = "http://127.0.0.1:41234/bili/https://openvair.example.test";
    assert_eq!(changes[0].after.as_deref(), Some(wrapped));
    let prior = AgentId::ClaudeCode
        .apply(home.path(), &settings(&server, SECRET, "claude-sonnet-4-5"))
        .unwrap();
    let current = AgentId::ClaudeCode.read(home.path()).unwrap();
    assert_eq!(current.base_url.as_deref(), Some(wrapped));
    AgentId::ClaudeCode.remove(home.path(), &prior).unwrap();
    assert_eq!(
        AgentId::ClaudeCode
            .read(home.path())
            .unwrap()
            .base_url
            .as_deref(),
        Some("http://127.0.0.1:41234/bili/https://old.example.test")
    );
}

fn codex_cache(home: &Path, etag: &str) {
    write(
        &home.join(".codex/models_cache.json"),
        &json!({
            "etag": etag,
            "fetched_at": "2026-10-15T00:00:00Z",
            "client_version": "0.162.1",
            "models": [
                {"slug": "gpt-b", "visibility": "list", "priority": 2,
                 "model_messages": {"instructions_template": etag},
                 "context_window": 272000, "input_modalities": ["text", "image"]},
                {"slug": "gpt-a", "visibility": "list", "priority": 1,
                 "model_messages": {"instructions_template": etag},
                 "service_tiers": [{"id": "priority"}], "shell_type": "unified_exec",
                 "tool_mode": "code_mode_only", "context_window": 272000,
                 "input_modalities": ["text", "image"]},
            ],
        })
        .to_string(),
    );
}

fn catalog(home: &Path) -> serde_json::Map<String, Value> {
    read_json(&home.join(".codex/openvair-models.json")).unwrap()
}

fn slugs(catalog: &serde_json::Map<String, Value>) -> Vec<String> {
    catalog["models"]
        .as_array()
        .unwrap()
        .iter()
        .map(|model| model["slug"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn codex_lists_key_models_it_does_not_know() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(".codex/config.toml");
    write(&path, "model = \"o3\"\n\n[desktop]\nfoo = 1\n");
    codex_cache(home.path(), "one");
    let server = server();
    let models = [
        "deepseek-v4-pro".to_owned(),
        "gpt-a".to_owned(),
        "gpt-4o".to_owned(),
    ];
    let prior = AgentId::Codex
        .apply(
            home.path(),
            &Settings {
                models: &models,
                ..settings(&server, SECRET, "kimi-k3")
            },
        )
        .unwrap();

    let text = std::fs::read_to_string(&path).unwrap();
    let parsed: toml_edit::DocumentMut = text.parse().unwrap();
    let file = home.path().join(".codex/openvair-models.json");
    assert_eq!(
        parsed["model_catalog_json"].as_str(),
        Some(file.to_str().unwrap())
    );
    // Codex ignores a top-level key written under a table.
    assert!(text.find("model_catalog_json").unwrap() < text.find("[desktop]").unwrap());

    let listed = catalog(home.path());
    assert_eq!(
        slugs(&listed),
        ["gpt-b", "gpt-a", "deepseek-v4-pro", "kimi-k3"]
    );
    let deepseek = &listed["models"][2];
    assert_eq!(deepseek["model_messages"]["instructions_template"], "one");
    assert_eq!(deepseek["context_window"], 1_048_576);
    assert_eq!(deepseek["shell_type"], "shell_command");
    assert_eq!(deepseek["tool_mode"], Value::Null);
    assert_eq!(deepseek["supports_search_tool"], false);
    assert_eq!(deepseek["input_modalities"], json!(["text"]));
    assert_eq!(deepseek["priority"], 3);
    assert!(deepseek.get("service_tiers").is_none());
    assert_eq!(listed["models"][3]["context_window"], 262_144);

    // A newer download from Codex is carried over.
    codex_cache(home.path(), "two");
    assert!(AgentId::Codex.refresh(home.path()).unwrap());
    let listed = catalog(home.path());
    assert_eq!(
        slugs(&listed),
        ["gpt-b", "gpt-a", "deepseek-v4-pro", "kimi-k3"]
    );
    assert_eq!(
        listed["models"][2]["model_messages"]["instructions_template"],
        "two"
    );
    assert!(!AgentId::Codex.refresh(home.path()).unwrap());

    AgentId::Codex.remove(home.path(), &prior).unwrap();
    let parsed: toml_edit::DocumentMut = std::fs::read_to_string(&path).unwrap().parse().unwrap();
    assert!(parsed.get("model_catalog_json").is_none());
    assert!(!file.exists());
    assert!(!AgentId::Codex.refresh(home.path()).unwrap());
}

fn shipped(slugs: &[(&str, i64)]) -> Value {
    slugs
        .iter()
        .map(|(slug, priority)| {
            json!({"slug": slug, "visibility": "list", "priority": priority,
                   "model_messages": {"instructions_template": "shipped"}})
        })
        .collect()
}

#[test]
fn codex_takes_new_models_from_the_release_it_ships() {
    let home = tempfile::tempdir().unwrap();
    codex_cache(home.path(), "one");
    codex_bundled::fake("0.166.0-alpha.1", shipped(&[("gpt-a", 1), ("gpt-c", 7)]));
    let server = server();
    let models = ["deepseek-v4-pro".to_owned(), "gpt-c".to_owned()];
    AgentId::Codex
        .apply(
            home.path(),
            &Settings {
                models: &models,
                ..settings(&server, SECRET, "kimi-k3")
            },
        )
        .unwrap();
    // The release is newer than the download, so its entries win.
    let listed = catalog(home.path());
    assert_eq!(
        slugs(&listed),
        ["gpt-a", "gpt-c", "gpt-b", "deepseek-v4-pro", "kimi-k3"]
    );
    assert_eq!(
        listed["models"][0]["model_messages"]["instructions_template"],
        "shipped"
    );
    assert_eq!(
        listed["models"][3]["model_messages"]["instructions_template"],
        "shipped"
    );
    assert_eq!(listed["models"][3]["priority"], 8);
    assert!(!AgentId::Codex.refresh(home.path()).unwrap());

    // After a change to an older Codex, the download's entries win.
    codex_bundled::fake("0.160.0", shipped(&[("gpt-a", 1), ("gpt-c", 7)]));
    assert!(AgentId::Codex.refresh(home.path()).unwrap());
    let listed = catalog(home.path());
    assert_eq!(
        slugs(&listed),
        ["gpt-b", "gpt-a", "gpt-c", "deepseek-v4-pro", "kimi-k3"]
    );
    assert_eq!(
        listed["models"][1]["model_messages"]["instructions_template"],
        "one"
    );
    assert!(!AgentId::Codex.refresh(home.path()).unwrap());
}

#[test]
fn codex_builds_a_catalog_without_a_download() {
    let home = tempfile::tempdir().unwrap();
    codex_bundled::fake("0.164.0", shipped(&[("gpt-a", 1)]));
    let server = server();
    AgentId::Codex
        .apply(home.path(), &settings(&server, SECRET, "deepseek-v4-pro"))
        .unwrap();
    assert_eq!(slugs(&catalog(home.path())), ["gpt-a", "deepseek-v4-pro"]);
}

#[test]
fn codex_needs_no_catalog_for_models_it_knows() {
    let home = tempfile::tempdir().unwrap();
    codex_cache(home.path(), "one");
    let server = server();
    let models = ["gpt-a".to_owned(), "gpt-image-2".to_owned()];
    AgentId::Codex
        .apply(
            home.path(),
            &Settings {
                models: &models,
                ..settings(&server, SECRET, "gpt-4o")
            },
        )
        .unwrap();
    let text = std::fs::read_to_string(home.path().join(".codex/config.toml")).unwrap();
    assert!(!text.contains("model_catalog_json"));
    assert!(!home.path().join(".codex/openvair-models.json").exists());
}

#[test]
fn codex_keeps_the_users_own_catalog() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(".codex/config.toml");
    write(&path, "model_catalog_json = \"/mine.json\"\n");
    codex_cache(home.path(), "one");
    let server = server();
    let prior = AgentId::Codex
        .apply(home.path(), &settings(&server, SECRET, "deepseek-v4-pro"))
        .unwrap();
    assert!(!home.path().join(".codex/openvair-models.json").exists());
    AgentId::Codex.remove(home.path(), &prior).unwrap();
    let parsed: toml_edit::DocumentMut = std::fs::read_to_string(&path).unwrap().parse().unwrap();
    assert_eq!(parsed["model_catalog_json"].as_str(), Some("/mine.json"));
}

#[test]
fn claude_code_lists_the_keys_other_models_in_the_picker() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(".claude/settings.json");
    let server = server();
    let models = [
        "claude-sonnet-4-5".to_owned(),
        "deepseek-v4-pro".to_owned(),
        "kimi-k3".to_owned(),
    ];
    let mut chosen = settings(&server, SECRET, "deepseek-v4-flash");
    chosen.models = &models;
    let prior = AgentId::ClaudeCode.apply(home.path(), &chosen).unwrap();
    let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let rows: Vec<&str> = value["modelPicker"]["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["model"].as_str().unwrap())
        .collect();
    assert_eq!(rows, ["deepseek-v4-flash", "deepseek-v4-pro", "kimi-k3"]);
    assert!(value["modelPicker"].get("replaceBuiltInOptions").is_none());

    // Switching to a key with only Claude models drops the rows.
    AgentId::ClaudeCode
        .apply(home.path(), &settings(&server, OTHER, "claude-sonnet-4-5"))
        .unwrap();
    let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(value.get("modelPicker").is_none());

    AgentId::ClaudeCode.apply(home.path(), &chosen).unwrap();
    AgentId::ClaudeCode.remove(home.path(), &prior).unwrap();
    let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(value.get("modelPicker").is_none());
}

#[test]
fn claude_code_keeps_the_users_own_picker() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(".claude/settings.json");
    let own = r#"{"options":[{"model":"claude-opus-4-8","label":"Opus (production)"}]}"#;
    write(&path, &format!(r#"{{"modelPicker":{own}}}"#));
    let server = server();
    let models = ["deepseek-v4-pro".to_owned()];
    let mut chosen = settings(&server, SECRET, "deepseek-v4-pro");
    chosen.models = &models;
    let prior = AgentId::ClaudeCode.apply(home.path(), &chosen).unwrap();
    let expected: Value = serde_json::from_str(own).unwrap();
    let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(value["modelPicker"], expected);
    AgentId::ClaudeCode.remove(home.path(), &prior).unwrap();
    let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(value["modelPicker"], expected);
}

#[test]
fn syncing_follows_the_keys_models_and_writes_only_changes() {
    let home = tempfile::tempdir().unwrap();
    codex_cache(home.path(), "one");
    let server = server();
    let first = ["deepseek-v4-pro".to_owned()];
    for agent in [AgentId::Codex, AgentId::ClaudeCode] {
        agent
            .apply(
                home.path(),
                &Settings {
                    models: &first,
                    ..settings(&server, SECRET, "deepseek-v4-pro")
                },
            )
            .unwrap();
        assert!(
            !agent
                .sync_models(home.path(), "deepseek-v4-pro", &first)
                .unwrap()
        );
    }

    let later = ["deepseek-v4-pro".to_owned(), "kimi-k3".to_owned()];
    assert!(
        AgentId::Codex
            .sync_models(home.path(), "deepseek-v4-pro", &later)
            .unwrap()
    );
    assert_eq!(
        slugs(&catalog(home.path())),
        ["gpt-b", "gpt-a", "deepseek-v4-pro", "kimi-k3"]
    );
    assert!(
        AgentId::ClaudeCode
            .sync_models(home.path(), "deepseek-v4-pro", &later)
            .unwrap()
    );
    let path = home.path().join(".claude/settings.json");
    let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(value["modelPicker"]["options"][1]["model"], "kimi-k3");
    assert_eq!(value["env"]["ANTHROPIC_AUTH_TOKEN"], SECRET);

    // Only Claude models left: OpenVAIR's rows go, the connection stays.
    let claude = ["claude-sonnet-4-5".to_owned()];
    assert!(
        AgentId::ClaudeCode
            .sync_models(home.path(), "claude-sonnet-4-5", &claude)
            .unwrap()
    );
    let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(value.get("modelPicker").is_none());
    assert_eq!(value["env"]["ANTHROPIC_AUTH_TOKEN"], SECRET);
}

#[test]
fn syncing_leaves_files_vair_did_not_write() {
    let home = tempfile::tempdir().unwrap();
    let models = ["deepseek-v4-pro".to_owned()];
    // Not connected: nothing is created.
    assert!(
        !AgentId::ClaudeCode
            .sync_models(home.path(), "deepseek-v4-pro", &models)
            .unwrap()
    );
    assert!(
        !AgentId::Codex
            .sync_models(home.path(), "deepseek-v4-pro", &models)
            .unwrap()
    );
    assert!(!home.path().join(".claude/settings.json").exists());

    let path = home.path().join(".codex/config.toml");
    write(&path, "model = \"o3\"\n");
    codex_cache(home.path(), "one");
    assert!(
        !AgentId::Codex
            .sync_models(home.path(), "deepseek-v4-pro", &models)
            .unwrap()
    );
    assert!(!home.path().join(".codex/openvair-models.json").exists());

    let own = "model_catalog_json = \"/mine.json\"\nmodel_provider = \"openvair\"\n";
    write(&path, own);
    assert!(
        !AgentId::Codex
            .sync_models(home.path(), "deepseek-v4-pro", &models)
            .unwrap()
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), own);
}

#[test]
fn claude_code_points_aliases_at_a_key_without_claude_models() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(".claude/settings.json");
    write(
        &path,
        r#"{"env":{"ANTHROPIC_DEFAULT_HAIKU_MODEL":"claude-haiku-4-5"}}"#,
    );
    let server = server();
    let models = ["deepseek-v4-flash".to_owned(), "deepseek-v4-pro".to_owned()];
    assert!(
        AgentId::ClaudeCode
            .asks_background_model(home.path(), &models)
            .unwrap()
    );
    let chosen = Settings {
        models: &models,
        background_model: Some("deepseek-v4-flash"),
        ..settings(&server, SECRET, "deepseek-v4-pro")
    };
    let changes = AgentId::ClaudeCode.preview(home.path(), &chosen).unwrap();
    let haiku = changes
        .iter()
        .find(|change| change.field == "env.ANTHROPIC_DEFAULT_HAIKU_MODEL")
        .unwrap();
    assert_eq!(haiku.before.as_deref(), Some("claude-haiku-4-5"));
    assert_eq!(haiku.after.as_deref(), Some("deepseek-v4-flash"));
    assert!(
        changes
            .iter()
            .any(|change| change.field.starts_with("modelPicker")
                && change.after.as_deref() == Some("deepseek-v4-flash, deepseek-v4-pro"))
    );

    let prior = AgentId::ClaudeCode.apply(home.path(), &chosen).unwrap();
    let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        value["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"],
        "deepseek-v4-flash"
    );
    assert_eq!(
        value["env"]["ANTHROPIC_DEFAULT_SONNET_MODEL"],
        "deepseek-v4-pro"
    );
    assert_eq!(
        value["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"],
        "deepseek-v4-pro"
    );

    AgentId::ClaudeCode.remove(home.path(), &prior).unwrap();
    let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        value["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"],
        "claude-haiku-4-5"
    );
    assert!(value["env"].get("ANTHROPIC_DEFAULT_SONNET_MODEL").is_none());
    assert!(value["env"].get("ANTHROPIC_DEFAULT_OPUS_MODEL").is_none());
}

#[test]
fn claude_code_leaves_aliases_a_claude_key_can_serve() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(".claude/settings.json");
    write(
        &path,
        r#"{"env":{"ANTHROPIC_DEFAULT_HAIKU_MODEL":"claude-haiku-4-5","ANTHROPIC_DEFAULT_OPUS_MODEL":"kimi-k3"}}"#,
    );
    let server = server();
    let models = ["claude-sonnet-4-5".to_owned(), "deepseek-v4-pro".to_owned()];
    assert!(
        !AgentId::ClaudeCode
            .asks_background_model(home.path(), &models)
            .unwrap()
    );
    let chosen = Settings {
        models: &models,
        ..settings(&server, SECRET, "claude-sonnet-4-5")
    };
    AgentId::ClaudeCode.apply(home.path(), &chosen).unwrap();
    let value: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        value["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"],
        "claude-haiku-4-5"
    );
    // The key does not list kimi-k3, so opus follows the chosen model.
    assert_eq!(
        value["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"],
        "claude-sonnet-4-5"
    );
    assert!(value["env"].get("ANTHROPIC_DEFAULT_SONNET_MODEL").is_none());
}

#[test]
fn codex_preview_names_the_models_the_catalog_adds() {
    let home = tempfile::tempdir().unwrap();
    write(&home.path().join(".codex/config.toml"), "model = \"o3\"\n");
    codex_cache(home.path(), "one");
    let server = server();
    let models = ["deepseek-v4-pro".to_owned(), "gpt-4o".to_owned()];
    let changes = AgentId::Codex
        .preview(
            home.path(),
            &Settings {
                models: &models,
                ..settings(&server, "openvair_test", "kimi-k3")
            },
        )
        .unwrap();
    let catalog = changes
        .iter()
        .find(|change| change.field == "model_catalog_json")
        .unwrap();
    assert!(catalog.before.is_none());
    let after = catalog.after.as_deref().unwrap();
    assert!(
        after.ends_with("（加入 deepseek-v4-pro, kimi-k3）"),
        "{after}"
    );

    // Nothing to add, nothing to show.
    let changes = AgentId::Codex
        .preview(home.path(), &settings(&server, "openvair_test", "gpt-a"))
        .unwrap();
    assert!(
        !changes
            .iter()
            .any(|change| change.field == "model_catalog_json")
    );
}

fn json_file(path: &Path) -> serde_json::Map<String, Value> {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn qwen_code_adds_its_own_rows_and_restores_the_old_selection() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(".qwen/settings.json");
    write(
        &path,
        r#"{"theme":"dark","modelProviders":{"openai":[{"id":"qwen3-coder-plus","envKey":"DASHSCOPE_API_KEY","baseUrl":"https://dashscope.aliyuncs.com/compatible-mode/v1"}]},"security":{"auth":{"selectedType":"qwen-oauth"}}}"#,
    );
    let server = server();
    let models = vec!["glm-5".to_owned(), "kimi-k2".to_owned()];
    let mut chosen = settings(&server, SECRET, "deepseek-v4");
    chosen.models = &models;
    chosen.protocol = Some(Protocol::Anthropic);
    let prior = AgentId::QwenCode.apply(home.path(), &chosen).unwrap();
    let config = json_file(&path);
    assert_eq!(config["theme"], "dark");
    assert_eq!(
        config["modelProviders"]["openai"].as_array().unwrap().len(),
        1
    );
    let rows = config["modelProviders"]["anthropic"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0]["id"], "deepseek-v4");
    assert_eq!(rows[0]["baseUrl"], "https://openvair.example.test");
    assert_eq!(config["env"]["OPENVAIR_API_KEY"], SECRET);
    assert_eq!(config["security"]["auth"]["selectedType"], "anthropic");
    assert_eq!(config["model"]["name"], "deepseek-v4");

    let current = AgentId::QwenCode.read(home.path()).unwrap();
    assert_eq!(current.secret.as_deref(), Some(SECRET));
    assert_eq!(current.model.as_deref(), Some("deepseek-v4"));

    assert!(
        AgentId::QwenCode
            .sync_models(home.path(), "deepseek-v4", &[])
            .unwrap()
    );
    let config = json_file(&path);
    assert_eq!(
        config["modelProviders"]["anthropic"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    AgentId::QwenCode.remove(home.path(), &prior).unwrap();
    let config = json_file(&path);
    assert_eq!(config["security"]["auth"]["selectedType"], "qwen-oauth");
    assert!(config.get("env").is_none());
    assert!(config.get("model").is_none());
    assert!(config["modelProviders"].get("anthropic").is_none());
    assert_eq!(
        config["modelProviders"]["openai"].as_array().unwrap().len(),
        1
    );
}

#[test]
fn kimi_cli_keeps_comments_and_restores_the_default_model() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(".kimi/config.toml");
    write(
        &path,
        "# mine\ndefault_model = \"kimi-for-coding\"\n\n[providers.kimi]\ntype = \"kimi\"\n\n[models.kimi-for-coding]\nprovider = \"kimi\"\nmodel = \"kimi-for-coding\"\nmax_context_size = 262144\n",
    );
    let server = server();
    let models = vec!["glm-5".to_owned()];
    let mut chosen = settings(&server, SECRET, "deepseek-v4");
    chosen.models = &models;
    let prior = AgentId::KimiCli.apply(home.path(), &chosen).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("# mine"));
    let parsed: toml_edit::DocumentMut = text.parse().unwrap();
    assert_eq!(
        parsed["default_model"].as_str(),
        Some("openvair/deepseek-v4")
    );
    assert_eq!(
        parsed["providers"]["openvair"]["base_url"].as_str(),
        Some("https://openvair.example.test/v1")
    );
    assert_eq!(
        parsed["providers"]["openvair"]["type"].as_str(),
        Some("openai_legacy")
    );
    assert_eq!(
        parsed["models"]["openvair/glm-5"]["max_context_size"].as_integer(),
        Some(131_072)
    );

    let current = AgentId::KimiCli.read(home.path()).unwrap();
    assert_eq!(current.secret.as_deref(), Some(SECRET));
    assert_eq!(current.model.as_deref(), Some("deepseek-v4"));

    AgentId::KimiCli.remove(home.path(), &prior).unwrap();
    let parsed: toml_edit::DocumentMut = std::fs::read_to_string(&path).unwrap().parse().unwrap();
    assert_eq!(parsed["default_model"].as_str(), Some("kimi-for-coding"));
    assert!(parsed["providers"].get("openvair").is_none());
    assert!(parsed["models"].get("openvair/glm-5").is_none());
    assert!(parsed["models"].get("kimi-for-coding").is_some());
}

#[test]
fn opencode_adds_a_provider_and_leaves_jsonc_alone() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(".config/opencode/opencode.json");
    let server = server();
    let mut chosen = settings(&server, SECRET, "glm-5");
    chosen.protocol = Some(Protocol::Anthropic);
    let prior = AgentId::OpenCode.apply(home.path(), &chosen).unwrap();
    let config = json_file(&path);
    assert_eq!(config["$schema"], "https://opencode.ai/config.json");
    assert_eq!(config["model"], "openvair/glm-5");
    assert_eq!(config["provider"]["openvair"]["npm"], "@ai-sdk/anthropic");
    assert_eq!(
        config["provider"]["openvair"]["options"]["baseURL"],
        "https://openvair.example.test/v1"
    );
    let current = AgentId::OpenCode.read(home.path()).unwrap();
    assert_eq!(current.secret.as_deref(), Some(SECRET));
    assert_eq!(current.model.as_deref(), Some("glm-5"));

    AgentId::OpenCode.remove(home.path(), &prior).unwrap();
    let config = json_file(&path);
    assert!(config.get("provider").is_none());
    assert!(config.get("model").is_none());

    write(&path.with_extension("jsonc"), "// mine\n{}\n");
    assert_eq!(
        AgentId::OpenCode.notice(home.path()),
        Some("opencode-jsonc")
    );
    assert!(AgentId::OpenCode.apply(home.path(), &chosen).is_err());
}

const ZCODE: &str = r#"{"schemaVersion":1,"config":{"providerOrder":["zhipu"],"providerConfigRules":{"providerRules":[{"providerId":"zhipu","providerName":"Zhipu","enabled":true}]},"modelConfigRules":{"providerModelRules":[]},"defaultModelSelection":{"providerId":"zhipu","modelId":"glm-5"}}}"#;

#[test]
fn zcode_adds_a_provider_rule_and_restores_the_selection() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(".zcode/v2/provider_config.json");
    let server = server();
    let chosen = settings(&server, SECRET, "deepseek-v4");
    assert_eq!(AgentId::ZCode.notice(home.path()), Some("zcode-open-first"));
    assert!(AgentId::ZCode.apply(home.path(), &chosen).is_err());

    write(&path, ZCODE);
    assert_eq!(AgentId::ZCode.notice(home.path()), None);
    let prior = AgentId::ZCode.apply(home.path(), &chosen).unwrap();
    let config = json_file(&path);
    assert_eq!(
        config["config"]["providerOrder"],
        json!(["openvair", "zhipu"])
    );
    let rules = config["config"]["providerConfigRules"]["providerRules"]
        .as_array()
        .unwrap();
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[1]["config"]["api"]["type"], "openai-chat-completions");
    assert_eq!(
        rules[1]["config"]["api"]["baseUrl"],
        "https://openvair.example.test/v1"
    );
    assert_eq!(rules[1]["config"]["access"]["type"], "api-key");
    assert_eq!(
        config["config"]["defaultModelSelection"],
        json!({"providerId": "openvair", "modelId": "deepseek-v4"})
    );
    let current = AgentId::ZCode.read(home.path()).unwrap();
    assert_eq!(current.secret.as_deref(), Some(SECRET));
    assert_eq!(current.model.as_deref(), Some("deepseek-v4"));

    AgentId::ZCode.remove(home.path(), &prior).unwrap();
    assert_eq!(
        json_file(&path),
        serde_json::from_str::<serde_json::Map<String, Value>>(ZCODE).unwrap()
    );
}

#[test]
fn zcode_leaves_unknown_versions_and_moved_data_alone() {
    let home = tempfile::tempdir().unwrap();
    let path = home.path().join(".zcode/v2/provider_config.json");
    let server = server();
    let chosen = settings(&server, SECRET, "deepseek-v4");
    write(
        &path,
        &ZCODE.replace("\"schemaVersion\":1", "\"schemaVersion\":2"),
    );
    assert!(AgentId::ZCode.apply(home.path(), &chosen).is_err());

    write(&path, ZCODE);
    write(
        &home.path().join(".zcode/v2/setting.json"),
        r#"{"dataBaseDir":"/Volumes/data/zcode"}"#,
    );
    assert_eq!(AgentId::ZCode.notice(home.path()), Some("zcode-moved"));
    assert!(AgentId::ZCode.apply(home.path(), &chosen).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), ZCODE);
}

const DSH_PATCH: &str = "# my rows\n- id: llm-deepseek\n  name: '@deepseek-ai/dsh-llm-deepseek-api-key'\n  config:\n    reasoningEffort: high\n";

#[test]
fn deepseek_harness_appends_a_block_and_drops_it_on_disconnect() {
    let home = tempfile::tempdir().unwrap();
    let patch = home.path().join(".dsh/cordis.patch.yml");
    let env = home.path().join(".dsh/.env");
    write(&patch, DSH_PATCH);
    write(&env, "DEEPSEEK_API_KEY=sk-own\n");
    let server = server();
    let models = vec!["deepseek-flash".to_owned(), "deepseek-v4-pro".to_owned()];
    let mut chosen = settings(&server, SECRET, "deepseek-v4-pro");
    chosen.models = &models;

    let prior = AgentId::DeepSeekHarness
        .apply(home.path(), &chosen)
        .unwrap();
    let text = std::fs::read_to_string(&patch).unwrap();
    assert!(text.starts_with(DSH_PATCH));
    assert!(text.contains("    baseURL: 'https://openvair.example.test'\n"));
    assert!(text.contains("    apiKeyEnv: OPENVAIR_API_KEY\n"));
    assert!(text.contains("      - id: 'deepseek-flash'\n        name: DeepSeek-V4.1-Flash\n"));
    assert!(text.contains("    model: 'deepseek-v4-pro'\n"));
    let env_text = std::fs::read_to_string(&env).unwrap();
    assert!(env_text.contains("DEEPSEEK_API_KEY=sk-own"));
    assert!(env_text.contains(&format!("OPENVAIR_API_KEY=\"{SECRET}\"")));

    let current = AgentId::DeepSeekHarness.read(home.path()).unwrap();
    assert_eq!(
        current.base_url.as_deref(),
        Some("https://openvair.example.test")
    );
    assert_eq!(current.secret.as_deref(), Some(SECRET));
    assert_eq!(current.model.as_deref(), Some("deepseek-v4-pro"));

    let only = vec!["deepseek-flash".to_owned()];
    assert!(
        AgentId::DeepSeekHarness
            .sync_models(home.path(), "deepseek-flash", &only)
            .unwrap()
    );
    assert!(
        !AgentId::DeepSeekHarness
            .sync_models(home.path(), "deepseek-flash", &only)
            .unwrap()
    );

    AgentId::DeepSeekHarness
        .remove(home.path(), &prior)
        .unwrap();
    assert_eq!(std::fs::read_to_string(&patch).unwrap(), DSH_PATCH);
    assert!(
        !std::fs::read_to_string(&env)
            .unwrap()
            .contains("OPENVAIR_API_KEY")
    );
}

#[test]
fn deepseek_harness_keeps_the_patch_bootable() {
    let home = tempfile::tempdir().unwrap();
    let patch = home.path().join(".dsh/cordis.patch.yml");
    let server = server();
    let chosen = settings(&server, SECRET, "deepseek-flash");
    let prior = AgentId::DeepSeekHarness
        .apply(home.path(), &chosen)
        .unwrap();
    AgentId::DeepSeekHarness
        .remove(home.path(), &prior)
        .unwrap();
    assert_eq!(std::fs::read_to_string(&patch).unwrap(), "[]\n");

    write(&patch, "[]\n");
    AgentId::DeepSeekHarness
        .apply(home.path(), &chosen)
        .unwrap();
    assert!(!std::fs::read_to_string(&patch).unwrap().contains("[]"));

    write(&patch, "rows: {}\n");
    assert!(
        AgentId::DeepSeekHarness
            .apply(home.path(), &chosen)
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(&patch).unwrap(), "rows: {}\n");
}

#[test]
fn a_configuration_folder_alone_is_not_an_install() {
    let home = tempfile::tempdir().unwrap();
    let traces = AgentId::OpenCode.meta().traces;
    write(&home.path().join(".config/opencode/opencode.json"), "{}");
    assert!(!super::detect::installed(
        home.path(),
        &["openvair-test-none"],
        &traces[..1]
    ));
    write(&home.path().join(".local/share/opencode/log/1.log"), "");
    assert!(super::detect::installed(
        home.path(),
        &["openvair-test-none"],
        &traces[..1]
    ));
}

#[test]
fn traces_are_never_files_vair_writes() {
    let home = tempfile::tempdir().unwrap();
    for id in ALL {
        let files = id.files(home.path());
        for trace in id.meta().traces {
            let trace = home.path().join(trace);
            assert!(
                files
                    .iter()
                    .all(|file| !file.starts_with(&trace) && !trace.starts_with(file)),
                "{} trace {}",
                id.name(),
                trace.display()
            );
            assert_ne!(trace, id.config_dir(home.path()), "{}", id.name());
        }
    }
}

#[test]
fn finds_commands_in_every_node_version() {
    let home = tempfile::tempdir().unwrap();
    let name = if cfg!(windows) {
        "openvair-test-agent.cmd"
    } else {
        "openvair-test-agent"
    };
    write(
        &home
            .path()
            .join(".nvm/versions/node/v22.1.0/bin")
            .join(name),
        "",
    );
    let folders = super::detect::folders(home.path(), None);
    assert!(super::detect::found(&folders, &["openvair-test-agent"]));
    assert!(!super::detect::found(&folders, &["openvair-test-none"]));
}
