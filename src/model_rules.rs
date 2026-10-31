//! Authorization/routing rules. Prices never use these matchers.
use crate::error::{AppError, Result};

pub fn concrete(value: &str) -> bool {
    !value.trim().is_empty()
        && value == value.trim()
        && value.len() <= 256
        && !value.contains('*')
        && !value.chars().any(char::is_control)
}

pub fn normalize(value: &str) -> Result<String> {
    let value = value.trim();
    let name = value.strip_suffix('*').unwrap_or(value);
    if !concrete(name) {
        return Err(AppError::BadRequest(
            "请输入具体模型名，或仅在末尾包含一个 * 的非空前缀规则".into(),
        ));
    }
    Ok(value.to_owned())
}

pub fn matches(rule: &str, model: &str) -> bool {
    concrete(model)
        && (rule == model
            || rule
                .strip_suffix('*')
                .is_some_and(|prefix| concrete(prefix) && model.starts_with(prefix)))
}

pub fn allows(rules: &[String], model: &str) -> bool {
    rules.iter().any(|rule| matches(rule, model))
}

pub fn validate_mode(mode: &str, all_mode: &str, rules: &[String]) -> Result<()> {
    if mode == all_mode || (mode == "allowlist" && !rules.is_empty()) {
        return Ok(());
    }
    Err(AppError::BadRequest(
        "按规则模式至少需要一条模型规则；如需全部模型，请显式切换模式".into(),
    ))
}

/// Keys allowing every model store no rules. Versions before 0033 have no mode
/// column, and the database maps their non-empty rule lists back to `allowlist`.
pub fn key_rules(mode: &str, rules: Vec<String>) -> Vec<String> {
    if mode == "all" { Vec::new() } else { rules }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_trailing_prefixes_are_rules_and_matching_is_case_sensitive() {
        for invalid in ["*", "gpt**", "g*pt*", "gpt*foo", "", "a\nb", "gpt- *"] {
            assert!(normalize(invalid).is_err(), "{invalid:?}");
        }
        assert_eq!(normalize(" gpt-* ").unwrap(), "gpt-*");
        assert!(matches("gpt-*", "gpt-new"));
        assert!(!matches("gpt-*", "GPT-new"));
        assert!(!matches("gpt-4.1", "gpt-4.1-mini"));
        assert!(!matches("gpt-*", "gpt-*"));
        assert!(validate_mode("allowlist", "all", &[]).is_err());
        assert!(validate_mode("all", "all", &[]).is_ok());
    }
}
