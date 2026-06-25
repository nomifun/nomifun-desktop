// Acceptance tests for the memory system end-to-end.
//
// These tests verify that the memory system's file I/O, index management,
// and prompt building work together correctly. No LLM API calls are needed.

use nomi_agent::context::{SystemPromptCache, build_system_prompt};
use nomi_memory::index::{append_index_entry, remove_index_entry};
use nomi_memory::paths::ENTRYPOINT_NAME;
use nomi_memory::store::{delete_memory, write_memory};
use nomi_memory::types::{MemoryEntry, MemoryType};

/// TC-A1-01: Memory injection into system prompt.
///
/// Verifies that when a memory directory exists with an index file and a
/// memory entry, `build_system_prompt()` produces output containing both
/// the compact behavioral instructions and the MEMORY.md index content.
#[test]
fn memory_injection_into_system_prompt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mem_dir = tmp.path().join("memory");
    std::fs::create_dir_all(&mem_dir).unwrap();

    // Write MEMORY.md index with one entry
    let index_path = mem_dir.join(ENTRYPOINT_NAME);
    std::fs::write(
        &index_path,
        "# Memory Index\n\n- [User role](user_role.md) \u{2014} senior Rust engineer\n",
    )
    .unwrap();

    // Write a corresponding memory entry file
    std::fs::write(
        mem_dir.join("user_role.md"),
        "---\nname: user_role\ndescription: senior Rust engineer\ntype: user\n---\n\nThe user is a senior Rust engineer.\n",
    )
    .unwrap();

    let prompt = build_system_prompt(
        &mut SystemPromptCache::new(),
        None,
        "/tmp",
        "test-model",
        &[],
        None,
        Some(&mem_dir),
        false,
        false,
        false, // browser_enabled
    );

    // Behavioral instructions must be present
    assert!(
        prompt.contains("auto memory"),
        "system prompt should contain the memory display name"
    );
    assert!(
        prompt.contains("Memory types:"),
        "system prompt should contain the compact memory type summary"
    );

    // MEMORY.md content must be injected
    assert!(
        prompt.contains("user_role.md"),
        "system prompt should contain the MEMORY.md index filename reference"
    );
    assert!(
        prompt.contains("senior Rust engineer"),
        "system prompt should contain the MEMORY.md index summary"
    );
}

/// TC-A1-02: Memory full lifecycle (create, index, verify, delete, verify gone).
///
/// Exercises the complete lifecycle of a memory entry through the public API:
///   1. write_memory()     -> create the file
///   2. append_index_entry() -> add to MEMORY.md
///   3. build_system_prompt() -> verify the content appears
///   4. delete_memory()    -> remove the file
///   5. remove_index_entry() -> clean the index
///   6. build_system_prompt() -> verify the content is gone
#[test]
fn memory_full_lifecycle() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mem_dir = tmp.path().join("memory");
    std::fs::create_dir_all(&mem_dir).unwrap();

    let index_path = mem_dir.join(ENTRYPOINT_NAME);

    // -- Phase 1: Create memory entry via the store API -----------------------

    let entry = MemoryEntry::build(
        "project status",
        "current sprint goals",
        MemoryType::Project,
        "We are migrating the auth service to the new provider.",
    );

    let entry_path = write_memory(&mem_dir, &entry).unwrap();
    assert!(entry_path.exists(), "memory file should be created on disk");

    let entry_filename = entry_path.file_name().unwrap().to_str().unwrap().to_owned();

    // -- Phase 2: Add the entry to the MEMORY.md index ------------------------

    append_index_entry(
        &index_path,
        "Project status",
        &entry_filename,
        "current sprint goals",
    )
    .unwrap();

    let index_content = std::fs::read_to_string(&index_path).unwrap();
    assert!(
        index_content.contains(&entry_filename),
        "MEMORY.md should reference the new entry file"
    );

    // -- Phase 3: Verify system prompt includes the memory content ------------

    let prompt_with_memory = build_system_prompt(
        &mut SystemPromptCache::new(),
        None,
        "/tmp",
        "test-model",
        &[],
        None,
        Some(&mem_dir),
        false,
        false,
        false, // browser_enabled
    );

    assert!(
        prompt_with_memory.contains("auto memory"),
        "prompt should contain behavioral instructions"
    );
    assert!(
        prompt_with_memory.contains("Memory types:"),
        "prompt should contain compact memory type summary"
    );
    assert!(
        prompt_with_memory.contains(&entry_filename),
        "prompt should contain the memory entry filename from the index"
    );
    assert!(
        prompt_with_memory.contains("current sprint goals"),
        "prompt should contain the index summary"
    );

    // -- Phase 4: Delete the memory file --------------------------------------

    delete_memory(&entry_path).unwrap();
    assert!(
        !entry_path.exists(),
        "memory file should be removed from disk"
    );

    // -- Phase 5: Remove the entry from the MEMORY.md index -------------------

    remove_index_entry(&index_path, &entry_filename).unwrap();

    let index_after = std::fs::read_to_string(&index_path).unwrap();
    assert!(
        !index_after.contains(&entry_filename),
        "MEMORY.md should no longer reference the deleted entry"
    );

    // -- Phase 6: Verify the content is gone from the system prompt -----------

    let prompt_after_delete = build_system_prompt(
        &mut SystemPromptCache::new(),
        None,
        "/tmp",
        "test-model",
        &[],
        None,
        Some(&mem_dir),
        false,
        false,
        false, // browser_enabled
    );

    assert!(
        !prompt_after_delete.contains(&entry_filename),
        "prompt should no longer contain the deleted entry filename"
    );
    assert!(
        !prompt_after_delete.contains("current sprint goals"),
        "prompt should no longer contain the deleted entry summary"
    );
    // With everything removed, the index is empty — the prompt should show the empty state
    assert!(
        prompt_after_delete.contains("currently empty"),
        "prompt should show empty memory state after all entries are removed"
    );
}
