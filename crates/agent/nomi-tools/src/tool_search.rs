use async_trait::async_trait;
use serde_json::{Value, json};

use nomi_protocol::events::ToolCategory;
use nomi_types::tool::{JsonSchema, ToolResult};

use crate::{
    Tool,
    registry::{DeferredToolState, MAX_DEFERRED_SEARCH_MATCHES},
};

const MIN_INEXACT_QUERY_CHARS: usize = 3;

/// Built-in tool that searches for deferred tools and activates their full
/// schemas for subsequent provider turns.
/// Core tool (never deferred itself) — always available to the LLM.
pub struct ToolSearchTool {
    /// Live deferred catalog and activations shared with the registry that
    /// builds each provider request.
    deferred_state: DeferredToolState,
}

impl ToolSearchTool {
    pub fn new(deferred_state: DeferredToolState) -> Self {
        Self { deferred_state }
    }
}

#[async_trait]
impl Tool for ToolSearchTool {
    fn name(&self) -> &str {
        "ToolSearch"
    }

    fn description(&self) -> &str {
        "Search for deferred tools and activate their full schema for subsequent turns. \
         Use a provider tool name, original MCP tool name, MCP server name, or keyword \
         before calling a deferred tool; at most five best matches are activated per search."
    }

    fn input_schema(&self) -> JsonSchema {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Tool name or keyword to search for"
                }
            },
            "required": ["query"]
        })
    }

    fn is_concurrency_safe(&self, _input: &Value) -> bool {
        true
    }

    async fn execute(&self, input: Value) -> ToolResult {
        let query = input["query"].as_str().unwrap_or("").trim();
        if query.is_empty() {
            return ToolResult {
                content: "Error: query is required".to_string(),
                is_error: true,
                images: Vec::new(),
            };
        }
        let information_chars = query.chars().filter(|ch| ch.is_alphanumeric()).count();
        if information_chars < MIN_INEXACT_QUERY_CHARS
            && !self.deferred_state.has_exact_search_term(query)
        {
            return ToolResult {
                content: format!(
                    "Error: query must contain at least {MIN_INEXACT_QUERY_CHARS} letters or \
                     digits unless it exactly matches a deferred tool name"
                ),
                is_error: true,
                images: Vec::new(),
            };
        }

        let matched_defs = self.deferred_state.search_and_activate(query);

        if matched_defs.is_empty() {
            return ToolResult {
                content: format!("No deferred tools matching \"{}\" found.", query),
                is_error: false,
                images: Vec::new(),
            };
        }

        let matches: Vec<Value> = matched_defs
            .into_iter()
            .map(|d| {
                json!({
                    "name": d.name,
                    "description": d.description,
                    "parameters": d.input_schema
                })
            })
            .collect();

        debug_assert!(matches.len() <= MAX_DEFERRED_SEARCH_MATCHES);

        ToolResult {
            content: serde_json::to_string_pretty(&matches).unwrap_or_default(),
            is_error: false,
            images: Vec::new(),
        }
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Info
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::ToolRegistry;

    struct SearchTestTool {
        name: &'static str,
        description: &'static str,
        schema: Value,
        deferred: bool,
    }

    #[async_trait]
    impl Tool for SearchTestTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            self.description
        }

        fn input_schema(&self) -> JsonSchema {
            self.schema.clone()
        }

        fn is_concurrency_safe(&self, _input: &Value) -> bool {
            true
        }

        async fn execute(&self, _input: Value) -> ToolResult {
            ToolResult::text("ok")
        }

        fn category(&self) -> ToolCategory {
            ToolCategory::Info
        }

        fn is_deferred(&self) -> bool {
            self.deferred
        }
    }

    struct AliasedSearchTestTool {
        name: &'static str,
        aliases: Vec<String>,
    }

    #[async_trait]
    impl Tool for AliasedSearchTestTool {
        fn name(&self) -> &str {
            self.name
        }

        fn deferred_search_aliases(&self) -> Vec<String> {
            self.aliases.clone()
        }

        fn description(&self) -> &str {
            "aliased deferred tool"
        }

        fn input_schema(&self) -> JsonSchema {
            json!({"type": "object"})
        }

        fn is_concurrency_safe(&self, _input: &Value) -> bool {
            true
        }

        async fn execute(&self, _input: Value) -> ToolResult {
            ToolResult::text("ok")
        }

        fn category(&self) -> ToolCategory {
            ToolCategory::Info
        }

        fn is_deferred(&self) -> bool {
            true
        }
    }

    fn build_state() -> DeferredToolState {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(SearchTestTool {
            name: "Read",
            description: "Read a file",
            schema: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
            deferred: false,
        }));
        registry.register(Box::new(SearchTestTool {
            name: "AgentDelegateTool",
            description: "Delegate work to an Agent",
            schema: json!({"type": "object", "properties": {"agents": {"type": "array"}}}),
            deferred: true,
        }));
        registry.register(Box::new(SearchTestTool {
            name: "EnterPlanMode",
            description: "Enter plan mode",
            schema: json!({"type": "object", "properties": {}}),
            deferred: true,
        }));
        registry.deferred_state()
    }

    #[tokio::test]
    async fn search_by_exact_name() {
        let state = build_state();
        let tool = ToolSearchTool::new(state.clone());
        let result = tool
            .execute(json!({"query": "AgentDelegateTool"}))
            .await;
        assert!(!result.is_error);
        assert!(result.content.contains("AgentDelegateTool"));
        assert!(result.content.contains("Delegate work to an Agent"));
        assert!(result.content.contains("parameters"));
        assert!(state.is_activated("AgentDelegateTool"));
        assert!(!state.is_activated("EnterPlanMode"));
    }

    #[tokio::test]
    async fn search_case_insensitive() {
        let state = build_state();
        let tool = ToolSearchTool::new(state.clone());
        let result = tool
            .execute(json!({"query": "agentdelegatetool"}))
            .await;
        assert!(!result.is_error);
        assert!(result.content.contains("AgentDelegateTool"));
        assert!(state.is_activated("AgentDelegateTool"));
    }

    #[tokio::test]
    async fn search_by_description_keyword() {
        let state = build_state();
        let tool = ToolSearchTool::new(state.clone());
        let result = tool.execute(json!({"query": "plan"})).await;
        assert!(!result.is_error);
        assert!(result.content.contains("EnterPlanMode"));
        assert!(state.is_activated("EnterPlanMode"));
    }

    #[tokio::test]
    async fn search_excludes_non_deferred() {
        let state = build_state();
        let tool = ToolSearchTool::new(state.clone());
        let result = tool.execute(json!({"query": "Read"})).await;
        // "Read" is not deferred, should not appear in results
        assert!(
            !result.content.contains("\"name\": \"Read\"")
                || result.content.contains("No deferred tools")
        );
        assert!(!state.is_activated("Read"));
    }

    #[tokio::test]
    async fn search_no_match() {
        let state = build_state();
        let tool = ToolSearchTool::new(state.clone());
        let result = tool.execute(json!({"query": "nonexistent"})).await;
        assert!(!result.is_error);
        assert!(result.content.contains("No deferred tools"));
        assert!(state.activated_identities().is_empty());
    }

    #[tokio::test]
    async fn search_empty_query_returns_error() {
        let state = build_state();
        let tool = ToolSearchTool::new(state.clone());
        let result = tool.execute(json!({"query": ""})).await;
        assert!(result.is_error);
        assert!(state.activated_identities().is_empty());
    }

    #[tokio::test]
    async fn search_whitespace_only_query_returns_error() {
        let state = build_state();
        let tool = ToolSearchTool::new(state.clone());

        let result = tool.execute(json!({"query": "  \t\n  "})).await;

        assert!(result.is_error);
        assert!(state.activated_identities().is_empty());
    }

    #[tokio::test]
    async fn search_rejects_low_information_inexact_query() {
        let state = build_state();
        let tool = ToolSearchTool::new(state.clone());

        let result = tool.execute(json!({"query": "a"})).await;

        assert!(result.is_error);
        assert!(result.content.contains("at least 3"));
        assert!(state.activated_identities().is_empty());
    }

    #[tokio::test]
    async fn short_query_is_allowed_for_exact_tool_name() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(SearchTestTool {
            name: "AI",
            description: "Short exact tool",
            schema: json!({"type": "object"}),
            deferred: true,
        }));
        let state = registry.deferred_state();
        let tool = ToolSearchTool::new(state.clone());

        let result = tool.execute(json!({"query": "  ai  "})).await;

        assert!(!result.is_error);
        assert!(result.content.contains("\"name\": \"AI\""));
        assert_eq!(state.activated_identities(), vec!["AI".to_string()]);
    }

    #[tokio::test]
    async fn short_exact_origin_alias_is_searchable_but_result_uses_provider_name() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(AliasedSearchTestTool {
            name: "ProviderCanonicalAI",
            aliases: vec!["A".to_owned(), "original_tool".to_owned()],
        }));
        let state = registry.deferred_state();
        let tool = ToolSearchTool::new(state.clone());

        let result = tool.execute(json!({"query": "a"})).await;

        assert!(!result.is_error);
        let matches: Vec<Value> = serde_json::from_str(&result.content).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["name"], "ProviderCanonicalAI");
        assert_eq!(
            state.activated_identities(),
            vec!["ProviderCanonicalAI".to_string()]
        );
    }

    #[tokio::test]
    async fn provider_exact_name_outranks_another_tools_informational_alias() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(AliasedSearchTestTool {
            name: "ExactProviderRoute",
            aliases: vec!["origin_a".to_owned()],
        }));
        registry.register(Box::new(AliasedSearchTestTool {
            name: "DifferentProviderRoute",
            aliases: vec!["ExactProviderRoute".to_owned()],
        }));
        let state = registry.deferred_state();
        let tool = ToolSearchTool::new(state.clone());

        let result = tool.execute(json!({"query": "ExactProviderRoute"})).await;

        let matches: Vec<Value> = serde_json::from_str(&result.content).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["name"], "ExactProviderRoute");
        assert!(state.is_activated("ExactProviderRoute"));
        assert!(!state.is_activated("DifferentProviderRoute"));
    }

    #[tokio::test]
    async fn exact_name_activates_only_exact_match() {
        let mut registry = ToolRegistry::new();
        for name in ["Plan", "PlanExtended", "RePlan"] {
            registry.register(Box::new(SearchTestTool {
                name,
                description: "plan helper",
                schema: json!({"type": "object"}),
                deferred: true,
            }));
        }
        let state = registry.deferred_state();
        let tool = ToolSearchTool::new(state.clone());

        let result = tool.execute(json!({"query": " Plan "})).await;

        assert!(!result.is_error);
        let matches: Vec<Value> = serde_json::from_str(&result.content).unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["name"], "Plan");
        assert_eq!(state.activated_identities(), vec!["Plan".to_string()]);
    }

    #[tokio::test]
    async fn broad_query_activates_at_most_five_deterministic_matches() {
        let mut registry = ToolRegistry::new();
        for name in [
            "AlphaTool",
            "BetaTool",
            "DeltaTool",
            "EpsilonTool",
            "EtaTool",
            "GammaTool",
            "ThetaTool",
        ] {
            registry.register(Box::new(SearchTestTool {
                name,
                description: "shared broad keyword",
                schema: json!({"type": "object"}),
                deferred: true,
            }));
        }
        let state = registry.deferred_state();
        let tool = ToolSearchTool::new(state.clone());

        let result = tool.execute(json!({"query": "broad"})).await;

        assert!(!result.is_error);
        let matches: Vec<Value> = serde_json::from_str(&result.content).unwrap();
        assert_eq!(matches.len(), MAX_DEFERRED_SEARCH_MATCHES);
        assert_eq!(
            matches
                .iter()
                .map(|entry| entry["name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["AlphaTool", "BetaTool", "DeltaTool", "EpsilonTool", "EtaTool"]
        );
        assert_eq!(
            state.activated_identities().len(),
            MAX_DEFERRED_SEARCH_MATCHES
        );
        assert!(!state.is_activated("GammaTool"));
        assert!(!state.is_activated("ThetaTool"));
    }

    #[tokio::test]
    async fn search_sees_tools_registered_after_tool_search_is_created() {
        let mut registry = ToolRegistry::new();
        let state = registry.deferred_state();
        let search = ToolSearchTool::new(state.clone());
        registry.register(Box::new(SearchTestTool {
            name: "DynamicDeferred",
            description: "Registered after ToolSearch",
            schema: json!({
                "type": "object",
                "properties": {"required_value": {"type": "string"}},
                "required": ["required_value"]
            }),
            deferred: true,
        }));

        let result = search.execute(json!({"query": "DynamicDeferred"})).await;

        assert!(!result.is_error);
        assert!(result.content.contains("required_value"));
        assert!(state.is_activated("DynamicDeferred"));
        let definition = registry
            .to_tool_defs()
            .into_iter()
            .find(|definition| definition.name == "DynamicDeferred")
            .unwrap();
        assert!(!definition.deferred);
        assert_eq!(definition.input_schema["required"][0], "required_value");
    }
}
