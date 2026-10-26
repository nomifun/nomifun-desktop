/** Manual cloud-office product contracts; never an Agent transcript or checkpoint. */
export type RuntimePhase = 'missing' | 'ready' | 'incompatible' | 'broken' | 'preparing' | 'error';
export type RuntimeSource = 'shared_user' | 'external' | 'selected';
export interface RuntimeInfo {
  phase: RuntimePhase; executable: string | null; version: string | null; source: RuntimeSource | null;
  message: string | null; official_install_command: string;
}
export interface RuntimeUpdateInfo {
  installed_version: string | null;
  latest_version: string | null;
  compatible_version: string;
  update_available: boolean;
  message: string | null;
}
export interface OfficeConnection {
  id: string; name: string; enabled: boolean; profile: string | null; executable: string | null;
  created_at: string; updated_at: string;
}
export interface ConnectionUpdate { name: string; enabled: boolean; profile: string | null }
export interface OfficeActor { app_id: string; user_id: string; name: string }
export interface LogoutRequest { profile: string; expected_actor: OfficeActor | null }
export interface OfficeProfile { name: string; app_id: string; brand: string; user_name: string | null; token_status: string | null }
export interface OfficePermission { capability: string; required_scopes: string[]; granted: boolean }
export type ConnectionPhase = 'disabled' | 'not_configured' | 'needs_login' | 'ready' | 'expired' | 'error';
export interface OfficeStatus {
  provider: string; connection: OfficeConnection; runtime: RuntimeInfo; phase: ConnectionPhase;
  actor: OfficeActor | null; permissions: OfficePermission[];
  host_name: string; host_os: string; message: string | null;
}
export type AuthKind = 'configure_new' | 'configure_existing' | 'login';
export interface AuthStartRequest {
  kind: AuthKind; scopes: string[]; profile: string | null; app_id: string | null; app_secret: string | null;
}
export type AuthPhase = 'starting' | 'waiting_user' | 'waiting_admin' | 'cancelling' | 'complete' | 'partial' | 'cancelled' | 'denied' | 'expired' | 'error';
export interface AuthorizationFlow {
  id: string; kind: AuthKind; phase: AuthPhase; verification_url: string | null; expires_at: string | null;
  missing_scopes: string[]; console_url: string | null; message: string | null;
}
export type TaskRelation = 'assigned' | 'created' | 'tasklist';
export type DocumentFilter = 'all' | 'document' | 'base';
export type OfficeQuery =
  | { kind: 'documents'; query: string; filter: DocumentFilter; cursor: string | null }
  | { kind: 'people'; query: string }
  | { kind: 'tasklists'; cursor: string | null }
  | { kind: 'tasks'; relation: TaskRelation; tasklist_id: string | null; cursor: string | null }
  | { kind: 'agenda'; start: string; end: string }
  | { kind: 'calendar_event'; calendar_id: string; event_id: string }
  | { kind: 'base_records'; base_token: string; table_id: string; offset: number | null; filter: unknown | null };
export interface OfficeItem {
  id: string; kind: string; title: string; url: string | null; description: string | null; modified_at: string | null;
  assignee_ids: string[]; creator_id: string | null; tasklist_ids: string[]; due: string | null; completed: boolean | null;
  start: string | null; end: string | null; calendar_id: string | null; base_token: string | null; table_id: string | null;
  pending_operation_id: string | null; pending_status: ItemStatus | null;
  fields: Record<string, unknown>;
}
export interface OfficeField { id: string; name: string; field_type: string; writable: boolean; options: unknown | null }
export interface OfficeQueryResult {
  items: OfficeItem[]; fields: OfficeField[]; has_more: boolean; next_cursor: string | null; warnings: string[];
}
export type OfficeMutation =
  | { kind: 'task_create'; client_item_id: string; title: string; assignee_id: string; assignee_name: string | null; due: string | null; tasklist_id: string | null }
  | { kind: 'task_complete'; client_item_id: string; task_id: string; completed: boolean }
  | { kind: 'document_create'; client_item_id: string; title: string }
  | { kind: 'document_delete'; client_item_id: string; document_id: string; document_type: string; title: string }
  | { kind: 'calendar_create'; client_item_id: string; title: string; start: string; end: string; timezone: string }
  | { kind: 'base_update'; client_item_id: string; base_token: string; table_id: string; record_id: string; fields: Record<string, unknown>; before: Record<string, unknown> };
export interface ExecuteRequest { request_id: string; expected_actor: OfficeActor; profile: string; items: OfficeMutation[] }
export type ItemStatus = 'queued' | 'running' | 'succeeded' | 'not_executed' | 'unknown' | 'accepted_pending_confirmation';
export type OperationStatus = ItemStatus | 'partial';
export interface OperationAttempt {
  id: string; number: number; client_token: string; mutation: Record<string, unknown>; status: ItemStatus;
  started_at: string; finished_at: string | null; message: string | null; result: unknown | null;
}
export interface OperationObservation {
  id: string; kind: 'original_receipt' | 'target_readback' | 'inconclusive' | 'error'; created_at: string; message: string; data: unknown | null;
}
export interface OperationItem {
  id: string; mutation: Record<string, unknown>; target_key: string; status: ItemStatus; remote_id: string | null; remote_url: string | null;
  message: string | null; attempts: OperationAttempt[]; observations: OperationObservation[];
}
export interface OfficeOperation {
  id: string; request_id: string; connection_id: string; actor: OfficeActor; profile: string; status: OperationStatus;
  items: OperationItem[];
}
