/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export {
  hydrateStandaloneWorkbenchDraftReferences,
  isExactWorkbenchDraftModelAvailable,
  type HydratedStandaloneWorkbenchDraftReferences,
  type StandaloneWorkbenchDraftAssetReader,
} from './hydration';
export {
  clearStandaloneWorkbenchDraft,
  parseStandaloneWorkbenchDraft,
  readStandaloneWorkbenchDraft,
  standaloneWorkbenchDraftStorageKey,
  writeStandaloneWorkbenchDraft,
  STANDALONE_WORKBENCH_DRAFT_KEY_PREFIX,
  STANDALONE_WORKBENCH_DRAFT_MAX_PROMPT_LENGTH,
  STANDALONE_WORKBENCH_DRAFT_MAX_SERIALIZED_LENGTH,
  type StandaloneWorkbenchDraftStorage,
} from './storage';
export {
  createDefaultImageWorkbenchDraft,
  createDefaultVideoWorkbenchDraft,
  createImageWorkbenchDraft,
  createVideoWorkbenchDraft,
  imageWorkbenchSettingsFromDraft,
  STANDALONE_WORKBENCH_DRAFT_VERSION,
  type ImageWorkbenchDraftParameters,
  type ImageWorkbenchSessionDraft,
  type StandaloneWorkbenchDraftKind,
  type StandaloneWorkbenchSessionDraft,
  type VideoWorkbenchDraftParameters,
  type VideoWorkbenchSessionDraft,
} from './types';
