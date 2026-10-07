import { pluginPlatform } from '@/common/adapter/pluginPlatformBridge';
import { conversation } from '@/common/adapter/ipcBridge';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import type { PluginLibraryState } from '@/common/types/pluginPlatform';
import { isDesktopShell } from '@/renderer/utils/platform';

export const PLUGIN_LIBRARY_CHANGED = 'nomifun:plugin-library-changed';

export function notifyPluginLibraryChanged(): void {
  window.dispatchEvent(new Event(PLUGIN_LIBRARY_CHANGED));
}

/** GUI mutations, conversation tools and reconnects share the same library refresh. */
export function subscribePluginLibraryChanges(refresh: () => void): () => void {
  window.addEventListener(PLUGIN_LIBRARY_CHANGED, refresh);
  const offChanges = pluginPlatform.plugins.changed.on(refresh);
  const offReconnect = conversation.reconnected.on(refresh);
  return () => {
    window.removeEventListener(PLUGIN_LIBRARY_CHANGED, refresh);
    offChanges();
    offReconnect();
  };
}

let mutationQueue: Promise<unknown> = Promise.resolve();

export function updatePluginLibraryState(
  update: (state: PluginLibraryState) => void,
): Promise<PluginLibraryState> {
  if (!isDesktopShell()) {
    return Promise.reject(new Error('Plugin library organization is read-only in WebUI'));
  }
  const operation = mutationQueue.catch(() => undefined).then(async () => {
    for (let attempt = 0; attempt < 3; attempt += 1) {
      const current = await pluginPlatform.libraryState.get.invoke();
      const next: PluginLibraryState = {
        revision: current.revision,
        collections: [...current.collections],
        items: current.items.map((item) => ({ ...item })),
      };
      update(next);
      try {
        const saved = await pluginPlatform.libraryState.update.invoke({
          expected_revision: next.revision,
          collections: next.collections,
          items: next.items,
        });
        notifyPluginLibraryChanged();
        return saved;
      } catch (error) {
        if (!isBackendHttpError(error) || error.status !== 409 || attempt === 2) throw error;
      }
    }
    throw new Error('Plugin library state could not be saved');
  });
  mutationQueue = operation;
  return operation;
}
