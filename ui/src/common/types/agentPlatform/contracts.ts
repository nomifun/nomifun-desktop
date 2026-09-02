import type { EntityId } from '@/common/types/ids';

declare const digestBrand: unique symbol;
declare const catalogIdBrand: unique symbol;

export type AgentPresetId = EntityId<'agent-preset'>;
export type AgentSessionId = EntityId<'agent-session'>;
export type RemoteBindingId = EntityId<'remote-binding'>;
export type ResolvedSnapshotId = EntityId<'resolved-snapshot'>;
export type DigestHex = string & { readonly [digestBrand]: 'sha256' };
export type CatalogId<Kind extends string> = string & { readonly [catalogIdBrand]: Kind };
export type CapabilityId = CatalogId<'capability'>;
export type SkillId = CatalogId<'skill'>;
export type PackageId = CatalogId<'package'>;

export const OFFICIAL_PRESET_KEYS = [
  'chat.minimal',
  'assistant.general',
  'coding.codex',
  'companion.default',
  'robot.default',
  'customer-service.default',
  'creative-studio.default',
] as const;

export type OfficialPresetKey = (typeof OFFICIAL_PRESET_KEYS)[number];
export type AgentPresetSource = 'official' | 'user' | 'package';
export type CapabilityExposure = 'advertised' | 'discoverable' | 'hidden';
export type CatalogMaterializationState = 'materialized' | 'unavailable';
export const AGENT_CHAT_MODEL_TASK = 'agent_chat' as const;
export const CHAT_ROUTE_RECORD_SCHEMA = 'nomifun.chat-route-record.v1' as const;
export type ChatRouteProtocol =
  | 'anthropic'
  | 'openai_chat'
  | 'openai_responses'
  | 'gemini'
  | 'bedrock'
  | 'vertex';
export type ChatRouteFeature =
  | 'text_input'
  | 'image_input'
  | 'audio_input'
  | 'text_output'
  | 'audio_output'
  | 'tool_calls'
  | 'reasoning'
  | 'reasoning_signature'
  | 'prompt_cache'
  | 'structured_output'
  | 'provider_round_state'
  | 'native_responses_items';

export interface ChatRouteCandidate {
  model_route_id: string;
  model_route_revision: number;
  provider_id: string;
  model: string;
  protocol: ChatRouteProtocol;
  connection_config_ref: string;
  config_revision_digest: DigestHex;
  credential_ref: string;
  features: ChatRouteFeature[];
}

export interface ChatRouteRecord {
  schema: typeof CHAT_ROUTE_RECORD_SCHEMA;
  task: typeof AGENT_CHAT_MODEL_TASK;
  primary: ChatRouteCandidate;
  failovers: ChatRouteCandidate[];
}

export interface ExactCatalogRef<Kind extends string = string> {
  id: CatalogId<Kind>;
  version: string;
}

export interface PresetRevisionRef {
  preset_id: AgentPresetId;
  revision: number;
  revision_digest: DigestHex;
}

export interface ResolvedSnapshotRef {
  snapshot_id: ResolvedSnapshotId;
  snapshot_digest: DigestHex;
}

export interface TypedResourceBinding {
  binding_id: string;
  resource_kind: string;
  resource_id: string;
  owner_id: string;
  operations: string[];
  connection_config_ref?: string;
  typed_parameters?: Record<string, string>;
}

export interface AgentBindingValue {
  preset_revision_ref: PresetRevisionRef;
  resolved_snapshot_ref: ResolvedSnapshotRef;
  typed_resource_bindings: TypedResourceBinding[];
  binding_version: number;
}

export interface CapabilitySelection {
  capability: ExactCatalogRef<'capability'>;
  required: boolean;
  exposure: CapabilityExposure;
  action_allowlist?: string[];
  resource_binding_refs?: string[];
  destination_constraints?: string[];
  context_budget_override?: number;
  tool_budget_override?: number;
  config: unknown;
}

export interface ExactRoleContractRef {
  key: {
    role_id: string;
    contract_version: string;
  };
  contract_digest: DigestHex;
}

export interface RoleProviderSelection {
  role: ExactRoleContractRef;
  provider_mount_id: string;
}

export interface AgentPresetDocument {
  schema_version: string;
  surfaces: string[];
  model_route_refs: Record<string, string>;
  chat_route_records: Partial<Record<typeof AGENT_CHAT_MODEL_TASK, ChatRouteRecord>>;
  initial_capabilities: CapabilitySelection[];
  on_demand_capabilities: CapabilitySelection[];
  skill_bindings: ExactCatalogRef<'skill'>[];
  resource_bindings: TypedResourceBinding[];
  system_role_provider_overrides: Record<string, RoleProviderSelection>;
  persona: string;
  instructions: string;
  context_policy: Record<string, unknown>;
  execution_constraints: Record<string, unknown>;
  runtime_budget: Record<string, unknown>;
}

export interface AgentPresetDraft {
  preset_id: AgentPresetId;
  display_name: string;
  description?: string;
  /** Editor/request-only template provenance; never persisted in AgentPreset facts. */
  source_template_key?: OfficialPresetKey;
  current_revision?: PresetRevisionRef;
  document: AgentPresetDocument;
}

export interface AgentPresetSummary {
  preset_id: AgentPresetId;
  owner_user_id?: string;
  source: AgentPresetSource;
  display_name: string;
  description?: string;
  current_stable_revision?: PresetRevisionRef;
  bound_target_count: number;
}

export interface TypedResourceDefault {
  slot_key: string;
  resource_kind: string;
  required: boolean;
  operations: string[];
  binding_policy: 'require_explicit_selection' | 'select_only_owned_resource' | 'leave_unbound';
}

export interface OfficialPresetSeed {
  initial_capabilities: ExactCatalogRef<'capability'>[];
  on_demand_capabilities: ExactCatalogRef<'capability'>[];
  skill_bindings: ExactCatalogRef<'skill'>[];
  typed_resource_defaults: TypedResourceDefault[];
  required_runtime_features: string[];
}

export interface OfficialPresetRoleCoverage {
  required_capability_categories: string[];
  required_capability_ids: CapabilityId[];
  required_runtime_features: string[];
  required_resource_kinds: string[];
}

export interface OfficialPresetTemplate {
  template_key: OfficialPresetKey;
  seed: OfficialPresetSeed;
  role_coverage: OfficialPresetRoleCoverage;
  immutable: true;
  forkable: true;
}

export interface AgentBindingSummary {
  target_kind: string;
  target_id: string;
  preset_revision_ref: PresetRevisionRef;
  resolved_snapshot_ref: ResolvedSnapshotRef;
  binding_version: number;
}

export interface AgentPresetLibraryResponse {
  official_templates: OfficialPresetTemplate[];
  user_presets: AgentPresetSummary[];
  active_bindings: AgentBindingSummary[];
  fresh_start: {
    data_generation: 4;
    legacy_data_imported: false;
    official_template_count: number;
    user_preset_count: number;
  };
}

export interface CapabilityCatalogItem {
  capability: ExactCatalogRef<'capability'>;
  kind: string;
  display_name: string;
  description: string;
  source_package: ExactCatalogRef<'package'>;
  source_kind: string;
  materialization_state: CatalogMaterializationState;
  unavailable_code?: string;
  supported_surfaces: string[];
  required_runtime_features: string[];
  required_resource_kinds: string[];
  required_capabilities: ExactCatalogRef<'capability'>[];
  conflicting_capabilities: ExactCatalogRef<'capability'>[];
  action_count: number;
  context_contributor_count: number;
}

export interface SkillCatalogItem {
  skill: ExactCatalogRef<'skill'>;
  display_name: string;
  description: string;
  source_package: ExactCatalogRef<'package'>;
  source_kind: string;
  required_capabilities: ExactCatalogRef<'capability'>[];
  supported_surfaces: string[];
}

export interface McpToolCatalogItem {
  server_id: string;
  canonical_tool_key: string;
  capability: ExactCatalogRef<'capability'>;
  source_package: ExactCatalogRef<'package'>;
  schema_digest: DigestHex;
  materialization_version: string;
}

export interface AgentCatalogResponse {
  capabilities: CapabilityCatalogItem[];
  skills: SkillCatalogItem[];
  mcp_tools: McpToolCatalogItem[];
}

export type PreviewStatus = 'ready' | 'blocked';
export type PreviewDiagnosticSeverity = 'error' | 'warning' | 'info';

export interface PreviewDiagnostic {
  severity: PreviewDiagnosticSeverity;
  code: string;
  message: string;
  subject?: string;
  details?: unknown;
}

export interface PreviewCapability {
  capability: ExactCatalogRef<'capability'>;
  display_name: string;
  source_package: ExactCatalogRef<'package'>;
  dependency_path: CapabilityId[];
  required_runtime_features: string[];
}

export interface PreviewSummary {
  initial_count: number;
  on_demand_count: number;
  active_at_start_count: number;
  model_tool_count: number;
  context_contributor_count: number;
  on_demand_index_count: number;
  skill_count: number;
  mcp_count: number;
  resource_binding_count: number;
  provider_initialization_count: number;
}

export interface RevisionDiff {
  added_initial: CapabilityId[];
  removed_initial: CapabilityId[];
  added_on_demand: CapabilityId[];
  removed_on_demand: CapabilityId[];
  added_skills: SkillId[];
  removed_skills: SkillId[];
  resource_bindings_changed: boolean;
  model_routes_changed: boolean;
  instructions_changed: boolean;
}

export interface SnapshotInspector {
  snapshot_ref?: ResolvedSnapshotRef;
  preset_revision_ref?: PresetRevisionRef;
  runtime_profile?: 'coding_native' | 'managed_minimal';
  required_runtime_protocol_version: string;
  required_runtime_features: string[];
  initial_capabilities: PreviewCapability[];
  on_demand_capabilities: PreviewCapability[];
  compact_on_demand_index: CapabilityId[];
  tool_schema_refs: string[];
  context_schema_refs: string[];
  mcp_materializations: McpToolCatalogItem[];
  typed_resource_bindings: TypedResourceBinding[];
  service_key_diagnostics: string[];
}

export interface ResolveAgentPresetPreviewRequest {
  expected_current_revision?: PresetRevisionRef;
  draft: AgentPresetDraft;
  scene: 'agent_settings';
  surface: 'desktop';
  audience: 'owner';
}

export interface ResolveAgentPresetPreviewResponse {
  status: PreviewStatus;
  draft_digest: DigestHex;
  preview_digest: DigestHex;
  candidate_revision_ref: PresetRevisionRef;
  resolved_snapshot_ref?: ResolvedSnapshotRef;
  summary: PreviewSummary;
  diagnostics: PreviewDiagnostic[];
  revision_diff: RevisionDiff;
  inspector: SnapshotInspector;
  can_save_revision: boolean;
  can_create_session: boolean;
}

export interface AgentPresetRevision {
  reference: PresetRevisionRef;
  document: AgentPresetDocument;
  created_by: string;
  created_at_ms: number;
  reason?: string;
}

export interface AgentPresetEditorResponse {
  preset: AgentPresetSummary;
  revision?: AgentPresetRevision;
  draft: AgentPresetDraft;
}

export interface CreateAgentPresetRequest {
  display_name: string;
  description?: string;
  fork_from_revision?: PresetRevisionRef;
}

export interface CreateAgentPresetFromTemplateRequest {
  display_name: string;
  description?: string;
  resource_bindings: TemplateResourceSelection[];
  model_route_refs: Record<string, string>;
  chat_route_records: Partial<Record<typeof AGENT_CHAT_MODEL_TASK, ChatRouteRecord>>;
}

export interface TemplateResourceSelection {
  slot_key: string;
  resource_kind: string;
  resource_id: string;
  connection_config_ref?: string;
  typed_parameters?: Record<string, string>;
}

export interface SaveAgentPresetRevisionRequest {
  expected_current_revision?: PresetRevisionRef;
  preview_digest: DigestHex;
  draft: AgentPresetDraft;
  reason?: string;
}

export interface SaveAgentPresetRevisionResponse {
  preset: AgentPresetSummary;
  revision: AgentPresetRevision;
  resolved_snapshot_ref: ResolvedSnapshotRef;
  preview_digest: DigestHex;
}

export interface AgentBindingTarget {
  target_kind: string;
  target_id: string;
}

export interface AgentBindingRecord {
  target: AgentBindingTarget;
  owner_user_id: string;
  agent_binding: AgentBindingValue;
}

export interface PutAgentBindingRequest {
  expected_binding_version?: number;
  agent_binding: AgentBindingValue;
}

export interface RemoteBinding {
  remote_binding_id: RemoteBindingId;
  owner_user_id: string;
  name: string;
  agent_binding: AgentBindingValue;
}

export interface CreateRemoteBindingRequest {
  name: string;
  agent_binding: AgentBindingValue;
}

export interface UpdateRemoteBindingRequest {
  expected_binding_version: number;
  expected_agent_binding_digest: DigestHex;
  name: string;
  agent_binding: AgentBindingValue;
}

export type RemoteOpenState =
  | { state: 'opening' }
  | { state: 'ready' }
  | { state: 'failed'; code: string; recoverable: boolean };

export interface RemoteOpenRequest {
  binding_id: RemoteBindingId;
  idempotency_key: string;
  initial_input?: unknown;
}

export interface RemoteOpenResponse {
  agent_session_id: AgentSessionId;
  agent_binding: AgentBindingValue;
  open_state: RemoteOpenState;
  cursor: SessionCursor;
}

export interface RemoteTurnRequest {
  agent_session_id: AgentSessionId;
  input: unknown;
  idempotency_key: string;
}

export interface RemoteObserveRequest {
  agent_session_id: AgentSessionId;
  after_cursor: SessionCursor;
  limit: number;
}

export interface RemoteCancelRequest {
  agent_session_id: AgentSessionId;
  idempotency_key: string;
}

export interface RemoteMutationResponse {
  agent_session_id: AgentSessionId;
  cursor: SessionCursor;
  session_status: string;
}

export interface RemoteObserveResponse {
  agent_session_id: AgentSessionId;
  events: unknown[];
  messages: unknown[];
  next_cursor: SessionCursor;
}

export interface SessionCursor {
  agent_session_id: AgentSessionId;
  seq: number;
}

export interface CreateAgentSessionRequest {
  agent_binding: AgentBindingValue;
  title?: string;
}

export interface CreateAgentSessionResponse {
  agent_session_id: AgentSessionId;
  agent_binding: AgentBindingValue;
  state: string;
  cursor: SessionCursor;
}

export interface CreateAgentSessionTurnRequest {
  input: unknown;
  idempotency_key: string;
}

export interface CreateAgentSessionTurnResponse {
  agent_session_id: AgentSessionId;
  operation_id: string;
  cursor: SessionCursor;
  status: string;
}

export interface AgentSessionEventsResponse {
  agent_session_id: AgentSessionId;
  events: unknown[];
  messages: unknown[];
  next_cursor: SessionCursor;
}

export interface ForkAgentSessionRequest {
  target_agent_binding: AgentBindingValue;
  parent_through_seq: number;
  title?: string;
}

export interface ForkAgentSessionResponse {
  parent_agent_session_id: AgentSessionId;
  child_agent_session_id: AgentSessionId;
  child_agent_binding: AgentBindingValue;
  parent_through_seq: number;
  child_base_is_self_contained: true;
  copies_full_transcript: false;
  migrates_runtime_private_handles: false;
  replays_tool_or_effect: false;
}

export interface SnapshotContractMismatch {
  kind: string;
  subject: string;
  expected: string;
  actual?: string;
}

export type SnapshotCompatibilityView =
  | {
      result: 'compatible_exact';
      runtime_release_digest: DigestHex;
      hello_payload_digest: DigestHex;
    }
  | {
      result: 'executor_unavailable';
      error_code: 'SNAPSHOT_EXECUTOR_UNAVAILABLE';
      mismatches: SnapshotContractMismatch[];
    };

export interface AgentSessionContinuationView {
  agent_session_id: AgentSessionId;
  compatibility: SnapshotCompatibilityView;
  history_read_only: boolean;
  can_continue_same_session: boolean;
  requires_explicit_fork: boolean;
  fork_request?: ForkAgentSessionRequest;
}

export type InstallationTokenStatus = 'unconfigured' | 'active' | 'revoked';

export interface RemoteCredentialContinuation {
  requires_same_owner: true;
  requires_explicit_agent_session_id: true;
  implicit_session_lookup: false;
  auth_error_code: 'REMOTE_AUTH_REQUIRED';
  rest_status: 401;
}

export interface InstallationTokenStateResponse {
  status: InstallationTokenStatus;
  configured: boolean;
  continuation: RemoteCredentialContinuation;
}

export interface RotateInstallationTokenResponse {
  access_token: string;
  status: 'active';
  shown_once: true;
  existing_sessions_unchanged: true;
  continuation: RemoteCredentialContinuation;
}

export interface RevokeInstallationTokenResponse {
  status: 'revoked';
  existing_sessions_unchanged: true;
  admitted_operations_continue_to_finite_boundary: true;
  continuation: RemoteCredentialContinuation;
}

export const asAgentPresetId = (value: string): AgentPresetId => value as AgentPresetId;
export const asAgentSessionId = (value: string): AgentSessionId => value as AgentSessionId;
export const asRemoteBindingId = (value: string): RemoteBindingId => value as RemoteBindingId;
export const asResolvedSnapshotId = (value: string): ResolvedSnapshotId =>
  value as ResolvedSnapshotId;
export const asDigestHex = (value: string): DigestHex => value as DigestHex;
export const asCapabilityId = (value: string): CapabilityId => value as CapabilityId;
export const asSkillId = (value: string): SkillId => value as SkillId;
export const asPackageId = (value: string): PackageId => value as PackageId;
