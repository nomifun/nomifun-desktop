pub mod context_modifier;
pub mod executor;
pub mod frontmatter;
pub mod hooks;
pub mod loader;
pub mod mcp;
pub mod paths;
pub mod permissions;
pub mod prompt;
pub mod shell;
pub mod substitution;
pub mod types;

#[cfg(test)]
mod permissions_supplemental_tests;

#[cfg(test)]
#[path = "integration_tests.rs"]
mod integration_tests;
