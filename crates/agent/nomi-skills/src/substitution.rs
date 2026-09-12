use regex::Regex;
use std::sync::LazyLock;

/// Substitute skill variables and argument placeholders in a single pass.
/// Inserted values are literal, not templates. Named arguments take precedence
/// over indexed/full arguments; numeric names are reserved for positional args.
/// With no args, only skill directory and session variables are substituted.
/// Non-empty args are appended when no argument placeholder was consumed.
pub fn substitute_arguments(
    content: &str,
    args: Option<&str>,
    argument_names: &[String],
    skill_root: Option<&str>,
    session_id: Option<&str>,
) -> String {
    static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\w").unwrap());
    static INDEX: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^(?:ARGUMENTS\[(\d+)\]|(\d+))").unwrap());
    let parsed = args.map(parse_arguments).unwrap_or_default();
    let env = [
        ("{NOMI_SKILL_DIR}", skill_root),
        ("{NOMI_SESSION_ID}", session_id),
    ];
    let mut result = String::with_capacity(content.len());
    let mut remaining = content;
    let mut used_args = false;

    while let Some(start) = remaining.find('$') {
        result.push_str(&remaining[..start]);
        let token = &remaining[start + 1..];
        let replacement = env
            .iter()
            .find_map(|(name, value)| token.starts_with(name).then_some((name.len(), (*value)?)))
            .or_else(|| {
                let args = args?;
                let named = argument_names.iter().enumerate().find_map(|(i, name)| {
                    if name.is_empty() || name.chars().all(|c| c.is_ascii_digit()) {
                        return None;
                    }
                    let tail = token.strip_prefix(name)?;
                    if tail.starts_with('[') || WORD.is_match(tail) {
                        return None;
                    }
                    Some((name.len(), parsed.get(i).map(String::as_str).unwrap_or("")))
                });
                let replacement = named
                    .or_else(|| {
                        let caps = INDEX.captures(token)?;
                        let matched = caps.get(0)?;
                        // Bracketed indexes need no trailing boundary; $n does.
                        if caps.get(2).is_some() && WORD.is_match(&token[matched.end()..]) {
                            return None;
                        }
                        let index = caps
                            .get(1)
                            .or_else(|| caps.get(2))?
                            .as_str()
                            .parse::<usize>()
                            .ok();
                        let value = index
                            .and_then(|i| parsed.get(i))
                            .map(String::as_str)
                            .unwrap_or("");
                        Some((matched.end(), value))
                    })
                    .or_else(|| {
                        let tail = token.strip_prefix("ARGUMENTS")?;
                        (!tail.starts_with('[') && !WORD.is_match(tail))
                            .then_some(("ARGUMENTS".len(), args))
                    });
                used_args |= replacement.is_some();
                replacement
            });
        if let Some((length, value)) = replacement {
            result.push_str(value);
            remaining = &token[length..];
        } else {
            result.push('$');
            remaining = token;
        }
    }
    result.push_str(remaining);
    if let Some(args) = args.filter(|args| !args.is_empty() && !used_args) {
        result.push_str("\n\nARGUMENTS: ");
        result.push_str(args);
    }
    result
}

/// Parse an argument string into individual arguments.
///
/// Handles double-quoted and single-quoted strings so that
/// `"hello world" foo` parses as `["hello world", "foo"]`.
/// Falls back to whitespace splitting if no quoted strings are present.
pub fn parse_arguments(args: &str) -> Vec<String> {
    if args.trim().is_empty() {
        return Vec::new();
    }

    let mut result = Vec::new();
    let mut current = String::new();
    let mut in_double = false;
    let mut in_single = false;
    let mut started = false;

    for ch in args.chars() {
        match ch {
            '"' if !in_single => {
                in_double = !in_double;
                started = true;
            }
            '\'' if !in_double => {
                in_single = !in_single;
                started = true;
            }
            ch if ch.is_whitespace() && !in_double && !in_single => {
                if started {
                    result.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            _ => {
                current.push(ch);
                started = true;
            }
        }
    }
    if started {
        result.push(current);
    }

    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- parse_arguments ---

    #[test]
    fn test_parse_empty_and_quoted_arguments() {
        assert!(parse_arguments("").is_empty());
        assert!(parse_arguments("   ").is_empty());
        assert_eq!(parse_arguments(r#""" next ''"#), vec!["", "next", ""]);
        assert_eq!(parse_arguments("first\nsecond\r\nthird"), vec!["first", "second", "third"]);
    }

    #[test]
    fn test_parse_simple_words() {
        assert_eq!(parse_arguments("foo bar baz"), vec!["foo", "bar", "baz"]);
    }

    #[test]
    fn test_parse_double_quoted() {
        assert_eq!(
            parse_arguments(r#""hello world" foo"#),
            vec!["hello world", "foo"]
        );
    }

    #[test]
    fn test_parse_single_quoted() {
        assert_eq!(
            parse_arguments("'hello world' foo"),
            vec!["hello world", "foo"]
        );
    }

    #[test]
    fn test_parse_mixed_quotes() {
        assert_eq!(
            parse_arguments(r#"foo "bar baz" qux"#),
            vec!["foo", "bar baz", "qux"]
        );
    }

    // --- substitute_arguments ---

    #[test]
    fn test_adjacent_placeholders_are_all_replaced() {
        for (template, names) in [
            ("$0$1$0", vec![]),
            ("$foo$bar$foo", vec!["foo".into(), "bar".into()]),
            ("$foo-bar$other.name$foo-bar", vec!["foo-bar".into(), "other.name".into()]),
        ] {
            assert_eq!(substitute_arguments(template, Some("A B"), &names, None, None), "ABA");
        }
    }

    #[test]
    fn test_replacement_values_are_literal() {
        let names = vec!["foo".into(), "bar".into()];
        for (template, args, expected) in [
            ("$foo $bar", "$bar end", "$bar end"),
            ("$foo", "$ARGUMENTS[1] end", "$ARGUMENTS[1]"),
            ("$ARGUMENTS[0]", "$1 end", "$1"),
            ("$0", "$ARGUMENTS end", "$ARGUMENTS"),
            ("$0", "$0", "$0"),
            ("$ARGUMENTS", "$ARGUMENTS", "$ARGUMENTS"),
        ] {
            assert_eq!(substitute_arguments(template, Some(args), &names, None, None), expected);
        }
        assert_eq!(
            substitute_arguments("${NOMI_SKILL_DIR}|${NOMI_SESSION_ID}|$0", Some("X"), &[],
                Some("/skills/$0/${NOMI_SESSION_ID}"), Some("$ARGUMENTS")),
            "/skills/$0/${NOMI_SESSION_ID}|$ARGUMENTS|X"
        );
    }

    #[test]
    fn test_placeholder_boundaries() {
        let names = vec!["foo".into()];
        assert_eq!(
            substitute_arguments("$foo_ $foo[0] $foo中 $0x $ARGUMENTS_extra $ARGUMENTS[bad] $foo",
                Some("X"), &names, None, None),
            "$foo_ $foo[0] $foo中 $0x $ARGUMENTS_extra $ARGUMENTS[bad] X"
        );
    }

    #[test]
    fn test_no_args_returns_unchanged() {
        let content = "hello $ARGUMENTS world";
        let result = substitute_arguments(content, None, &[], None, None);
        assert_eq!(result, content);
    }

    #[test]
    fn test_arguments_full_substitution() {
        let result = substitute_arguments("run $ARGUMENTS now", Some("foo bar"), &[], None, None);
        assert_eq!(result, "run foo bar now");
    }

    #[test]
    fn test_arguments_indexed() {
        let result = substitute_arguments(
            "first=$ARGUMENTS[0] second=$ARGUMENTS[1]",
            Some("alpha beta"),
            &[],
            None,
            None,
        );
        assert_eq!(result, "first=alpha second=beta");
    }

    #[test]
    fn test_arguments_shorthand() {
        let result = substitute_arguments("a=$0 b=$1", Some("x y"), &[], None, None);
        assert_eq!(result, "a=x b=y");
    }

    #[test]
    fn test_named_arguments() {
        let names = vec!["filename".to_string(), "target".to_string()];
        let result = substitute_arguments(
            "file=$filename dest=$target",
            Some("foo.rs /tmp"),
            &names,
            None,
            None,
        );
        assert_eq!(result, "file=foo.rs dest=/tmp");
    }

    #[test]
    fn test_named_arg_no_partial_match() {
        // $foo should not match inside $foobar
        let names = vec!["foo".to_string()];
        let result = substitute_arguments("$foobar and $foo", Some("X"), &names, None, None);
        // $foobar stays (not a word boundary match), $foo becomes X
        assert_eq!(result, "$foobar and X");
    }

    #[test]
    fn test_nomi_skill_dir_substitution() {
        let result =
            substitute_arguments("dir=${NOMI_SKILL_DIR}", None, &[], Some("/my/skill"), None);
        assert_eq!(result, "dir=/my/skill");
    }

    #[test]
    fn test_nomi_session_id_substitution() {
        let result =
            substitute_arguments("sid=${NOMI_SESSION_ID}", None, &[], None, Some("sess-123"));
        assert_eq!(result, "sid=sess-123");
    }

    #[test]
    fn test_fallback_append_when_no_placeholder() {
        let result = substitute_arguments("hello world", Some("my-arg"), &[], None, None);
        assert_eq!(result, "hello world\n\nARGUMENTS: my-arg");
    }

    #[test]
    fn test_no_fallback_when_args_empty() {
        // Empty string — no fallback appended
        let result = substitute_arguments("hello world", Some(""), &[], None, None);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_arguments_out_of_bounds_replaced_with_empty() {
        let result = substitute_arguments("$ARGUMENTS[5]", Some("a"), &[], None, None);
        assert_eq!(result, "");
    }

    #[test]
    fn test_substitution_order_indexed_before_full() {
        // $ARGUMENTS[0] must be replaced before $ARGUMENTS to avoid partial corruption
        let result = substitute_arguments(
            "$ARGUMENTS[0] and $ARGUMENTS",
            Some("hello world"),
            &[],
            None,
            None,
        );
        assert_eq!(result, "hello and hello world");
    }
}

// ---------------------------------------------------------------------------
// Supplemental tests (tester role — covers test-plan.md cases not in impl tests)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod supplemental_tests {
    use super::*;

    #[test]
    fn tc_1_3_multiple_quoted_groups() {
        assert_eq!(
            parse_arguments(r#""arg one" "arg two" plain"#),
            vec!["arg one", "arg two", "plain"]
        );
    }

    #[test]
    fn tc_1_6_single_unquoted_arg() {
        assert_eq!(parse_arguments("single"), vec!["single"]);
    }

    #[test]
    fn tc_1_7_quoted_path_with_spaces() {
        assert_eq!(
            parse_arguments(r#""path/to/file with spaces.txt" --flag"#),
            vec!["path/to/file with spaces.txt", "--flag"]
        );
    }

    #[test]
    fn tc_1_8_unclosed_quote_no_panic() {
        // Must not panic; result is implementation-defined but non-empty
        assert_eq!(parse_arguments(r#""unclosed arg"#), vec!["unclosed arg"]);
    }

    #[test]
    fn tc_2_3_arguments_multiple_occurrences() {
        let r = substitute_arguments("$ARGUMENTS and $ARGUMENTS", Some("x"), &[], None, None);
        assert_eq!(r, "x and x");
    }

    #[test]
    fn tc_3_4_arguments_index_with_quoted_arg() {
        let r = substitute_arguments(
            "$ARGUMENTS[0]",
            Some(r#""hello world" foo"#),
            &[],
            None,
            None,
        );
        assert_eq!(r, "hello world");
    }

    #[test]
    fn tc_4_1_shorthand_0() {
        let r = substitute_arguments("Hello $0", Some("world"), &[], None, None);
        assert_eq!(r, "Hello world");
    }

    #[test]
    fn tc_4_4_shorthand_no_args() {
        let r = substitute_arguments("Run $0", None, &[], None, None);
        // args = None → no substitution, content returned unchanged
        assert_eq!(r, "Run $0");
    }

    #[test]
    fn tc_5_1_single_named_arg() {
        // $query maps to argument index 0; args "rust programming" parses to ["rust", "programming"].
        // $query is replaced with the first parsed argument "rust".
        // "programming" is the second argument but has no placeholder in content.
        let names = vec!["query".to_string()];
        let r = substitute_arguments(
            "Search for $query",
            Some("rust programming"),
            &names,
            None,
            None,
        );
        assert_eq!(r, "Search for rust");
    }

    #[test]
    fn tc_5_4_named_arg_index_out_of_range() {
        // $second maps to index 1 but only one arg provided
        let names = vec!["first".to_string(), "second".to_string()];
        let r = substitute_arguments("File: $second", Some("only_one"), &names, None, None);
        assert_eq!(r, "File: ");
    }

    #[test]
    fn tc_6_2_skill_dir_none_not_replaced() {
        // skill_root = None → ${NOMI_SKILL_DIR} stays unreplaced
        let r = substitute_arguments("cd ${NOMI_SKILL_DIR}", None, &[], None, None);
        assert_eq!(r, "cd ${NOMI_SKILL_DIR}");
    }

    #[test]
    fn tc_6_3_skill_dir_multiple_occurrences() {
        let r = substitute_arguments(
            "${NOMI_SKILL_DIR}/a and ${NOMI_SKILL_DIR}/b",
            None,
            &[],
            Some("/skills/foo"),
            None,
        );
        assert_eq!(r, "/skills/foo/a and /skills/foo/b");
    }

    #[test]
    fn tc_7_2_session_id_none_not_replaced() {
        let r = substitute_arguments("Session: ${NOMI_SESSION_ID}", None, &[], None, None);
        assert_eq!(r, "Session: ${NOMI_SESSION_ID}");
    }

    #[test]
    fn tc_8_2_no_placeholder_no_args_no_append() {
        let r = substitute_arguments("Do the task.", None, &[], None, None);
        assert_eq!(r, "Do the task.");
    }

    #[test]
    fn tc_8_3_with_placeholder_no_append() {
        let r = substitute_arguments("Run $ARGUMENTS", Some("x"), &[], None, None);
        assert_eq!(r, "Run x");
        assert!(!r.contains("ARGUMENTS:"));
    }

    #[test]
    fn tc_9_1_multiple_placeholder_types() {
        let r = substitute_arguments(
            "cd ${NOMI_SKILL_DIR} && run $ARGUMENTS[0] with $ARGUMENTS",
            Some("alpha beta"),
            &[],
            Some("/skills/foo"),
            None,
        );
        assert_eq!(r, "cd /skills/foo && run alpha with alpha beta");
    }

    #[test]
    fn tc_9_2_empty_content_with_args_appends() {
        let r = substitute_arguments("", Some("foo"), &[], None, None);
        assert_eq!(r, "\n\nARGUMENTS: foo");
    }

    #[test]
    fn tc_9_3_empty_content_no_args() {
        let r = substitute_arguments("", None, &[], None, None);
        assert_eq!(r, "");
    }

    #[test]
    fn tc_15_2_skill_dir_and_arguments_same_line() {
        let r = substitute_arguments(
            "${NOMI_SKILL_DIR}: $ARGUMENTS",
            Some("test"),
            &[],
            Some("/root"),
            None,
        );
        assert_eq!(r, "/root: test");
    }

    #[test]
    fn tc_15_3_large_args_no_panic() {
        let big_arg = "x".repeat(10_000);
        let r = substitute_arguments("$ARGUMENTS", Some(&big_arg), &[], None, None);
        assert_eq!(r, big_arg);
    }
}
