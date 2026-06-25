import type { IPlatformServices } from './IPlatformServices';

let _services: IPlatformServices | null = null;

/**
 * Resolve the dev-mode app name for environment isolation.
 * Centralised so that every call-site stays in sync.
 */
export function getDevAppName(): string {
  const isMultiInstance = process.env.NOMIFUN_MULTI_INSTANCE === '1';
  return isMultiInstance ? 'NomiFun-Dev-2' : 'NomiFun-Dev';
}

export function registerPlatformServices(services: IPlatformServices): void {
  _services = services;
}

export function getPlatformServices(): IPlatformServices {
  if (!_services) {
    // Electron auto-registration was removed with the Electron shell. Platform
    // services are Node-only and must be registered explicitly via
    // registerPlatformServices(); the Tauri renderer never reaches this module.
    throw new Error(
      '[Platform] Services not registered. Call registerPlatformServices() before using platform APIs.'
    );
  }
  return _services;
}

export type {
  IPlatformServices,
  IPlatformPaths,
  IWorkerProcess,
  IWorkerProcessFactory,
  IPowerManager,
  INotificationService,
  INetworkService,
} from './IPlatformServices';
