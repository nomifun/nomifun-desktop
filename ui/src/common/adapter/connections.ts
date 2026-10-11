/** Mobile credentials belong to one Desktop installation, independently of
 * browser sessions, MCP tokens, and companion channel bindings. */
import { httpDelete, httpGet, httpPost } from './httpBridge';

export interface ConnectionInfo {
  desktop_id: string;
  desktop_name: string;
  protocol_version: number;
  min_client_protocol: number;
  capabilities: string[];
}

export interface ConnectionDevice {
  device_id: string;
  device_name: string;
  created_at_ms: number;
  last_seen_at_ms: number;
  revoked_at_ms: number | null;
}

export interface ConnectionPairing {
  ticket_id: string;
  qr_token: string;
  expires_at_ms: number;
}

export interface ConnectionPairingStatus {
  ticket_id: string;
  state: 'ready' | 'redeeming' | 'used' | 'failed' | 'expired';
  device_id: string | null;
  failure_code?: string | null;
  expires_at_ms: number;
}

export const connections = {
  info: httpGet<ConnectionInfo>('/api/connections/info'),
  pairing: httpPost<ConnectionPairing, Record<string, never>>('/api/connections/pairing'),
  pairingStatus: httpGet<ConnectionPairingStatus, string>(
    (ticketId) => `/api/connections/pairing/${encodeURIComponent(ticketId)}`
  ),
  devices: httpGet<ConnectionDevice[]>('/api/connections/devices'),
  revoke: httpDelete<ConnectionDevice, string>(
    (deviceId) => `/api/connections/devices/${encodeURIComponent(deviceId)}`
  ),
};

export function buildMobilePairingUrl(baseUrl: string, token: string): string {
  const endpoint = new URL('/device-pair', `${baseUrl.replace(/\/$/, '')}/`);
  endpoint.searchParams.set('token', token);
  return `nomifun://pair?v=1&url=${encodeURIComponent(endpoint.toString())}`;
}
