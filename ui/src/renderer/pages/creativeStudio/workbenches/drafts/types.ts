/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { CreativeModelSelectionRef } from '../../models';
import type {
  ImageWorkbenchLayout,
  ImageWorkbenchSettings,
} from '../image';
import type { VideoWorkbenchLayout } from '../video';

export const STANDALONE_WORKBENCH_DRAFT_VERSION = 1 as const;

export type StandaloneWorkbenchDraftKind = 'image' | 'video';

export interface ImageWorkbenchDraftParameters {
  interfaceMode: ImageWorkbenchSettings['interfaceMode'];
  quality: ImageWorkbenchSettings['quality'];
  width: number | null;
  height: number | null;
  aspectRatio: string;
  count: number;
}

export interface VideoWorkbenchDraftParameters {
  resolution: '720p' | '1080p';
  aspect: '16:9' | '9:16' | '1:1';
  duration: '5' | '10';
  taskCount: 1;
}

interface StandaloneWorkbenchSessionDraftBase {
  version: typeof STANDALONE_WORKBENCH_DRAFT_VERSION;
  prompt: string;
  model: CreativeModelSelectionRef | null;
  referenceAssetIds: string[];
}

export interface ImageWorkbenchSessionDraft
  extends StandaloneWorkbenchSessionDraftBase {
  workbenchKind: 'image';
  layout: ImageWorkbenchLayout;
  parameters: ImageWorkbenchDraftParameters;
}

export interface VideoWorkbenchSessionDraft
  extends StandaloneWorkbenchSessionDraftBase {
  workbenchKind: 'video';
  layout: VideoWorkbenchLayout;
  parameters: VideoWorkbenchDraftParameters;
}

export type StandaloneWorkbenchSessionDraft =
  | ImageWorkbenchSessionDraft
  | VideoWorkbenchSessionDraft;

export function createDefaultImageWorkbenchDraft(): ImageWorkbenchSessionDraft {
  return {
    version: STANDALONE_WORKBENCH_DRAFT_VERSION,
    workbenchKind: 'image',
    layout: 'side',
    prompt: '',
    model: null,
    parameters: {
      interfaceMode: 'images',
      quality: 'auto',
      width: 1024,
      height: 1024,
      aspectRatio: '1:1',
      count: 1,
    },
    referenceAssetIds: [],
  };
}

export function imageWorkbenchSettingsFromDraft(
  draft: ImageWorkbenchSessionDraft
): ImageWorkbenchSettings {
  return {
    model: draft.model ? { ...draft.model } : null,
    ...draft.parameters,
  };
}

export function createImageWorkbenchDraft(input: {
  layout: ImageWorkbenchLayout;
  prompt: string;
  settings: ImageWorkbenchSettings;
  referenceAssetIds: readonly string[];
}): ImageWorkbenchSessionDraft {
  return {
    version: STANDALONE_WORKBENCH_DRAFT_VERSION,
    workbenchKind: 'image',
    layout: input.layout,
    prompt: input.prompt,
    model: input.settings.model
      ? {
          providerId:
            input.settings.model.providerId as CreativeModelSelectionRef['providerId'],
          model: input.settings.model.model,
        }
      : null,
    parameters: {
      interfaceMode: input.settings.interfaceMode,
      quality: input.settings.quality,
      width: input.settings.width,
      height: input.settings.height,
      aspectRatio: input.settings.aspectRatio,
      count: input.settings.count,
    },
    referenceAssetIds: [...input.referenceAssetIds],
  };
}

export function createDefaultVideoWorkbenchDraft(): VideoWorkbenchSessionDraft {
  return {
    version: STANDALONE_WORKBENCH_DRAFT_VERSION,
    workbenchKind: 'video',
    layout: 'side',
    prompt: '',
    model: null,
    parameters: {
      resolution: '1080p',
      aspect: '16:9',
      duration: '5',
      taskCount: 1,
    },
    referenceAssetIds: [],
  };
}

export function createVideoWorkbenchDraft(input: {
  layout: VideoWorkbenchLayout;
  prompt: string;
  model: { providerId: string; model: string } | null;
  resolution: string;
  aspect: string;
  duration: string;
  taskCount: number;
  referenceAssetIds: readonly string[];
}): VideoWorkbenchSessionDraft {
  return {
    version: STANDALONE_WORKBENCH_DRAFT_VERSION,
    workbenchKind: 'video',
    layout: input.layout,
    prompt: input.prompt,
    model: input.model
      ? {
          providerId: input.model.providerId as CreativeModelSelectionRef['providerId'],
          model: input.model.model,
        }
      : null,
    parameters: {
      resolution: input.resolution as VideoWorkbenchDraftParameters['resolution'],
      aspect: input.aspect as VideoWorkbenchDraftParameters['aspect'],
      duration: input.duration as VideoWorkbenchDraftParameters['duration'],
      taskCount: input.taskCount as VideoWorkbenchDraftParameters['taskCount'],
    },
    referenceAssetIds: [...input.referenceAssetIds],
  };
}
