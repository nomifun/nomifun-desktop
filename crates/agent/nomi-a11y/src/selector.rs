//! A focused, platform-independent selector grammar for addressing
//! accessibility elements deterministically — the durable way to re-locate an
//! element across snapshots (vs a `[ref]`, which is snapshot-scoped).
//!
//! Grammar (v1 subset; positional/relative combinators are a planned
//! extension): `prefix:value` terms joined by `&&` / `||`, each optionally
//! negated with a leading `!`.
//!
//! ```text
//! role:Button && name:Save
//! name:Save || name:Submit
//! role:Button && !name:Cancel
//! role:Button && name:Item && nth:2
//! ```
//!
//! Prefixes: `role:` `name:` `text:` `nth:`. A bare term with no prefix is
//! treated as `name:`. `name`/`role` match case-insensitively as substrings;
//! `text` matches case-sensitively (visible-text semantics).

use crate::engine::ElementEntry;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selector {
    /// Accessibility role (case-insensitive substring).
    Role(String),
    /// Accessible name/label (case-insensitive substring).
    Name(String),
    /// Visible text / value (case-sensitive substring).
    Text(String),
    /// Pick the Nth (0-based) of the otherwise-matching set.
    Nth(usize),
    /// All must match.
    And(Vec<Selector>),
    /// Any may match.
    Or(Vec<Selector>),
    /// Must not match.
    Not(Box<Selector>),
}

impl Selector {
    /// Parse a selector expression. Returns a human-readable error on malformed
    /// input (which the caller surfaces to the model, not a panic).
    pub fn parse(input: &str) -> Result<Selector, String> {
        let s = input.trim();
        if s.is_empty() {
            return Err("empty selector".to_string());
        }
        parse_or(s)
    }

    /// True if this selector (ignoring any positional `Nth`) matches `e`.
    pub fn matches(&self, e: &ElementEntry) -> bool {
        match self {
            Selector::Role(r) => contains_ci(&e.role, r),
            Selector::Name(n) => e.name.as_deref().is_some_and(|v| contains_ci(v, n)),
            Selector::Text(t) => {
                e.name.as_deref().is_some_and(|v| v.contains(t.as_str()))
                    || e.value.as_deref().is_some_and(|v| v.contains(t.as_str()))
            }
            Selector::Nth(_) => true, // positional; applied in `select`
            Selector::And(parts) => parts.iter().all(|p| p.matches(e)),
            Selector::Or(parts) => parts.iter().any(|p| p.matches(e)),
            Selector::Not(inner) => !inner.matches(e),
        }
    }

    /// Resolve against a snapshot's entries: filter by the match predicate, then
    /// apply any top-level `Nth` positional pick. Returns matching refs in order.
    pub fn select<'a>(&self, entries: &'a [ElementEntry]) -> Vec<&'a ElementEntry> {
        let matched: Vec<&ElementEntry> = entries.iter().filter(|e| self.matches(e)).collect();
        match self.find_nth() {
            Some(n) => matched.into_iter().skip(n).take(1).collect(),
            None => matched,
        }
    }

    /// Find a top-level `Nth` index if present (directly or inside a top `And`).
    fn find_nth(&self) -> Option<usize> {
        match self {
            Selector::Nth(n) => Some(*n),
            Selector::And(parts) => parts.iter().find_map(|p| match p {
                Selector::Nth(n) => Some(*n),
                _ => None,
            }),
            _ => None,
        }
    }
}

fn contains_ci(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

fn parse_or(s: &str) -> Result<Selector, String> {
    let parts = split_top(s, "||");
    if parts.len() == 1 {
        return parse_and(parts[0]);
    }
    let parsed: Result<Vec<_>, _> = parts.iter().map(|p| parse_and(p)).collect();
    Ok(Selector::Or(parsed?))
}

fn parse_and(s: &str) -> Result<Selector, String> {
    let parts = split_top(s, "&&");
    if parts.len() == 1 {
        return parse_term(parts[0]);
    }
    let parsed: Result<Vec<_>, _> = parts.iter().map(|p| parse_term(p)).collect();
    Ok(Selector::And(parsed?))
}

fn parse_term(s: &str) -> Result<Selector, String> {
    let t = s.trim();
    if let Some(rest) = t.strip_prefix('!') {
        return Ok(Selector::Not(Box::new(parse_simple(rest.trim())?)));
    }
    parse_simple(t)
}

fn parse_simple(s: &str) -> Result<Selector, String> {
    let t = s.trim();
    if t.is_empty() {
        return Err("empty selector term".to_string());
    }
    let (prefix, value) = match t.split_once(':') {
        Some((p, v)) => (p.trim().to_lowercase(), v.trim().to_string()),
        None => ("name".to_string(), t.to_string()),
    };
    if value.is_empty() && prefix != "nth" {
        return Err(format!("selector term `{t}` has an empty value"));
    }
    match prefix.as_str() {
        "role" => Ok(Selector::Role(value)),
        "name" => Ok(Selector::Name(value)),
        "text" => Ok(Selector::Text(value)),
        "nth" => value
            .parse::<usize>()
            .map(Selector::Nth)
            .map_err(|_| format!("`nth:` expects a non-negative integer, got `{value}`")),
        other => Err(format!(
            "unknown selector prefix `{other}:` (supported: role, name, text, nth)"
        )),
    }
}

/// Split on a two-char operator at the top level. (v1 has no parentheses, so
/// this is a plain delimiter split; selector values do not contain `&&`/`||`.)
fn split_top<'a>(s: &'a str, op: &str) -> Vec<&'a str> {
    s.split(op).map(|p| p.trim()).filter(|p| !p.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Rect, Source};

    fn entry(r: u32, role: &str, name: Option<&str>) -> ElementEntry {
        ElementEntry {
            r#ref: r,
            role: role.to_string(),
            name: name.map(|s| s.to_string()),
            value: None,
            states: vec![],
            bounds: Rect { x: 0.0, y: 0.0, w: 1.0, h: 1.0 },
            source: Source::A11y,
        }
    }

    #[test]
    fn parses_role_and_name() {
        let s = Selector::parse("role:Button && name:Save").unwrap();
        assert_eq!(
            s,
            Selector::And(vec![
                Selector::Role("Button".into()),
                Selector::Name("Save".into())
            ])
        );
    }

    #[test]
    fn bare_term_is_name() {
        assert_eq!(Selector::parse("Submit").unwrap(), Selector::Name("Submit".into()));
    }

    #[test]
    fn parses_or_and_not() {
        let s = Selector::parse("name:Save || name:Submit").unwrap();
        assert!(matches!(s, Selector::Or(_)));
        let n = Selector::parse("!name:Cancel").unwrap();
        assert!(matches!(n, Selector::Not(_)));
    }

    #[test]
    fn matches_case_insensitive_substring() {
        let e = entry(1, "AXButton", Some("Save Document"));
        assert!(Selector::parse("role:button && name:save").unwrap().matches(&e));
        assert!(!Selector::parse("name:delete").unwrap().matches(&e));
        assert!(Selector::parse("role:Button && !name:Cancel").unwrap().matches(&e));
    }

    #[test]
    fn nth_picks_positionally() {
        let entries = vec![
            entry(1, "AXButton", Some("Item")),
            entry(2, "AXButton", Some("Item")),
            entry(3, "AXButton", Some("Item")),
        ];
        let s = Selector::parse("role:Button && name:Item && nth:1").unwrap();
        let got = s.select(&entries);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].r#ref, 2);
    }

    #[test]
    fn empty_is_error() {
        assert!(Selector::parse("   ").is_err());
        assert!(Selector::parse("bogus:x").is_err());
    }
}
