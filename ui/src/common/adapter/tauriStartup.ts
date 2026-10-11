/** The pre-backend page uses native IPC only; no backend HTTP endpoint exists yet. */
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export type ToolPreparationPhase = 'idle' | 'checking' | 'downloading' | 'extracting' | 'verifying' | 'ready' | 'error';
export interface DesktopStartupSnapshot {
  revision: number;
  stage: 'preparing' | 'failed' | 'starting_backend' | 'ready' | 'quitting';
  preparation: {
    tool: 'node';
    version: string;
    revision: number;
    phase: ToolPreparationPhase;
    received_bytes: number;
    total_bytes: number | null;
    error: { code: string; message: string; retryable: boolean } | null;
  };
}
export interface DesktopStartupIO {
  snapshot: () => Promise<DesktopStartupSnapshot>;
  subscribe: (receive: (snapshot: DesktopStartupSnapshot) => void) => Promise<() => void>;
  retry: () => Promise<DesktopStartupSnapshot>;
  quit: () => Promise<void>;
}
export const desktopStartup: DesktopStartupIO = {
  snapshot: () => invoke('desktop_startup_status'),
  subscribe: async (receive) => listen<DesktopStartupSnapshot>('desktop-startup://state-changed', (event) => receive(event.payload)),
  retry: () => invoke('desktop_startup_retry'),
  quit: () => invoke('desktop_startup_quit'),
};
