import type { AppSettings } from '@/common/types/appHosting';
import type { TChatConversation } from '@/common/config/storage';
import { isBackendHttpError } from '@/common/adapter/httpBridge';

/** Ephemeral preview notification for the process observed at Turn start. */
export interface AppPreviewFileChange {
  appId: string;
  sessionId: string;
  turnId: string;
  pid: number | null;
  url: string | null | undefined;
}

/** Paused and unknown active ownership also keeps shared project files fenced. */
export function isAppProjectSessionBusy(conversation: Pick<TChatConversation, 'status' | 'runtime'>): boolean {
  return conversation.status === 'running' || conversation.runtime?.is_processing === true || Boolean(conversation.runtime?.active_turn_id);
}

export function appErrorMessage(error: unknown): string {
  if (isBackendHttpError(error)) return error.backendMessage;
  return error instanceof Error ? error.message : String(error);
}

export function isLoopbackHostname(value: string): boolean {
  const host = value.toLowerCase().replace(/^\[|\]$/g, '').replace(/\.$/, '');
  return host === 'localhost' || host.endsWith('.localhost') || host === '::1' || /^127\.\d+\.\d+\.\d+$/.test(host);
}

/** A remote browser must never mistake the Host's loopback for its own computer. */
export function canOpenAppUrl(value: string | null | undefined, desktopShell: boolean, clientHostname: string): boolean {
  if (!value) return false;
  try {
    const url = new URL(value);
    if (!['http:', 'https:'].includes(url.protocol)) return false;
    return !isLoopbackHostname(url.hostname) || desktopShell || isLoopbackHostname(clientHostname);
  } catch { return false; }
}

/** An app frame must never share the renderer origin or receive its connection authority. */
export function canEmbedAppUrl(value: string | null | undefined, desktopShell: boolean, rendererUrl: string): boolean {
  try {
    const renderer = new URL(rendererUrl);
    if (!canOpenAppUrl(value, desktopShell, renderer.hostname)) return false;
    const app = new URL(value!);
    return app.origin !== renderer.origin && !app.username && !app.password;
  } catch { return false; }
}

export function parseAppEnvironment(text: string): Record<string, string> {
  const environment: Record<string, string> = {};
  for (const line of text.split(/\r?\n/)) {
    if (!line.trim()) continue;
    const separator = line.indexOf('=');
    const key = line.slice(0, separator).trim();
    if (separator < 1 || !/^[A-Za-z_][A-Za-z0-9_]*$/.test(key)) throw new Error(`Invalid environment variable: ${line}`);
    if (key.startsWith('NOMIFUN_')) throw new Error('NOMIFUN_ environment variables are prepared by the Host');
    environment[key] = line.slice(separator + 1);
  }
  return environment;
}

export function editableAppSettings(settings: AppSettings): AppSettings {
  return {
    project_directory: settings.project_directory,
    install_command: settings.install_command,
    preview_command: settings.preview_command,
    build_command: settings.build_command,
    output_directory: settings.output_directory,
    start_command: settings.start_command,
    web_url: settings.web_url,
    node_path: settings.node_path,
    environment: { ...settings.environment },
    mcp_url: settings.mcp_url,
    agent_id: settings.agent_id,
  };
}
