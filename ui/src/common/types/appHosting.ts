import type { TChatConversation } from '../config/storage';
import type { SessionReasoningEffort } from './reasoningEffort';

/** Current command hosting contract. Connection credentials never enter this DTO. */
export type AppImportKind = 'source' | 'artifact';
export type AppOperation = 'preview' | 'use' | 'start' | 'stop' | 'rebuild';
export interface AppSettings {
  project_directory: string;
  install_command?: string | null;
  preview_command?: string | null;
  build_command?: string | null;
  output_directory?: string | null;
  start_command?: string | null;
  web_url?: string | null;
  node_path?: string | null;
  environment: Record<string, string>;
  mcp_url?: string | null;
  agent_id?: string | null;
}

/** Only edited fields are submitted; omitted fields keep their current values. */
export interface AppUpdateRequest {
  id: string;
  name?: string;
  description?: string;
  tags?: string[];
  import_kind?: AppImportKind;
  settings?: Partial<AppSettings>;
  detect_settings?: boolean;
  /** turn_id is the turn.completed notification's accepted source message UUID. */
  completion?: { session_id: string; turn_id: string };
}
export interface HostedApp {
  id: string;
  name: string;
  description: string;
  tags: string[];
  development_session_id: string | null;
  directory: string;
  import_kind: AppImportKind;
  settings: AppSettings;
  state: 'running' | 'stopped' | 'failed';
  operation: string | null;
  last_error: string | null;
  output: string;
  pid: number | null;
  preview: boolean;
  prepared: boolean;
}
export interface AppDevelopmentSessionRequest {
  id: string;
  session_id?: string;
  new_session?: boolean;
  creation_key?: string;
  model?: { provider_id: string; model: string };
  reasoning_effort?: SessionReasoningEffort;
  refresh_workflow?: boolean;
}
export interface AppProjectSession {
  conversation: TChatConversation;
  editable_in_app: boolean;
  archived: boolean;
}
/** A Host query over application-owned canonical Sessions, not another history store. */
export interface AppProjectSessionPage {
  items: AppProjectSession[];
  total: number;
  /** All owned Sessions, including archived Sessions and pending deletion cleanup. */
  owned_session_count: number;
  has_more: boolean;
  next_cursor: string | null;
  project_directory: string;
  current_session_id: string | null;
}
/** A one-response observation, never persisted with the hosted application. */
export interface AppUpdateResponse extends HostedApp {
  readonly completion_observation?: {
    readonly session_id: string;
    readonly turn_id: string;
    readonly files_changed: boolean;
  };
}
export interface ExecutableReport {
  path: string | null;
  version: string | null;
  error: string | null;
}
export interface AppEnvironmentReport {
  node: ExecutableReport;
  npm: ExecutableReport;
  bun: ExecutableReport;
  node_download_url: string;
}

export interface AppMcpSource {
  app_id: string;
  name: string;
  url: string;
  state: HostedApp['state'];
  connection_status: 'stopped' | 'untested' | 'connected' | 'error';
  tools: Array<{ name: string; description?: string; input_schema?: unknown }>;
  reason: string | null;
}
