// Memory system prompt construction.
//
// Builds the compact behavioral instructions and MEMORY.md content that
// get injected into the agent's system prompt so it knows how to read,
// write, and manage the persistent memory system.

use std::path::Path;

use crate::index::{read_index, truncate_index};
use crate::paths::ENTRYPOINT_NAME;

// ---------------------------------------------------------------------------
// Display name
// ---------------------------------------------------------------------------

const DISPLAY_NAME: &str = "auto memory";

// ---------------------------------------------------------------------------
// Directory existence guidance
// ---------------------------------------------------------------------------

/// Guidance appended to the memory directory prompt line so the model
/// doesn't waste turns on `ls` / `mkdir -p` before writing.
const DIR_EXISTS_GUIDANCE: &str = "This directory already exists \u{2014} \
    write to it directly with the Write tool \
    (do not run mkdir or check for its existence).";

// ---------------------------------------------------------------------------
// Minimal memory prompt (saves ~2,500 tokens vs a full taxonomy prompt)
// ---------------------------------------------------------------------------

/// Compact summary of the memory system rules, without a full type taxonomy,
/// examples, or detailed save/access instructions. Enough for the LLM to
/// read existing memories, save new ones, and know the system exists.
const MINIMAL_RULES: &str = "\
You should build up this memory system over time so that future conversations \
can have a complete picture of who the user is, how they'd like to collaborate \
with you, what behaviors to avoid or repeat, and the context behind the work \
the user gives you.

If the user explicitly asks you to remember something, save it immediately. \
If they ask you to forget something, find and remove the relevant entry.

Memory types: user, feedback, project, reference. Each memory is a Markdown file \
with YAML frontmatter (name, description, type). MEMORY.md is the index — one \
line per entry, never write content directly into it.

Before saving, read existing memories to avoid duplicates. \
Verify file/function names from memory still exist before recommending them.";

// ===========================================================================
// Public API
// ===========================================================================

/// Build a minimal memory prompt with just the path, compact rules,
/// and MEMORY.md index content.
pub fn build_memory_prompt_minimal(memory_dir: &Path) -> String {
    let dir_display = memory_dir.display();

    let mut parts = vec![
        format!("# {DISPLAY_NAME}"),
        String::new(),
        format!(
            "You have a persistent, file-based memory system at `{dir_display}`. \
             {DIR_EXISTS_GUIDANCE}"
        ),
        String::new(),
        MINIMAL_RULES.to_owned(),
        String::new(),
    ];

    // Append MEMORY.md index content (or an empty-state message).
    let entrypoint = memory_dir.join(ENTRYPOINT_NAME);
    let raw = read_index(&entrypoint);
    let trimmed = raw.trim();

    if trimmed.is_empty() {
        parts.push(format!("## {ENTRYPOINT_NAME}"));
        parts.push(String::new());
        parts.push(format!(
            "Your {ENTRYPOINT_NAME} is currently empty. \
             When you save new memories, they will appear here."
        ));
    } else {
        let truncation = truncate_index(&raw);
        parts.push(format!("## {ENTRYPOINT_NAME}"));
        parts.push(String::new());
        parts.push(truncation.content);
    }

    parts.join("\n")
}

// ---------------------------------------------------------------------------
// Citation contract (citation reflow)
// ---------------------------------------------------------------------------

/// Instruction appended to the memory prompt so the model emits a structured
/// citation block whenever its answer drew on a stored memory. The backend
/// parses the filenames out of this block at turn end and bumps each cited
/// file's `usage_count` / `last_used` (see `distill::parse_citation_filenames`
/// and `store::bump_memory_usage`).
///
/// Kept short (a few dozen tokens) and only injected when a memory directory
/// exists. The block is appended *after* the visible answer, one entry per
/// line: `<filename>|note=[one-line how-it-was-used]`.
pub const CITATION_CONTRACT: &str = "\
## Citing memory

If your answer drew on the MEMORY.md index or any memory file above, append a \
single citation block at the very end of your reply, listing only the files you \
actually used:

<nomi-mem-citation>
user_role.md|note=[one-line note on how this shaped the answer]
feedback_testing.md|note=[…]
</nomi-mem-citation>

One line per cited file: the memory filename, then `|note=[…]`. If you did not \
use any stored memory, do not emit the block at all.";

// ===========================================================================
// Unit tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- CITATION_CONTRACT ---------------------------------------------------

    #[test]
    fn citation_contract_has_block_tags_and_note_format() {
        assert!(CITATION_CONTRACT.contains("<nomi-mem-citation>"));
        assert!(CITATION_CONTRACT.contains("</nomi-mem-citation>"));
        assert!(CITATION_CONTRACT.contains("|note=["));
    }

    // -- build_memory_prompt_minimal -------------------------------------------

    #[test]
    fn minimal_prompt_contains_display_name() {
        let result = build_memory_prompt_minimal(Path::new("/test/memory"));
        assert!(result.contains(DISPLAY_NAME));
    }

    #[test]
    fn minimal_prompt_contains_dir_path() {
        let result = build_memory_prompt_minimal(Path::new("/custom/path/memory"));
        assert!(result.contains("/custom/path/memory"));
    }

    #[test]
    fn minimal_prompt_contains_dir_exists_guidance() {
        let result = build_memory_prompt_minimal(Path::new("/test/memory"));
        assert!(result.contains("already exists"));
    }

    #[test]
    fn minimal_prompt_contains_compact_rules() {
        let result = build_memory_prompt_minimal(Path::new("/test/memory"));
        assert!(
            result.contains("Memory types:"),
            "should list memory types compactly"
        );
        assert!(
            result.contains("MEMORY.md is the index"),
            "should mention MEMORY.md role"
        );
    }

    #[test]
    fn minimal_prompt_omits_full_type_taxonomy() {
        let result = build_memory_prompt_minimal(Path::new("/test/memory"));
        assert!(
            !result.contains("## Types of memory"),
            "minimal prompt should NOT contain full type taxonomy heading"
        );
        assert!(
            !result.contains("<types>"),
            "minimal prompt should NOT contain XML type definitions"
        );
        assert!(
            !result.contains("## What NOT to save"),
            "minimal prompt should NOT contain what-not-to-save section"
        );
        assert!(
            !result.contains("## How to save memories"),
            "minimal prompt should NOT contain detailed save instructions"
        );
    }

    #[test]
    fn minimal_prompt_nonexistent_dir_shows_empty_state() {
        let result = build_memory_prompt_minimal(Path::new("/nonexistent/memory/dir"));
        assert!(result.contains("currently empty"));
    }

    #[test]
    fn minimal_prompt_with_existing_index() {
        let tmp = tempfile::tempdir().unwrap();
        let mem_dir = tmp.path().join("memory");
        std::fs::create_dir_all(&mem_dir).unwrap();
        std::fs::write(
            mem_dir.join(ENTRYPOINT_NAME),
            "- [Role](user_role.md) \u{2014} senior engineer\n",
        )
        .unwrap();

        let result = build_memory_prompt_minimal(&mem_dir);
        assert!(result.contains("user_role.md"));
        assert!(result.contains("senior engineer"));
        assert!(!result.contains("currently empty"));
    }

    #[test]
    fn minimal_prompt_truncates_large_index() {
        let tmp = tempfile::tempdir().unwrap();
        let mem_dir = tmp.path().join("memory");
        std::fs::create_dir_all(&mem_dir).unwrap();

        // Create an index with 250 lines
        let content: String = (0..250)
            .map(|i| format!("- [Item {i}](item_{i}.md) \u{2014} summary {i}\n"))
            .collect();
        std::fs::write(mem_dir.join(ENTRYPOINT_NAME), &content).unwrap();

        let result = build_memory_prompt_minimal(&mem_dir);
        assert!(result.contains("WARNING"));
    }

    #[test]
    fn minimal_prompt_no_bb_brand() {
        let result = build_memory_prompt_minimal(Path::new("/test/memory"));
        assert!(
            !result.contains("~/.claude"),
            "should not reference bb config path"
        );
        assert!(
            !result.contains("CLAUDE.md"),
            "should not reference bb config file"
        );
    }
}
