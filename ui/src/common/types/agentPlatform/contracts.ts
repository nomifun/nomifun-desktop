import type {
  AgentId,
  AgentPresetId,
  AgentSessionId,
  KnowledgeBaseId,
  MessageId,
  ProviderId,
  RemoteBindingId,
  ResolvedSnapshotId,
} from '@/common/types/ids';
import type { IIdmmConfig } from '@/common/types/idmm';

export type {
  AgentPresetId,
  AgentSessionId,
  RemoteBindingId,
  ResolvedSnapshotId,
} from '@/common/types/ids';

declare const digestBrand: unique symbol;
declare const catalogIdBrand: unique symbol;

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
  'customer-service.default',
  'creative-studio.default',
] as const;

export type OfficialPresetKey = (typeof OFFICIAL_PRESET_KEYS)[number];
export type AgentPresetSource = 'official' | 'user';
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

export interface CapabilityRef {
  id: CapabilityId;
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

/**
 * Consumer-neutral AgentSession configuration returned with a Conversation,
 * Cron job, or execution projection. Preset/Snapshot identity is immutable;
 * explicitly mutable product resource subsets advance `binding_version`.
 *
 * The renderer only uses this for historical identity presentation. It never
 * resolves a live AgentPreset from this object and never treats it as an
 * editable catalog record.
 */
export interface AgentResolvedSnapshot {
  /** Frozen provenance, re-admitted by the host; not a client permission grant. */
  canonical_binding?: AgentBindingValue;
  preset_id: AgentPresetId;
  preset_revision: number;
  preset_name: string;
  routing_description?: string;
  instructions: string;
  resolved_agent_id?: AgentId;
  resolved_agent_type?: string;
  resolved_agent_backend?: string;
  /** Exact Chat model in this versioned Session binding; Conversation.model mirrors it. */
  resolved_model?: {
    provider_id: ProviderId;
    model: string;
  };
  included_skills: string[];
  excluded_auto_skills: string[];
  enabled_capabilities: string[];
  enabled_capability_actions: Record<string, string[]>;
  required_resource_kinds: string[];
  knowledge_policy: {
    enabled: boolean;
    writeback: boolean;
    eagerness?: 'manual' | 'auto';
    grounded: boolean;
  };
  warnings: string[];
}

export interface CapabilitySelection {
  capability: CapabilityRef;
  action_allowlist?: string[];
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

export interface InstallationRoleBinding {
  selection: RoleProviderSelection;
  binding_version: number;
  updated_at_ms: number;
}

export interface PutAgentRoleDefaultRequest {
  selection: RoleProviderSelection;
  expected_binding_version: number;
}

export interface AgentPresetDocument {
  /** Context order within each phase; omitted contributors follow canonical ID order. */
  context_order?: CapabilityId[];
  /** Request middleware composition order; omitted contributors follow ID order. */
  middleware_order?: CapabilityId[];
  schema_version: string;
  model_route_refs: Record<string, string>;
  chat_route_records: Partial<Record<typeof AGENT_CHAT_MODEL_TASK, ChatRouteRecord>>;
  enabled_capabilities: CapabilitySelection[];
  skill_bindings: ExactCatalogRef<'skill'>[];
  system_role_provider_overrides: Record<string, RoleProviderSelection>;
  persona: string;
  instructions: string;
  starter_prompts: string[];
  /** Runtime behavior defaults; never an Agent capability or permission grant. */
  runtime_policy: {
    idmm: IIdmmConfig;
  };
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

export interface OfficialPresetSeed {
  enabled_capabilities: CapabilitySelection[];
  skill_bindings: ExactCatalogRef<'skill'>[];
  required_resource_kinds: string[];
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
    data_generation: number;
    legacy_data_imported: false;
    official_template_count: number;
    user_preset_count: number;
  };
}

export interface CapabilityCatalogItem {
  capability: CapabilityRef;
  kind: string;
  middleware_phase?: 'before_model' | 'before_tool';
  display_name: string;
  description: string;
  source_package: ExactCatalogRef<'package'>;
  source_kind: string;
  materialization_state: CatalogMaterializationState;
  unavailable_code?: string;
  supported_surfaces: string[];
  required_runtime_features: string[];
  required_resource_kinds: string[];
  required_capabilities: CapabilityRef[];
  conflicting_capabilities: CapabilityRef[];
  action_count: number;
  context_contributor_count: number;
}

export type CapabilityModuleAuthoringPolicy =
  | 'direct'
  | 'dependency_only'
  | 'platform_managed'
  | 'internal';

export interface CapabilityModuleAction {
  action_id: string;
  input_schema: string;
  output_schema: string;
  effect_class: string;
  presentation: string;
}

export interface CapabilityModuleCatalogItem {
  module: CapabilityRef;
  display_name: string;
  description: string;
  source_package: ExactCatalogRef<'package'>;
  authoring_policy: CapabilityModuleAuthoringPolicy;
  summary_kind: string;
  actions: CapabilityModuleAction[];
  context_schema_refs: string[];
  event_schema_refs: string[];
  required_resource_kinds: string[];
  required_host_ports: ExactCatalogRef<'host_port'>[];
  required_modules: CapabilityRef[];
  conflicting_modules: CapabilityRef[];
  supported_surfaces: string[];
}

export interface SkillCatalogItem {
  skill: ExactCatalogRef<'skill'>;
  display_name: string;
  description: string;
  source_package: ExactCatalogRef<'package'>;
  source_kind: string;
  required_capabilities: CapabilityRef[];
  supported_surfaces: string[];
}

export interface McpToolCatalogItem {
  server_id: string;
  canonical_tool_key: string;
  capability: CapabilityRef;
  source_package: ExactCatalogRef<'package'>;
  schema_digest: DigestHex;
  materialization_version: string;
}

export interface RoleProviderCatalogItem {
  selection: RoleProviderSelection;
  display_name: string;
  description: string;
  source_package: ExactCatalogRef<'package'>;
  source_kind: string;
  supported_capabilities: CapabilityRef[];
}

/** Candidates only; the canonical compiler checks compatibility and authorization. */
export interface RoleCatalogItem {
  role: ExactRoleContractRef;
  capabilities: CapabilityRef[];
  providers: RoleProviderCatalogItem[];
}

export interface AgentCatalogResponse {
  modules: CapabilityModuleCatalogItem[];
  capabilities: CapabilityCatalogItem[];
  skills: SkillCatalogItem[];
  mcp_tools: McpToolCatalogItem[];
  roles: RoleCatalogItem[];
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
  document?: AgentPresetDocument;
}

export interface CreateAgentPresetFromTemplateRequest {
  model?: { provider_id: string; model: string };
  /** Required intent: true prepares an internal launch configuration; false creates a personal Agent. */
  reuse_existing: boolean;
  display_name: string;
  description?: string;
  model_route_refs: Record<string, string>;
  chat_route_records: Partial<Record<typeof AGENT_CHAT_MODEL_TASK, ChatRouteRecord>>;
}

export interface SaveAgentPresetRevisionRequest {
  expected_current_revision?: PresetRevisionRef;
  draft: AgentPresetDraft;
  reason?: string;
}

export interface SaveAgentPresetRevisionResponse {
  preset: AgentPresetSummary;
  revision: AgentPresetRevision;
  resolved_snapshot_ref: ResolvedSnapshotRef;
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

export interface SelectProductAgentBindingRequest {
  selection: ProductAgentSelection;
  model?: { provider_id: string; model: string };
  resource_selections?: AgentResourceSelection[];
  conversation_id?: string;
}

export type ProductAgentSelection =
  | { kind: 'template'; template_key: OfficialPresetKey }
  | { kind: 'preset'; preset_id: string };

export type ProductAgentUnavailableReason = 'web_search' | 'vision' | 'model' | 'capability' | 'removed';

export interface ProductAgentOptions {
  selection: ProductAgentSelection;
  needs_model: boolean;
  options: Array<{
    selection: ProductAgentSelection;
    display_name: string;
    available: boolean;
    reason: ProductAgentUnavailableReason | null;
  }>;
}

export interface ProductAgentSelectionResult {
  selection: ProductAgentSelection;
  needs_model: boolean;
  agent_binding?: AgentBindingValue;
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

/** Runtime identities are extension-owned strings, not a built-in engine enum. */
export interface RuntimeBuildDescriptor {
  family_id: string;
  build_id: string;
  build_digest: string;
  display_name: string;
  host_contract_version: number;
  supported_profiles: string[];
}

export interface RuntimeBuildBinding {
  family_id: string;
  build_id: string;
  build_digest: string;
  host_contract_version: number;
  profile: string;
}

export interface CreateAgentSessionRequest {
  model?: { provider_id: string; model: string };
  preset_id: AgentPresetId;
  title?: string;
  reasoning_effort?: import('../reasoningEffort').SessionReasoningEffort;
  resource_selections?: AgentResourceSelection[];
  /** Session-scoped disposition for selected Knowledge resources. This can
   * narrow behavior only; the backend still derives Actions and write access. */
  knowledge_policy?: {
    writeback: boolean;
    writeback_eagerness: 'manual' | 'auto';
  };
  /** User-selected host directory candidate. The backend validates and freezes
   * the canonical workspace resource; this field is never authority by itself. */
  workspace?: string;
}

export interface UpdateAgentSessionReasoningResponse {
  reasoning_effort?: import('../reasoningEffort').SessionReasoningEffort;
}

/** Live Knowledge selection owned by one AgentSession. Agent capabilities are
 * still immutable; this value can only narrow/select resources within them. */
export interface AgentSessionKnowledgeBinding {
  enabled: boolean;
  writeback: boolean;
  writeback_eagerness: 'manual' | 'auto';
  kb_ids: KnowledgeBaseId[];
}

export interface AgentResourceSelection {
  resource_kind: string;
  resource_id: string;
}

export type AgentSwitchSelection =
  | { kind: 'preset'; preset_id: AgentPresetId }
  | { kind: 'template'; template_key: OfficialPresetKey };

export interface PreviewAgentSessionSwitchRequest {
  selection: AgentSwitchSelection;
  model?: { provider_id: ProviderId; model: string };
}

export interface AgentSwitchIdentity {
  label: string;
  preset_id: AgentPresetId;
  preset_revision: number;
  resolved_snapshot_ref: ResolvedSnapshotRef;
  binding_version: number;
}

export interface AgentSwitchBlocker {
  code: string;
  message: string;
  details?: unknown;
}

export interface AgentHandoffAvailability {
  available: boolean;
  requirement_count: number;
  verified_artifact_count: number;
  unresolved_item_count: number;
  completion_gate_inherited: false;
}

export interface PreviewAgentSessionSwitchResponse {
  current: AgentSwitchIdentity;
  target: AgentSwitchIdentity;
  model: {
    provider_id: ProviderId;
    model: string;
    preserved: boolean;
    compatible: boolean;
    missing_features: string[];
  };
  resources: {
    retained: AgentResourceSelection[];
    dropped: AgentResourceSelection[];
    missing_kinds: string[];
  };
  capabilities: {
    gained: string[];
    lost: string[];
  };
  handoff: AgentHandoffAvailability;
  blockers: AgentSwitchBlocker[];
  expected_binding_version: number;
  can_apply: boolean;
}

export type AgentHandoffMode = 'continue_task' | 'context_only';

export interface ApplyAgentSessionSwitchRequest {
  selection: AgentSwitchSelection;
  handoff_mode: AgentHandoffMode;
  expected_binding_version: number;
  model?: { provider_id: ProviderId; model: string };
}

export interface ApplyAgentSessionSwitchResponse<TConversation = unknown> {
  conversation: TConversation;
  transition_id: string;
  previous_agent_label: string;
  current_agent_label: string;
  binding_version: number;
  effective_from: 'next_turn';
  handoff: AgentHandoffAvailability;
  warnings: string[];
}

export interface CreateAgentSessionResponse {
  runtime_build_binding?: RuntimeBuildBinding;
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
  message_id: MessageId;
  cursor: SessionCursor;
  status: string;
  replayed: boolean;
  completed: boolean;
  result_ok?: boolean;
  result_text?: string;
  result_error?: string;
  result_error_code?: string;
  result_error_retryable?: boolean;
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
