import { ipcBridge } from '@/common';
import { useCallback, useEffect, useRef } from 'react';
import type { ComposerCommandExecution } from '@/renderer/components/chat/commands/types';

interface UseOpenFileSelectorOptions {
  onFilesSelected: (files: string[]) => void;
  targetKey: string;
}

interface UseOpenFileSelectorResult {
  openFileSelector: (execution?: ComposerCommandExecution) => Promise<void>;
}

/**
 * Shared open-file selector behavior for send boxes.
 * Unifies '+' button and '/open' builtin command handling.
 *
 * In the desktop shell: opens the native file dialog.
 * In WebUI: triggers DirectorySelectionModal via bridge events.
 */
export function useOpenFileSelector(options: UseOpenFileSelectorOptions): UseOpenFileSelectorResult {
  const { onFilesSelected, targetKey } = options;
  const current = useRef({ targetKey, generation: 0, mounted: true });
  if (current.current.targetKey !== targetKey) {
    current.current = { ...current.current, targetKey, generation: current.current.generation + 1 };
  }
  useEffect(() => { current.current.mounted = true; return () => { current.current.mounted = false; }; }, []);

  const openFileSelector = useCallback(async (execution?: ComposerCommandExecution) => {
    const owner = { ...current.current };
    const isCurrent = () => current.current.mounted && owner.generation === current.current.generation
      && (!execution || execution.isCurrent());
    try {
      const files = await ipcBridge.dialog.showOpen.invoke({ properties: ['openFile', 'multiSelections'] });
      if (isCurrent() && files?.length) onFilesSelected(files);
    } catch (error) { if (isCurrent()) throw error; }
  }, [onFilesSelected]);
  return { openFileSelector };
}
