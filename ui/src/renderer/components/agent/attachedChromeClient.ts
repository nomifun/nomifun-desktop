import { httpRequest } from '@/common/adapter/httpBridge';

export type AttachedChromeSnapshot = {
  incarnation: string;
  state: 'connected' | 'connection_lost' | 'disconnecting' | 'cleanup_failed';
  chromium_major: number;
};

export interface AttachedChromeClient {
  snapshot(): Promise<AttachedChromeSnapshot | null>;
  connect(): Promise<AttachedChromeSnapshot>;
  disconnect(incarnation: string): Promise<void>;
}

const path = '/api/browser-providers/attached-chrome';
export const attachedChromeClient: AttachedChromeClient = {
  snapshot: () => httpRequest('GET', path),
  connect: () => httpRequest('POST', path, {}),
  disconnect: (incarnation) => httpRequest('DELETE', path, { incarnation }),
};
