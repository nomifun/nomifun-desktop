// Memory file read and write.
//
// This module handles the file-level operations for memory persistence:
// parsing YAML frontmatter and writing memory entries.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use crate::error::Result;
use crate::types::{MemoryEntry, MemoryFrontmatter};

/// Maximum number of lines to read when extracting frontmatter.
const FRONTMATTER_MAX_LINES: usize = 30;

/// YAML frontmatter delimiter.
const FRONTMATTER_DELIM: &str = "---";

// ---------------------------------------------------------------------------
// Read
// ---------------------------------------------------------------------------

/// Read a single memory file, parsing its YAML frontmatter and body.
///
/// Gracefully degrades: if the file has no valid frontmatter, returns
/// a default (empty) frontmatter with the entire file as body content.
pub fn read_memory(path: &Path) -> Result<MemoryEntry> {
    let raw = fs::read_to_string(path)?;
    let (frontmatter, content) = parse_frontmatter(&raw, Some(path));
    Ok(MemoryEntry::new(frontmatter, content))
}

// ---------------------------------------------------------------------------
// Write
// ---------------------------------------------------------------------------

/// Write a memory entry to a file in `dir`.
///
/// The filename is derived from the entry's type and name:
/// `<type>_<sanitized_name>.md`. Returns the full path of the written file.
///
/// Creates the directory if it doesn't exist.
pub fn write_memory(dir: &Path, entry: &MemoryEntry) -> Result<PathBuf> {
    fs::create_dir_all(dir)?;

    let filename = generate_filename(&entry.frontmatter);
    let path = dir.join(&filename);

    let content = serialize_entry(entry);
    fs::write(&path, content)?;

    Ok(path)
}

// ---------------------------------------------------------------------------
// Citation reflow: bump usage stats
// ---------------------------------------------------------------------------

/// Citation reflow: increment `usage_count` and set `last_used = now` on the
/// frontmatter of the memory file named `filename` inside `dir`, preserving
/// the body verbatim.
///
/// A missing or unreadable file is a no-op (returns `Ok`): citations may name
/// files that were renamed or removed, and that must not surface as an error.
pub fn bump_memory_usage(dir: &Path, filename: &str, now: DateTime<Utc>) -> Result<()> {
    let path = dir.join(filename);
    // Missing / unreadable file = no-op. Citations can name stale filenames.
    let Ok(mut entry) = read_memory(&path) else {
        return Ok(());
    };
    entry.frontmatter.usage_count = Some(entry.frontmatter.usage_count.unwrap_or(0) + 1);
    entry.frontmatter.last_used = Some(now);
    let content = serialize_entry(&entry);
    fs::write(&path, content)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Frontmatter parsing (internal)
// ---------------------------------------------------------------------------

/// Parse YAML frontmatter from raw file content.
///
/// Expects the format:
/// ```text
/// ---
/// name: value
/// type: user
/// ---
/// Body content here
/// ```
///
/// Returns `(frontmatter, body)`. On parse failure, returns default
/// frontmatter and the entire content as body.
fn parse_frontmatter(raw: &str, path: Option<&Path>) -> (MemoryFrontmatter, String) {
    let trimmed = raw.trim_start();

    // Must start with `---`
    if !trimmed.starts_with(FRONTMATTER_DELIM) {
        return (MemoryFrontmatter::default(), raw.to_owned());
    }

    // Find the closing `---`
    let after_open = &trimmed[FRONTMATTER_DELIM.len()..];

    // Skip the rest of the opening delimiter line (e.g. `---\n`)
    let after_newline = match after_open.find('\n') {
        Some(pos) => &after_open[pos + 1..],
        None => return (MemoryFrontmatter::default(), raw.to_owned()),
    };

    // Find the closing delimiter within the frontmatter max lines
    let mut search_offset = 0;
    let mut lines_seen = 0;
    let close_pos = loop {
        if lines_seen >= FRONTMATTER_MAX_LINES {
            // No closing delimiter within limit — treat as no frontmatter
            return (MemoryFrontmatter::default(), raw.to_owned());
        }
        match after_newline[search_offset..].find('\n') {
            Some(nl) => {
                let line = after_newline[search_offset..search_offset + nl].trim();
                if line == FRONTMATTER_DELIM {
                    break search_offset;
                }
                search_offset += nl + 1;
                lines_seen += 1;
            }
            None => {
                // Last line without trailing newline
                let line = after_newline[search_offset..].trim();
                if line == FRONTMATTER_DELIM {
                    break search_offset;
                }
                // No closing delimiter found
                return (MemoryFrontmatter::default(), raw.to_owned());
            }
        }
    };

    let yaml_str = &after_newline[..close_pos];
    let body_start = search_offset + FRONTMATTER_DELIM.len();
    let body = after_newline
        .get(body_start..)
        .unwrap_or("")
        .trim_start_matches('\n');

    // Parse YAML
    let frontmatter = match serde_yaml::from_str::<MemoryFrontmatter>(yaml_str) {
        Ok(fm) => fm,
        Err(e) => {
            if let Some(p) = path {
                tracing::warn!(target: "nomi_memory", path = %p.display(), error = %e, "failed to parse memory frontmatter");
            }
            MemoryFrontmatter::default()
        }
    };

    (frontmatter, body.to_owned())
}

// ---------------------------------------------------------------------------
// Entry serialization (internal)
// ---------------------------------------------------------------------------

/// Serialize a memory entry into the frontmatter + body format.
fn serialize_entry(entry: &MemoryEntry) -> String {
    let yaml = serde_yaml::to_string(&entry.frontmatter).unwrap_or_default();
    // serde_yaml adds a trailing newline; trim it for consistent formatting
    let yaml = yaml.trim_end();

    format!(
        "{FRONTMATTER_DELIM}\n{yaml}\n{FRONTMATTER_DELIM}\n\n{}",
        entry.content
    )
}

// ---------------------------------------------------------------------------
// Filename generation (internal)
// ---------------------------------------------------------------------------

/// Generate a safe filename from an entry's frontmatter.
///
/// Format: `<type>_<sanitized_name>.md`
/// Falls back to `memory_<hash>.md` if name is empty.
fn generate_filename(fm: &MemoryFrontmatter) -> String {
    let type_prefix = fm
        .memory_type
        .map(|t| t.as_str().to_owned())
        .unwrap_or_else(|| "memory".to_owned());

    let name_part = fm
        .name
        .as_deref()
        .filter(|n| !n.trim().is_empty())
        .map(sanitize_filename)
        .filter(|s| !s.is_empty()) // pure non-ASCII names sanitize to empty
        .unwrap_or_else(|| {
            // Use a simple hash of the current time as fallback
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            format!("{now:x}")
        });

    format!("{type_prefix}_{name_part}.md")
}

/// Sanitize a string for use as part of a filename.
///
/// Converts to lowercase, replaces non-alphanumeric chars with underscores,
/// collapses consecutive underscores, and trims leading/trailing underscores.
fn sanitize_filename(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();

    // Collapse consecutive underscores
    let mut result = String::with_capacity(sanitized.len());
    let mut prev_underscore = false;
    for c in sanitized.chars() {
        if c == '_' {
            if !prev_underscore {
                result.push(c);
            }
            prev_underscore = true;
        } else {
            result.push(c);
            prev_underscore = false;
        }
    }

    // Trim leading/trailing underscores
    result.trim_matches('_').to_owned()
}

// ===========================================================================
// Unit tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::MemoryType;
    use chrono::TimeZone;

    // -- parse_frontmatter ---------------------------------------------------

    #[test]
    fn parse_full_frontmatter() {
        let raw = "---\nname: test\ndescription: a test\ntype: feedback\n---\nBody content";
        let (fm, body) = parse_frontmatter(raw, None);
        assert_eq!(fm.name.as_deref(), Some("test"));
        assert_eq!(fm.description.as_deref(), Some("a test"));
        assert_eq!(fm.memory_type, Some(MemoryType::Feedback));
        assert_eq!(body, "Body content");
    }

    #[test]
    fn parse_no_frontmatter() {
        let raw = "Just plain text\nNo frontmatter here";
        let (fm, body) = parse_frontmatter(raw, None);
        assert_eq!(fm, MemoryFrontmatter::default());
        assert_eq!(body, raw);
    }

    #[test]
    fn parse_empty_content() {
        let (fm, body) = parse_frontmatter("", None);
        assert_eq!(fm, MemoryFrontmatter::default());
        assert_eq!(body, "");
    }

    #[test]
    fn parse_only_opening_delimiter() {
        let raw = "---\nname: orphan\nno closing delimiter";
        let (fm, body) = parse_frontmatter(raw, None);
        assert_eq!(fm, MemoryFrontmatter::default());
        assert_eq!(body, raw);
    }

    #[test]
    fn parse_partial_frontmatter_fields() {
        let raw = "---\nname: partial\n---\nBody";
        let (fm, body) = parse_frontmatter(raw, None);
        assert_eq!(fm.name.as_deref(), Some("partial"));
        assert_eq!(fm.description, None);
        assert_eq!(fm.memory_type, None);
        assert_eq!(body, "Body");
    }

    #[test]
    fn parse_frontmatter_with_leading_whitespace() {
        let raw = "  \n---\nname: spaced\n---\nContent";
        let (fm, body) = parse_frontmatter(raw, None);
        assert_eq!(fm.name.as_deref(), Some("spaced"));
        assert_eq!(body, "Content");
    }

    #[test]
    fn parse_invalid_yaml_degrades_gracefully() {
        // YAML with invalid structure — should return default frontmatter
        let raw = "---\n: :\n  :\n---\nBody after bad yaml";
        let (fm, body) = parse_frontmatter(raw, None);
        assert_eq!(fm, MemoryFrontmatter::default());
        assert_eq!(body, "Body after bad yaml");
    }

    #[test]
    fn parse_frontmatter_body_newline_handling() {
        let raw = "---\nname: test\n---\n\nParagraph one\n\nParagraph two";
        let (fm, body) = parse_frontmatter(raw, None);
        assert_eq!(fm.name.as_deref(), Some("test"));
        // Body should start at first content line after delimiter
        assert_eq!(body, "Paragraph one\n\nParagraph two");
    }

    // -- serialize_entry -----------------------------------------------------

    #[test]
    fn serialize_and_parse_roundtrip() {
        let entry = MemoryEntry::build("role", "user role info", MemoryType::User, "I am a dev");
        let serialized = serialize_entry(&entry);
        let (fm, body) = parse_frontmatter(&serialized, None);
        assert_eq!(fm.name.as_deref(), Some("role"));
        assert_eq!(fm.description.as_deref(), Some("user role info"));
        assert_eq!(fm.memory_type, Some(MemoryType::User));
        assert_eq!(body, "I am a dev");
    }

    // -- generate_filename ---------------------------------------------------

    #[test]
    fn filename_with_type_and_name() {
        let fm = MemoryFrontmatter {
            name: Some("My Role".into()),
            description: None,
            memory_type: Some(MemoryType::User),
            ..Default::default()
        };
        let name = generate_filename(&fm);
        assert_eq!(name, "user_my_role.md");
    }

    #[test]
    fn filename_without_type() {
        let fm = MemoryFrontmatter {
            name: Some("notes".into()),
            description: None,
            memory_type: None,
            ..Default::default()
        };
        let name = generate_filename(&fm);
        assert_eq!(name, "memory_notes.md");
    }

    #[test]
    fn filename_without_name() {
        let fm = MemoryFrontmatter {
            name: None,
            description: None,
            memory_type: Some(MemoryType::Feedback),
            ..Default::default()
        };
        let name = generate_filename(&fm);
        assert!(name.starts_with("feedback_"));
        assert!(name.ends_with(".md"));
    }

    #[test]
    fn filename_special_chars_sanitized() {
        let fm = MemoryFrontmatter {
            name: Some("Hello World! / Test: 123".into()),
            description: None,
            memory_type: Some(MemoryType::Project),
            ..Default::default()
        };
        let name = generate_filename(&fm);
        assert_eq!(name, "project_hello_world_test_123.md");
        assert!(!name.contains(' '));
        assert!(!name.contains('/'));
        assert!(!name.contains('!'));
    }

    // -- sanitize_filename ---------------------------------------------------

    #[test]
    fn sanitize_basic() {
        assert_eq!(sanitize_filename("Hello World"), "hello_world");
    }

    #[test]
    fn sanitize_collapses_underscores() {
        assert_eq!(sanitize_filename("a---b___c"), "a_b_c");
    }

    #[test]
    fn sanitize_trims_underscores() {
        assert_eq!(sanitize_filename("__test__"), "test");
    }

    #[test]
    fn sanitize_preserves_alphanumeric() {
        assert_eq!(sanitize_filename("abc123"), "abc123");
    }

    #[test]
    fn sanitize_pure_non_ascii_returns_empty() {
        assert_eq!(sanitize_filename("我的角色"), "");
        assert_eq!(sanitize_filename("全角文本"), "");
    }

    #[test]
    fn filename_pure_non_ascii_name_falls_back_to_hash() {
        let fm1 = MemoryFrontmatter {
            name: Some("我的角色".into()),
            description: None,
            memory_type: Some(MemoryType::User),
            ..Default::default()
        };
        let fm2 = MemoryFrontmatter {
            name: Some("项目状态".into()),
            description: None,
            memory_type: Some(MemoryType::User),
            ..Default::default()
        };
        let name1 = generate_filename(&fm1);
        let name2 = generate_filename(&fm2);
        // Both should get unique hash-based names, not collide
        assert!(name1.starts_with("user_"));
        assert!(name1.ends_with(".md"));
        assert_ne!(name1, "user_.md", "should not produce empty name part");
        // With time-based hash, names should differ (race possible but
        // extremely unlikely given nanos resolution)
        assert_ne!(name1, name2, "pure non-ASCII names should not collide");
    }

    // -- file operations (using tempdir) -------------------------------------

    #[test]
    fn write_then_read_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let entry = MemoryEntry::build("role", "my role", MemoryType::User, "I am a developer");

        let path = write_memory(tmp.path(), &entry).unwrap();
        assert!(path.exists());
        assert_eq!(path.file_name().unwrap().to_str().unwrap(), "user_role.md");

        let read_back = read_memory(&path).unwrap();
        assert_eq!(read_back.frontmatter.name, entry.frontmatter.name);
        assert_eq!(
            read_back.frontmatter.description,
            entry.frontmatter.description
        );
        assert_eq!(
            read_back.frontmatter.memory_type,
            entry.frontmatter.memory_type
        );
        assert_eq!(read_back.content, entry.content);
    }

    // -- bump_memory_usage (citation reflow) ---------------------------------

    #[test]
    fn bump_usage_first_time_sets_count_and_last_used() {
        let tmp = tempfile::tempdir().unwrap();
        let entry = MemoryEntry::build("role", "my role", MemoryType::User, "I am a developer");
        let path = write_memory(tmp.path(), &entry).unwrap();
        let filename = path.file_name().unwrap().to_str().unwrap();

        let now = Utc.with_ymd_and_hms(2026, 6, 14, 9, 30, 0).unwrap();
        bump_memory_usage(tmp.path(), filename, now).unwrap();

        let read_back = read_memory(&path).unwrap();
        assert_eq!(read_back.frontmatter.usage_count, Some(1));
        assert_eq!(read_back.frontmatter.last_used, Some(now));
        // Body preserved verbatim.
        assert_eq!(read_back.content, "I am a developer");
        // Original metadata preserved.
        assert_eq!(read_back.frontmatter.name.as_deref(), Some("role"));
        assert_eq!(read_back.frontmatter.memory_type, Some(MemoryType::User));
    }

    #[test]
    fn bump_usage_accumulates() {
        let tmp = tempfile::tempdir().unwrap();
        let entry = MemoryEntry::build("topic", "desc", MemoryType::Project, "body text");
        let path = write_memory(tmp.path(), &entry).unwrap();
        let filename = path.file_name().unwrap().to_str().unwrap();

        let t1 = Utc.with_ymd_and_hms(2026, 6, 14, 9, 0, 0).unwrap();
        let t2 = Utc.with_ymd_and_hms(2026, 6, 14, 10, 0, 0).unwrap();
        let t3 = Utc.with_ymd_and_hms(2026, 6, 14, 11, 0, 0).unwrap();
        bump_memory_usage(tmp.path(), filename, t1).unwrap();
        bump_memory_usage(tmp.path(), filename, t2).unwrap();
        bump_memory_usage(tmp.path(), filename, t3).unwrap();

        let read_back = read_memory(&path).unwrap();
        assert_eq!(read_back.frontmatter.usage_count, Some(3));
        assert_eq!(read_back.frontmatter.last_used, Some(t3));
    }

    #[test]
    fn bump_usage_missing_file_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        // No file written; must not error.
        let now = Utc.with_ymd_and_hms(2026, 6, 14, 9, 30, 0).unwrap();
        bump_memory_usage(tmp.path(), "user_absent.md", now).unwrap();
        assert!(!tmp.path().join("user_absent.md").exists());
    }

    #[test]
    fn bump_usage_preserves_multiline_body() {
        let tmp = tempfile::tempdir().unwrap();
        let body = "Line one\n\nLine two\n- bullet";
        let entry = MemoryEntry::build("notes", "desc", MemoryType::Reference, body);
        let path = write_memory(tmp.path(), &entry).unwrap();
        let filename = path.file_name().unwrap().to_str().unwrap();

        let now = Utc.with_ymd_and_hms(2026, 6, 14, 9, 30, 0).unwrap();
        bump_memory_usage(tmp.path(), filename, now).unwrap();

        let read_back = read_memory(&path).unwrap();
        assert_eq!(read_back.content, body);
    }
}
