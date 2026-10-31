//! Required settings that may come from a file instead of the environment.

/// Reads `name` from the environment or, when it is unset or empty, from the
/// file named by `{name}_FILE` (the Docker Compose setup keeps generated
/// secrets in files). Surrounding whitespace in the file is ignored.
pub fn required(name: &str) -> Result<String, String> {
    if let Some(value) = std::env::var(name).ok().filter(|value| !value.is_empty()) {
        return Ok(value);
    }
    let file = format!("{name}_FILE");
    let Some(path) = std::env::var(&file).ok().filter(|path| !path.is_empty()) else {
        return Err(format!("{name} (or {file}) is required"));
    };
    let value =
        std::fs::read_to_string(&path).map_err(|e| format!("{file}: cannot read {path}: {e}"))?;
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{file}: {path} is empty"));
    }
    Ok(value.to_owned())
}
