/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/** The one local Plugin identity used by every Unified Plugin Core resource. */
export type PluginId = string;
export type PluginDraftId = string;

// ---------------------------------------------------------------------------
// Manifest projection
// ---------------------------------------------------------------------------

export type PluginServiceMode = 'on_demand' | 'continuous';

export interface PluginEntrypointsSummary {
  ui?: string;
  service?: string;
  service_mode?: PluginServiceMode;
}

export type PluginActionEffect = 'read' | 'write' | 'external';

export interface PluginActionSummary {
  action_id: string;
  stable_id?: string;
  name: string;
  description: string;
  input_schema: Record<string, unknown>;
  output_schema: Record<string, unknown>;
  effect: PluginActionEffect;
}

export type PluginBindingPoint =
  | 'agent.tool'
  | 'agent.context'
  | 'agent.before_model'
  | 'agent.before_tool'
  | 'desktop.command'
  | 'desktop.event'
  | 'automation.action';

export interface PluginBindingSummary {
  point: PluginBindingPoint;
  action_id: string;
  action_stable_id?: string;
  optional: boolean;
  supported: boolean;
  unavailable_reason?: string;
}

export interface PluginDesktopCommand {
  action_id: string;
  name: string;
  description: string;
  input_schema: Record<string, unknown>;
  effect: PluginActionEffect;
}

export interface InvokePluginDesktopCommandRequest {
  action_id: string;
  input?: unknown;
}

export interface DispatchPluginDesktopEventRequest {
  input?: unknown;
}

export interface PluginDesktopEventReport {
  outputs: Array<{ action_id: string; output: unknown }>;
  failures: Array<{ action_id: string; code: string }>;
}

export interface PluginManifestSummary {
  schema: string;
  package_id: string;
  version: string;
  name: string;
  description: string;
  host_api: string;
  entrypoints: PluginEntrypointsSummary;
  actions: PluginActionSummary[];
  bindings: PluginBindingSummary[];
  data_version: number;
  config_schema: Record<string, unknown>;
  secret_slots: string[];
  permissions: string[];
}

// ---------------------------------------------------------------------------
// Installed Plugin state
// ---------------------------------------------------------------------------

export interface PluginArtifactDataPointer {
  artifact_digest: string;
  package_version: string;
  data_generation: string;
  data_version: number;
}

export type PluginObservedState = 'stopped' | 'starting' | 'running' | 'failed';

/** Ephemeral process observation; never a durable lifecycle field. */
export interface PluginServiceObservation {
  state: PluginObservedState;
  generation?: number;
  process_id?: number;
  started_at_ms?: number;
  error_code?: string;
}

export interface PluginSummary {
  plugin_id: PluginId;
  package_id: string;
  display_name: string;
  description: string;
  enabled: boolean;
  trashed_at_ms?: number;
  revision: number;
  active: PluginArtifactDataPointer;
  previous?: PluginArtifactDataPointer;
  has_ui: boolean;
  has_service: boolean;
  service_mode?: PluginServiceMode;
  action_count: number;
  binding_count: number;
  runtime: PluginServiceObservation;
  last_error?: string;
  updated_at_ms: number;
}

export interface PluginConfig {
  schema: Record<string, unknown>;
  values: Record<string, unknown>;
  valid: boolean;
  validation_errors: string[];
}

export type PluginCredentialBindingStatus =
  | 'unbound'
  | 'bound'
  | 'missing'
  | 'invalid';

export interface PluginCredentialBinding {
  slot: string;
  required: boolean;
  status: PluginCredentialBindingStatus;
  /** Host Credential Store identity only; never a credential value. */
  credential_id?: string;
}

export interface PluginGrant {
  permission: string;
  granted: boolean;
  confirmed_at_ms?: number;
}

export interface PluginDetail {
  summary: PluginSummary;
  manifest: PluginManifestSummary;
  config: PluginConfig;
  credential_bindings: PluginCredentialBinding[];
  grants: PluginGrant[];
}

export interface PluginLibraryResponse {
  revision: number;
  plugins: PluginSummary[];
}

export interface PluginLibraryItem {
  plugin_id: PluginId;
  collection_id?: string;
  pinned: boolean;
  custom_name?: string;
  last_opened_at_ms?: number;
}

export interface PluginLibraryState {
  revision: number;
  collections: string[];
  items: PluginLibraryItem[];
}

export interface PluginCredentialReference {
  credential_id: string;
  kind: 'provider' | 'connection';
  label: string;
  enabled: boolean;
}

export interface UpdatePluginLibraryStateRequest {
  expected_revision: number;
  collections: string[];
  items: PluginLibraryItem[];
}

// ---------------------------------------------------------------------------
// Draft authoring and preview
// ---------------------------------------------------------------------------

export type PluginDraftStatus =
  | 'ready'
  | 'generating'
  | 'failed';

export interface PluginDraftSummary {
  draft_id: PluginDraftId;
  revision: number;
  plugin_id?: PluginId;
  base_plugin_revision?: number;
  package_id?: string;
  display_name: string;
  description: string;
  status: PluginDraftStatus;
  error_code?: string;
  updated_at_ms: number;
}

export type PluginDraftMessageRole = 'user' | 'assistant';

export interface PluginDraftMessage {
  role: PluginDraftMessageRole;
  content: string;
}

export interface PluginDraftFile {
  path: string;
  media_type: string;
  digest: string;
  size_bytes: number;
  text?: string;
}

export interface PluginDraftDetail {
  summary: PluginDraftSummary;
  messages: PluginDraftMessage[];
  files: PluginDraftFile[];
}

export interface PluginDraftListResponse {
  drafts: PluginDraftSummary[];
}

export interface CreatePluginDraftRequest {
  plugin_id?: PluginId;
  expected_plugin_revision?: number;
  template?: string;
}

export interface GeneratePluginDraftRequest {
  expected_revision: number;
  provider_id: string;
  model: string;
  requirement: string;
}

export interface CancelPluginDraftGenerationRequest {
  expected_revision: number;
}

export interface ReplacePluginDraftFileRequest {
  expected_revision: number;
  path: string;
  content_base64: string;
}

export interface DeletePluginDraftFileRequest {
  expected_revision: number;
  path: string;
}

export interface PluginPreviewAccessRequest {
  permissions: string[];
  /** Slot -> Host Credential Store identity. */
  credential_bindings: Record<string, string>;
}

export interface PreviewPluginDraftRequest {
  expected_revision: number;
  config: Record<string, unknown>;
  access?: PluginPreviewAccessRequest;
}

export interface SavePluginDraftRequest {
  expected_revision: number;
  expected_plugin_revision?: number;
  /** Opaque Host-issued confirmation bound to the staged bytes. */
  permission_confirmation_id?: string;
  config: Record<string, unknown>;
  credential_bindings: Record<string, string>;
}

export interface DeletePluginDraftRequest {
  expected_revision: number;
}

// ---------------------------------------------------------------------------
// Import, install and permission confirmation
// ---------------------------------------------------------------------------

export type PluginImportKind = 'directory' | 'zip' | 'backup';

export interface InspectPluginImportRequest {
  source_path: string;
  kind: PluginImportKind;
}

export interface InstallPluginImportRequest {
  source_path: string;
  kind: PluginImportKind;
  expected_plugin_revision?: number;
  create_copy?: boolean;
  /** Opaque Host-issued confirmation bound to the freshly staged bytes. */
  permission_confirmation_id?: string;
  config?: Record<string, unknown>;
  credential_bindings?: Record<string, string>;
}

export interface PluginBackupDataSummary {
  includes_data: boolean;
  data_version: number;
  database_size_bytes: number;
  file_count: number;
  includes_config: boolean;
  credential_slots_to_rebind: string[];
}

export interface PluginPermissionExpansion {
  confirmation_id: string;
  added_permissions: string[];
  added_secret_slots: string[];
  trusted_local_service: boolean;
}

export interface PluginImportInspection {
  kind: PluginImportKind;
  /** Computed by the Host; never supplied by the caller. */
  artifact_digest: string;
  manifest: PluginManifestSummary;
  trusted_local_service: boolean;
  target_plugin_id?: PluginId;
  target_plugin_revision?: number;
  backup?: PluginBackupDataSummary;
  permission_expansion?: PluginPermissionExpansion;
}

export type PluginInstallOutcome =
  | { outcome: 'installed'; plugin: PluginDetail }
  | {
      outcome: 'confirmation_required';
      confirmation: PluginPermissionExpansion;
    };

export interface InstallPluginImportResponse {
  result: PluginInstallOutcome;
}

export interface SavePluginDraftResponse {
  draft: PluginDraftSummary;
  result: PluginInstallOutcome;
}

// ---------------------------------------------------------------------------
// Installed Plugin commands
// ---------------------------------------------------------------------------

export interface SetPluginEnabledRequest {
  expected_revision: number;
  enabled: boolean;
}

export interface ConfigurePluginRequest {
  expected_revision: number;
  config: Record<string, unknown>;
  /** Slot -> Host Credential Store identity, or null to unbind. */
  credential_bindings?: Record<string, string | null>;
  grants?: Record<string, boolean>;
}

export type PluginRestoreMode =
  | 'previous_code'
  | 'previous_code_and_data'
  | 'from_trash';

export interface RestorePluginRequest {
  expected_revision: number;
  mode: PluginRestoreMode;
  acknowledge_data_loss?: boolean;
}

export interface TrashPluginRequest {
  expected_revision: number;
}

export interface DeletePluginRequest {
  expected_revision: number;
  acknowledge_permanent_delete: boolean;
}

export interface ExportPluginPackageRequest {
  expected_revision: number;
  destination_path: string;
  include_source?: boolean;
}

export interface ExportPluginBackupRequest {
  expected_revision: number;
  destination_path: string;
}

export interface PluginExportResult {
  destination_path: string;
  digest: string;
  size_bytes: number;
}

// ---------------------------------------------------------------------------
// Surface and preview descriptors
// ---------------------------------------------------------------------------

export interface PluginSurfaceDescriptor {
  plugin_id?: PluginId;
  draft_id?: PluginDraftId;
  artifact_digest: string;
  surface_session_id: string;
  surface_generation: number;
  entrypoint: string;
  is_preview: boolean;
}

export interface PluginDraftPreviewResponse {
  draft_revision: number;
  descriptor: PluginSurfaceDescriptor;
}

export interface OpenPluginSurfaceRequest {
  expected_revision: number;
}

export interface ClosePluginSurfaceRequest {
  surface_session_id: string;
  surface_generation: number;
}

/** Transport command selecting the installed or preview Surface resource. */
export interface ClosePluginSurfaceCommand {
  plugin_id?: PluginId;
  draft_id?: PluginDraftId;
  is_preview: boolean;
  request: ClosePluginSurfaceRequest;
}

// ---------------------------------------------------------------------------
// Unified Surface Bridge
// ---------------------------------------------------------------------------

export type PluginKvRequest =
  | { operation: 'get'; key: string }
  | { operation: 'set'; key: string; value: unknown }
  | { operation: 'delete'; key: string }
  | {
      operation: 'compare_and_swap';
      key: string;
      expected_revision?: number;
      value?: unknown;
    };

export interface PluginDatabaseStatement {
  sql: string;
  parameters?: unknown[];
}

export type PluginDatabaseRequest =
  | { operation: 'query'; sql: string; parameters?: unknown[] }
  | { operation: 'execute'; sql: string; parameters?: unknown[] }
  | { operation: 'batch'; statements: PluginDatabaseStatement[] };

export type PluginFilesRequest =
  | { operation: 'read'; path: string }
  | {
      operation: 'write';
      path: string;
      content_base64: string;
      overwrite?: boolean;
    }
  | { operation: 'list'; path?: string }
  | { operation: 'delete'; path: string };

export type PluginCacheRequest =
  | { operation: 'get'; key: string }
  | { operation: 'set'; key: string; value: unknown; ttl_ms?: number }
  | { operation: 'delete'; key: string };

export type PluginBridgeTarget =
  | { target: 'kv'; request: PluginKvRequest }
  | { target: 'db'; request: PluginDatabaseRequest }
  | { target: 'files'; request: PluginFilesRequest }
  | { target: 'cache'; request: PluginCacheRequest }
  | { target: 'actions'; action: string; input: unknown }
  | { target: 'host'; capability: string; input: unknown }
  | { target: 'config' };

export interface PluginBridgeRequest {
  call_id: string;
  target: PluginBridgeTarget;
}

export interface DispatchPluginBridgeRequest {
  plugin_id?: PluginId;
  draft_id?: PluginDraftId;
  artifact_digest: string;
  surface_session_id: string;
  surface_generation: number;
  is_preview: boolean;
  request: PluginBridgeRequest;
}

export type PluginKvResult =
  | { outcome: 'value'; value?: unknown; revision?: number }
  | { outcome: 'written'; revision: number }
  | { outcome: 'deleted'; existed: boolean }
  | {
      outcome: 'compare_and_swap';
      applied: boolean;
      current_revision?: number;
    };

export type PluginDatabaseResult =
  | { outcome: 'rows'; rows: Array<Record<string, unknown>> }
  | {
      outcome: 'executed';
      rows_affected: number;
      last_insert_rowid?: number;
    }
  | { outcome: 'batch'; results: PluginDatabaseResult[] };

export interface PluginFileEntry {
  path: string;
  is_directory: boolean;
  size_bytes: number;
}

export type PluginFilesResult =
  | { outcome: 'data'; content_base64: string }
  | { outcome: 'written'; size_bytes: number }
  | { outcome: 'entries'; entries: PluginFileEntry[] }
  | { outcome: 'deleted'; existed: boolean };

export type PluginCacheResult =
  | { outcome: 'value'; value?: unknown }
  | { outcome: 'stored' }
  | { outcome: 'deleted'; existed: boolean };

export type PluginBridgeSuccess =
  | { target: 'kv'; result: PluginKvResult }
  | { target: 'db'; result: PluginDatabaseResult }
  | { target: 'files'; result: PluginFilesResult }
  | { target: 'cache'; result: PluginCacheResult }
  | { target: 'actions'; result: unknown }
  | { target: 'host'; result: unknown }
  | { target: 'config'; config: Record<string, unknown> };

export interface PluginBridgeError {
  code: string;
  message: string;
  outcome_unknown: boolean;
}

export type PluginBridgeResult =
  | { outcome: 'success'; call_id: string; result: PluginBridgeSuccess }
  | { outcome: 'failure'; call_id: string; error: PluginBridgeError };
