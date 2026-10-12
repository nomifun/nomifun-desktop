/** The pre-backend page uses native IPC only; no backend HTTP endpoint exists yet. */
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export interface DesktopStartupSnapshot {
  revision: number;
  stage: 'starting_backend' | 'ready' | 'quitting';
}
export interface DesktopStartupIO {
  snapshot: () => Promise<DesktopStartupSnapshot>;
  subscribe: (receive: (snapshot: DesktopStartupSnapshot) => void) => Promise<() => void>;
  quit: () => Promise<void>;
}
export const desktopStartup: DesktopStartupIO = {
  snapshot: () => invoke('desktop_startup_status'),
  subscribe: async (receive) => listen<DesktopStartupSnapshot>('desktop-startup://state-changed', (event) => receive(event.payload)),
  quit: () => invoke('desktop_startup_quit'),
};
