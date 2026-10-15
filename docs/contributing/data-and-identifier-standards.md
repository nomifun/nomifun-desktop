# Data and Identifier Standards

This is the repository-wide contributor standard for database schema, identity,
logical references, dataset reset, backup/restore, and related protocol
boundaries. It is mandatory for new code and changes to existing code.

The authority order is:

1. [`001_canonical_baseline.sql`](../../crates/backend/nomifun-db/migrations/001_canonical_baseline.sql)
   and
   [`id_schema_contract.rs`](../../crates/backend/nomifun-db/src/id_schema_contract.rs)
   — executable schema and runtime registry;
2. [`architecture/id-system.md`](../architecture/id-system.md) and
   [`architecture/data-and-storage.md`](../architecture/data-and-storage.md) —
   architecture contract and storage behavior;
3. this page — contributor workflow and review checklist.

Historical decisions and audit evidence live in Git history. They cannot
override the current architecture contract.

If implementation and documentation disagree, do not invent a compatibility
exception. Update the implementation and its authoritative documentation
together, or stop and request an architecture decision.

## 1. Mandatory product-table shape

Every NomiFun-owned persistent product table must begin with the same technical
primary key:

```sql
id INTEGER PRIMARY KEY AUTOINCREMENT
```

This applies to the non-Agent product tables in the runtime registry. The
generation-5 Agent Store tables listed by `agent_store_schema_manifest_payload`
are the deliberate exception: they use canonical TEXT/composite keys and
physical foreign keys for the single Session/Event/Effect fact graph. SQLite
internal tables, migration metadata, and temporary tables are not product
tables.

The technical `id` is local to one table and one active dataset. It is:

- an internal storage key and primary-index entry;
- represented as `i64` inside repository/storage code;
- allowed to have gaps and never assumed to be contiguous;
- preserved inside an exact current-generation SQLite backup/restore snapshot;
  a hard reset creates a new dataset without importing old rows;
- forbidden as a product API, event, file, manifest, backup-graph, or
  cross-dataset locator. Opaque database snapshots may retain it as storage data,
  never as a portable relationship key.

Do not rename this column to `row_id`, `table_id`, or another variant. Do not
copy one table's technical `id` into another table.

The current v3 lineage is a clean baseline, not a promise to preserve every
historical migration path. A schema change must first establish whether the
v3 baseline, a new lineage, or an explicitly approved release migration is the
correct target. Do not add compatibility migrations, legacy mappers, or
dual-read/dual-write paths merely to avoid making that decision.

## 2. Stable business IDs

Add a business ID only when an entity must remain addressable outside its local
database row: across databases, devices, managed files, APIs, events, backup
graphs, or side stores.

Use one field-specific name, for example:

```text
user_id
conversation_id
message_id
provider_id
execution_id
knowledge_base_id
webhook_id
credential_id
```

The value is a bare canonical UUIDv7:

```text
0190f5fe-7c00-7a00-8000-000000000003
```

The fixed contract is:

- standard `8-4-4-4-12` representation, exactly 36 characters;
- lowercase hexadecimal and standard hyphens;
- UUID version 7 and the RFC UUID variant;
- full 128-bit value, without truncation;
- stored as SQLite `TEXT` and serialized as a JSON string;
- no `prefix_UUIDv7`, suffix, braces, compact form, whitespace, or custom
  separator.

The field name, table meaning, and Rust/TypeScript domain type identify the
entity kind. The UUID text must not encode a type prefix. Do not introduce a
generic wire `id` when a named business field is available.

Do not give every row a UUID for visual uniformity. Internal-only relation,
singleton, cache, and event rows keep only the mandatory technical `id` unless
there is a real product or cross-store locator requirement.

## 3. Distinguish ID categories

The suffix `_id` does not by itself decide semantics. Before adding or
validating a field, classify it as one of these:

| Category | Rule |
| --- | --- |
| Technical row ID | The fixed local `id`; never crosses a repository or product boundary. |
| Stable business ID | Named bare UUIDv7 for a product-addressable entity. |
| Natural key | Domain-defined value such as a name, slug, URL, locale, or singleton key; preserve its format. |
| External ID | Opaque value issued by a provider or protocol, such as a platform user/chat ID or an upstream provider request ID. |
| Operation token | Request ID, idempotency key, nonce, workspace token, or receipt token; purpose-specific and not an entity identity. |
| Document identity | Canvas node/edge or similar document-local identity; not a database primary key. |

Protocol-specific UUIDv7 values need the same explicit classification. Do not
infer identity or reference semantics from a column suffix alone.

## 4. Logical references replace physical foreign keys

Non-Agent product DDL must not contain the following. The current frozen
exception is the canonical Agent Store tables, which retain the physical keys
and cascade actions declared in the baseline. Registered guard/validator
triggers in channel, requirements, terminal, creation and Agent domains remain
explicit schema-contract exceptions. Published migration files are immutable;
retired application tables are removed through forward migration rather than
checksum changes. No new domain may add physical constraints, cascades or
triggers without an explicit schema-contract update.

```text
FOREIGN KEY
REFERENCES
CREATE TRIGGER
ON DELETE CASCADE
ON UPDATE CASCADE
*_row_id
```

Store exactly one logical reference for one relationship:

- reference the parent's named business ID when the parent is
  product-addressable;
- use an owner business ID plus a sequence, natural key, or composite
  condition for internal-only rows;
- preserve an explicitly named opaque external ID when another system owns the
  identifier;
- never store both `conversation_id` and `conversation_row_id`, or any
  equivalent business-ID/row-ID pair.

Logical references are not informal conventions. Every reference must be
registered in `id_schema_contract.rs` (including JSON/side-store references
where applicable) with:

- child table and column, parent table and target column;
- kind, value contract, nullability, and scope/predicates;
- required index;
- delete policy: `RESTRICT`, application-level `CASCADE`, `SET_NULL`, or
  `KEEP_HISTORY`;
- backup/restore preservation and any explicitly supported import policy;
- orphan-audit policy/query.

Repositories and services are the only normal write path. They must validate
parents, enforce aggregate ownership, and apply the registered delete policy
inside explicit transactions. Application-level `CASCADE` is service behavior,
never a SQLite cascade or trigger. Raw SQL is limited to controlled fixtures,
diagnostics, and maintenance.

After restore/import, run the complete registered orphan audit across the
database and managed side stores. A missing physical FK is not permission to
accept unchecked orphan rows.

## 5. Dataset lineage and hard reset

The current baseline is intentionally incompatible with historical product
datasets. Startup identifies lineage and exact canonical migration-chain
compatibility. An absent dataset initializes the current baseline. Only the
application startup dataset coordinator may retire a readable incompatible
managed lineage and replace it with an empty current dataset. It does not
convert or import historical rows.

Database I/O failures and damage to a supported schema/data fail closed and
preserve the source. Migration-row counts do not establish application-version
ordering across destructive baselines. Backup, restore and standalone database
initialization do not inherit startup rebuild authority.

Forbidden compatibility behavior:

- table-by-table conversion or historical data migration;
- legacy ID normalization or old-to-new maps;
- compatibility reads, aliases, dual-read, or dual-write;
- selectively copying old JSON, caches, workspace indexes, or side-store rows;
- continuing startup after a partial reset.

The reset scope includes the SQLite database and its WAL/SHM sidecars plus all
managed side stores. Reset writes and finalizes the generation/reset receipt
before serving requests. A reset failure is fail-closed. External
user-owned workspaces are not deleted, but their historical database
references are not imported. Agent clean cut is a distinct operation: it clears
Agent data while preserving non-Agent configuration.

## 6. Backup, restore, and clone

The current generation-5 offline bundle accepts only the exact supported
`uarc-generation-5` schema and lineage. The implementation is
[`backup_bundle.rs`](../../crates/backend/nomifun-db/src/backup_bundle.rs).

- Back up a WAL-safe SQLite snapshot and the declared managed files as one unit.
- Restore the validated database snapshot as the same SQLite authority, retaining
  canonical TEXT/composite keys, physical Agent Store references, Session/Event/
  Payload/Effect facts, stable business UUIDv7 values and technical row `id`s.
- Do not remap rows, regenerate IDs or reconstruct a second logical graph.
- Rotate only the destination `storage-generation` namespace; preserve business
  and canonical identities. Managed workspace paths follow the existing portable
  restore policy, without importing external user projects.
- Reject incompatible historical bundles, invalid data and partial installs;
  validate the schema/data and complete registered reference audits before
  publishing the staged destination.

This is snapshot restoration into a new destination, not row merge/import or a
historical migration. A copy/clone cannot silently mint business IDs or acquire
startup rebuild authority. Retained technical IDs remain internal storage data,
not portable product graph locators.

## 7. Boundary and type rules

`nomifun-common` owns bare UUIDv7 generation and strict validation. Stable
business IDs should use small Rust domain newtypes and TypeScript branded
types where the boundary benefits from it.

At HTTP, WebSocket, MCP, Gateway, event, cache, filesystem, and backup
boundaries:

- use the named business or external field type from the domain model;
- keep technical `id` inside repository/storage implementation details;
- reject malformed values instead of coercing them to `0`, empty strings, or
  another ID kind;
- do not expose a generic numeric `id` as a portable product locator.

## 8. Required tests and verification

A database or identifier change must add or update focused tests for the
affected contract. At minimum, cover the applicable items:

- every non-Agent product table has `id INTEGER PRIMARY KEY AUTOINCREMENT`, and
  canonical Agent Store tables match their frozen TEXT/composite-key schema;
- non-Agent product DDL contains no physical FKs, `REFERENCES`, database
  cascades, or `*_row_id`; required guard triggers stay exact;
- accepted business IDs are canonical UUIDv7 and old/prefixed/short formats
  are rejected;
- logical-reference registry entries, indexes, scope checks, and delete
  policies;
- repository parent validation and transaction behavior;
- orphan audits, including JSON/side-store references;
- dataset lineage detection, hard reset, generation, and reset receipt;
- backup/restore preservation of the exact generation-5 SQLite identity graph.

Delete stale or misleading tests instead of preserving pseudo-coverage. Use
the narrowest targeted test first; reserve expensive workspace-wide gates for
the final integration stage.

For documentation-only changes, run at least:

```bash
git diff --check
```

## 9. New table or identifier checklist

Before opening a review:

1. Read this standard and the architecture ID contract.
2. Add the fixed `id` column to every new product table.
3. Decide whether a named business UUIDv7 is genuinely required.
4. Classify every `_id` field as business, natural, external, token, document,
   or logical reference.
5. Remove technical row-ID propagation and physical FK/trigger designs.
6. Register every logical relation, index, lifecycle policy, rebuild policy,
   and orphan audit.
7. Keep technical IDs out of wire/event/product locators; exact database backup
   snapshots preserve storage IDs without exposing a portable row-ID contract.
8. Add focused schema, repository, boundary, reset, or restore tests.
9. Review the complete managed-data reset impact.
10. Keep English and Simplified Chinese documentation synchronized.

## 10. Repository hygiene

Do not commit local data, secrets, generated builds, dependency trees, or
intermediate products. In particular, keep these out of Git:

```text
.tmp*/
.tmp***
target/
build.noindex/
node_modules/
dist/
coverage/
*.o
*.rmeta
*.rlib
```

If a tool produces a non-standard temporary directory or dependency artifact,
add it to the local ignore policy before continuing. Never "clean up" another
developer's generated files by adding them to a commit.
