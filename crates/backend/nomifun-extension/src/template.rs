use std::path::Path;

use crate::error::ExtensionError;

/// Resolve `${ENV_VAR}` placeholders in a string value.
///
/// - **Lenient mode** (default): undefined variables are replaced with empty string.
/// - **Strict mode** (`strict = true`): undefined variables return an error.
pub fn resolve_env_templates(value: &str, strict: bool) -> Result<String, ExtensionError> {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '$' && chars.peek() == Some(&'{') {
            chars.next(); // consume '{'
            let var_name = collect_until_closing_brace(&mut chars);
            if var_name.is_empty() {
                // Malformed `${}` — pass through literally
                result.push_str("${}");
                continue;
            }
            match std::env::var(&var_name) {
                Ok(val) => result.push_str(&val),
                Err(_) if strict => {
                    return Err(ExtensionError::UndefinedEnvVariable(var_name));
                }
                Err(_) => { /* lenient: replace with empty string */ }
            }
        } else {
            result.push(ch);
        }
    }

    Ok(result)
}

/// Collect characters until `}` or end-of-string.
fn collect_until_closing_brace(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut name = String::new();
    for ch in chars.by_ref() {
        if ch == '}' {
            return name;
        }
        name.push(ch);
    }
    name
}

/// If `value` starts with `@file:`, read the referenced file content relative to `ext_dir`.
/// Otherwise return the value unchanged.
///
/// Path traversal protection: the resolved path must remain within `ext_dir`.
pub fn resolve_file_reference(value: &str, ext_dir: &Path) -> Result<String, ExtensionError> {
    let Some(rel_path) = value.strip_prefix("@file:") else {
        return Ok(value.to_owned());
    };

    if rel_path.is_empty() {
        return Err(ExtensionError::FileReferenceNotFound("@file: with empty path".into()));
    }

    let full_path = ext_dir.join(rel_path);
    if !full_path.exists() {
        return Err(ExtensionError::FileReferenceNotFound(full_path.display().to_string()));
    }

    // Canonicalize both paths to resolve symlinks and `..` components,
    // then verify the target stays within the extension directory.
    let canonical_dir = ext_dir.canonicalize().map_err(ExtensionError::from)?;
    let canonical_file = full_path.canonicalize().map_err(ExtensionError::from)?;

    if !canonical_file.starts_with(&canonical_dir) {
        return Err(ExtensionError::PathTraversal(rel_path.to_owned()));
    }

    std::fs::read_to_string(&canonical_file).map_err(ExtensionError::from)
}

/// Resolve all `${ENV_VAR}` placeholders in a map of key-value env entries.
pub fn resolve_env_map(
    env: &std::collections::HashMap<String, String>,
    strict: bool,
) -> Result<std::collections::HashMap<String, String>, ExtensionError> {
    env.iter()
        .map(|(k, v)| {
            let resolved = resolve_env_templates(v, strict)?;
            Ok((k.clone(), resolved))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // -- resolve_env_templates --

    #[test]
    fn test_no_placeholders() {
        let result = resolve_env_templates("hello world", false).unwrap();
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_single_env_var() {
        unsafe { std::env::set_var("_TEST_RESOLVE_SINGLE", "resolved_value") };
        let result = resolve_env_templates("key=${_TEST_RESOLVE_SINGLE}", false).unwrap();
        assert_eq!(result, "key=resolved_value");
        unsafe { std::env::remove_var("_TEST_RESOLVE_SINGLE") };
    }

    #[test]
    fn test_multiple_env_vars() {
        unsafe { std::env::set_var("_TEST_A", "alpha") };
        unsafe { std::env::set_var("_TEST_B", "beta") };
        let result = resolve_env_templates("${_TEST_A} and ${_TEST_B}", false).unwrap();
        assert_eq!(result, "alpha and beta");
        unsafe { std::env::remove_var("_TEST_A") };
        unsafe { std::env::remove_var("_TEST_B") };
    }

    #[test]
    fn test_undefined_lenient_replaces_empty() {
        let result = resolve_env_templates("val=${_NONEXISTENT_VAR_123}", false).unwrap();
        assert_eq!(result, "val=");
    }

    #[test]
    fn test_undefined_strict_returns_error() {
        let err = resolve_env_templates("${_NONEXISTENT_VAR_456}", true).unwrap_err();
        assert!(matches!(err, ExtensionError::UndefinedEnvVariable(ref v) if v == "_NONEXISTENT_VAR_456"));
    }

    #[test]
    fn test_empty_braces_pass_through() {
        let result = resolve_env_templates("before${}after", false).unwrap();
        assert_eq!(result, "before${}after");
    }

    #[test]
    fn test_dollar_without_brace_pass_through() {
        let result = resolve_env_templates("cost is $50", false).unwrap();
        assert_eq!(result, "cost is $50");
    }

    #[test]
    fn test_nested_dollar_brace() {
        unsafe { std::env::set_var("_TEST_NESTED", "inner") };
        let result = resolve_env_templates("${_TEST_NESTED}", false).unwrap();
        assert_eq!(result, "inner");
        unsafe { std::env::remove_var("_TEST_NESTED") };
    }

    // -- resolve_file_reference --

    #[test]
    fn test_non_file_reference_unchanged() {
        let result = resolve_file_reference("just a string", Path::new("/tmp")).unwrap();
        assert_eq!(result, "just a string");
    }

    #[test]
    fn test_file_reference_reads_content() {
        let dir = std::env::temp_dir().join("ext_test_file_ref");
        std::fs::create_dir_all(&dir).unwrap();
        let file_path = dir.join("prompt.md");
        std::fs::write(&file_path, "You are a helpful assistant.").unwrap();

        let result = resolve_file_reference("@file:prompt.md", &dir).unwrap();
        assert_eq!(result, "You are a helpful assistant.");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn test_file_reference_not_found() {
        let err = resolve_file_reference("@file:nonexistent.md", Path::new("/tmp/no_such_ext")).unwrap_err();
        assert!(matches!(err, ExtensionError::FileReferenceNotFound(_)));
    }

    #[test]
    fn test_file_reference_empty_path() {
        let err = resolve_file_reference("@file:", Path::new("/tmp")).unwrap_err();
        assert!(matches!(err, ExtensionError::FileReferenceNotFound(_)));
    }

    #[test]
    fn test_file_reference_path_traversal_blocked() {
        let dir = std::env::temp_dir().join("ext_test_traversal");
        std::fs::create_dir_all(&dir).unwrap();

        // Create a file outside the extension directory
        let outside_file = std::env::temp_dir().join("ext_test_traversal_secret.txt");
        std::fs::write(&outside_file, "secret data").unwrap();

        let err = resolve_file_reference("@file:../ext_test_traversal_secret.txt", &dir).unwrap_err();
        assert!(matches!(err, ExtensionError::PathTraversal(_)));

        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_file(&outside_file).unwrap();
    }

    #[test]
    fn test_file_reference_nested_traversal_blocked() {
        let dir = std::env::temp_dir().join("ext_test_nested_traversal");
        let sub = dir.join("sub");
        std::fs::create_dir_all(&sub).unwrap();

        let outside_file = std::env::temp_dir().join("ext_test_nested_secret.txt");
        std::fs::write(&outside_file, "nested secret").unwrap();

        let err = resolve_file_reference("@file:sub/../../ext_test_nested_secret.txt", &dir).unwrap_err();
        assert!(matches!(err, ExtensionError::PathTraversal(_)));

        std::fs::remove_dir_all(&dir).unwrap();
        std::fs::remove_file(&outside_file).unwrap();
    }

    #[test]
    fn test_file_reference_valid_subdir_allowed() {
        let dir = std::env::temp_dir().join("ext_test_valid_subdir");
        let sub = dir.join("prompts");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("system.md"), "valid content").unwrap();

        let result = resolve_file_reference("@file:prompts/system.md", &dir).unwrap();
        assert_eq!(result, "valid content");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    // -- resolve_env_map --

    #[test]
    fn test_resolve_env_map_lenient() {
        unsafe { std::env::set_var("_TEST_MAP_KEY", "secret123") };
        let mut env = HashMap::new();
        env.insert("API_KEY".into(), "${_TEST_MAP_KEY}".into());
        env.insert("STATIC".into(), "static_value".into());

        let resolved = resolve_env_map(&env, false).unwrap();
        assert_eq!(resolved["API_KEY"], "secret123");
        assert_eq!(resolved["STATIC"], "static_value");

        unsafe { std::env::remove_var("_TEST_MAP_KEY") };
    }

    #[test]
    fn test_resolve_env_map_strict_error() {
        let mut env = HashMap::new();
        env.insert("KEY".into(), "${_MISSING_MAP_VAR}".into());

        let err = resolve_env_map(&env, true).unwrap_err();
        assert!(matches!(err, ExtensionError::UndefinedEnvVariable(_)));
    }
}
