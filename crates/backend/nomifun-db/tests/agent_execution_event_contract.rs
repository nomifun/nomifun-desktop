use std::collections::BTreeSet;

use nomifun_common::AgentExecutionEventKind;
use ts_rs::{Config, TS};

const MIGRATION: &str = include_str!("../migrations/037_unified_agent_execution.sql");
const TYPESCRIPT_BINDING: &str = include_str!(
    "../../../../ui/src/common/protocolBindings/AgentExecutionEventKind.ts"
);

fn quoted_values(source: &str, quote: char) -> BTreeSet<String> {
    let mut values = BTreeSet::new();
    let mut remaining = source;
    while let Some(start) = remaining.find(quote) {
        remaining = &remaining[start + quote.len_utf8()..];
        let end = remaining
            .find(quote)
            .expect("contract literal must have a closing quote");
        values.insert(remaining[..end].to_owned());
        remaining = &remaining[end + quote.len_utf8()..];
    }
    values
}

#[test]
fn rust_sql_and_typescript_share_one_exact_event_vocabulary() {
    let generated_typescript = AgentExecutionEventKind::export_to_string(&Config::default())
        .expect("canonical Rust event enum must generate a TypeScript binding");
    assert_eq!(
        TYPESCRIPT_BINDING, generated_typescript,
        "committed TypeScript binding is stale; regenerate the ts-rs export"
    );

    let sql_start = MIGRATION
        .find("event_type       TEXT NOT NULL CHECK (event_type IN (")
        .expect("event_type CHECK must exist");
    let sql_check = &MIGRATION[sql_start..];
    let sql_end = sql_check
        .find("    )),")
        .expect("event_type CHECK must remain bounded");
    let sql_values = quoted_values(&sql_check[..sql_end], '\'');

    let binding_start = generated_typescript
        .find("export type AgentExecutionEventKind =")
        .expect("ts-rs binding must export AgentExecutionEventKind");
    let binding = &generated_typescript[binding_start..];
    let binding_end = binding
        .find(';')
        .expect("generated TypeScript alias must be terminated");
    let typescript_values = quoted_values(&binding[..binding_end], '"');

    assert_eq!(sql_values.len(), 9, "durable event vocabulary changed");
    assert_eq!(typescript_values, sql_values);

    for wire in sql_values {
        let kind: AgentExecutionEventKind = wire
            .parse()
            .unwrap_or_else(|error| panic!("SQL event `{wire}` is not a Rust event kind: {error}"));
        assert_eq!(kind.as_str(), wire);
        assert_eq!(serde_json::to_value(kind).unwrap(), wire);
    }
}
