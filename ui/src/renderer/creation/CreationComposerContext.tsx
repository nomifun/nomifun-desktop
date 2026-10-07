import { createContext, useContext } from 'react';
import type { AgentPresetId } from '@/common/types/agentPlatform';
import type { CreationMode } from './types';
import type { CreationDraftController } from './useCreationDraft';

export interface CreationComposerValue extends CreationDraftController {
  presetId?: AgentPresetId;
  resolvePreset?(): Promise<AgentPresetId>;
  selectMode(mode: CreationMode): void;
  exit(): void;
  preparing?: boolean;
}
export const CreationComposerContext = createContext<CreationComposerValue | null>(null);
export const useCreationComposer = () => useContext(CreationComposerContext);
