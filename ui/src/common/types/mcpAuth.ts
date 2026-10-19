/** Server-owned authentication state; independent of MCP connection probes. */
export type McpAuthState = 'anonymous' | 'required' | 'authorized' | 'reauthorization_required' | 'configuration_required' | 'failed';

export interface McpAuthDiagnostic {
  code: string;
  stage: 'discovery' | 'client_registration' | 'authorization' | 'callback' | 'token_exchange' | 'refresh' | 'storage' | 'connection';
  retryable: boolean;
  user_action: 'none' | 'retry' | 'configure_client' | 'reauthorize' | 'grant_scope';
  trace_id: string;
  message: string;
  details?: unknown;
}

export interface McpAuthStatus {
  state: McpAuthState;
  expires_at?: number;
  granted_scopes: string[];
  diagnostic?: McpAuthDiagnostic;
}

export interface McpOAuthAttempt {
  attempt_id: string;
  status: 'waiting_for_user' | 'exchanging' | 'authorized' | 'failed' | 'cancelled' | 'expired';
  authorization_url?: string;
  diagnostic?: McpAuthDiagnostic;
}

/** Write-only login input; never persist this object in server JSON or UI storage. */
export type McpOAuthClientConfig =
  | { mode: 'auto' }
  | { mode: 'cimd'; client_id: string }
  | { mode: 'pre_registered'; client_id: string; client_secret?: string; token_endpoint_auth_method?: string; redirect_uri?: string };

export interface McpOAuthStartRequest {
  client?: McpOAuthClientConfig;
}
