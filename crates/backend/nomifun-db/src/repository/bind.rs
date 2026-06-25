//! Shared dynamic-bind helpers for repositories that build SQL with a
//! runtime-sized list of parameters.
//!
//! Several repositories (`sqlite_conversation`, `sqlite_cron`,
//! `sqlite_requirement`) assemble `UPDATE ... SET` / filtered `SELECT`
//! statements whose bind count is only known at runtime. They each used to
//! carry a private copy of this `BindValue` tagged union plus the per-query
//! `bind` dispatchers; this module centralizes them so the set of supported
//! bind types stays consistent across repositories.

/// Tagged union to carry heterogeneous bind values for dynamic SQL.
#[derive(Debug, Clone)]
pub(crate) enum BindValue {
    Str(String),
    OptStr(Option<String>),
    Bool(bool),
    I64(i64),
    OptI64(Option<i64>),
}

/// Binds a `BindValue` to a raw `sqlx::query::Query`.
pub(crate) fn bind_value<'q>(
    query: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
    val: &'q BindValue,
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
    match val {
        BindValue::Str(s) => query.bind(s.as_str()),
        BindValue::OptStr(s) => query.bind(s.as_deref()),
        BindValue::Bool(b) => query.bind(*b),
        BindValue::I64(n) => query.bind(*n),
        BindValue::OptI64(n) => query.bind(*n),
    }
}

/// Binds a `BindValue` to a `sqlx::query::QueryAs` (typed row output).
pub(crate) fn bind_value_as<'q, T>(
    query: sqlx::query::QueryAs<'q, sqlx::Sqlite, T, sqlx::sqlite::SqliteArguments<'q>>,
    val: &'q BindValue,
) -> sqlx::query::QueryAs<'q, sqlx::Sqlite, T, sqlx::sqlite::SqliteArguments<'q>> {
    match val {
        BindValue::Str(s) => query.bind(s.as_str()),
        BindValue::OptStr(s) => query.bind(s.as_deref()),
        BindValue::Bool(b) => query.bind(*b),
        BindValue::I64(n) => query.bind(*n),
        BindValue::OptI64(n) => query.bind(*n),
    }
}

/// Binds a `BindValue` to a `sqlx::query::QueryScalar` (single `i64` output).
pub(crate) fn bind_value_scalar<'q>(
    query: sqlx::query::QueryScalar<'q, sqlx::Sqlite, i64, sqlx::sqlite::SqliteArguments<'q>>,
    val: &'q BindValue,
) -> sqlx::query::QueryScalar<'q, sqlx::Sqlite, i64, sqlx::sqlite::SqliteArguments<'q>> {
    match val {
        BindValue::Str(s) => query.bind(s.as_str()),
        BindValue::OptStr(s) => query.bind(s.as_deref()),
        BindValue::Bool(b) => query.bind(*b),
        BindValue::I64(n) => query.bind(*n),
        BindValue::OptI64(n) => query.bind(*n),
    }
}
