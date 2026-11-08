import type { EventEmitter } from 'node:events';

interface DevSocketLogger {
  info(message: string, options?: { timestamp?: boolean }): void;
  error(message: string, options?: { timestamp?: boolean; error?: Error }): void;
}

export function guardDevUpgradeSockets(server: EventEmitter | null | undefined, logger: DevSocketLogger): () => void;
