/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { parseAssetId, parseProviderId } from '@/common/types/ids';

import {
  STANDALONE_WORKBENCH_DRAFT_VERSION,
  type ImageWorkbenchSessionDraft,
  type StandaloneWorkbenchDraftKind,
  type StandaloneWorkbenchSessionDraft,
  type VideoWorkbenchSessionDraft,
} from './types';

export const STANDALONE_WORKBENCH_DRAFT_KEY_PREFIX =
  'nomifun:creative-studio:standalone-workbench-draft';
export const STANDALONE_WORKBENCH_DRAFT_MAX_SERIALIZED_LENGTH = 128 * 1024;
export const STANDALONE_WORKBENCH_DRAFT_MAX_PROMPT_LENGTH = 20_000;

const MAX_MODEL_LENGTH = 512;
const MAX_ASPECT_RATIO_LENGTH = 32;
const MAX_IMAGE_REFERENCE_ASSETS = 100;

export type StandaloneWorkbenchDraftStorage = Pick<
  Storage,
  'getItem' | 'setItem' | 'removeItem'
>;

const browserSessionStorage = (): StandaloneWorkbenchDraftStorage | null => {
  if (typeof window === 'undefined') return null;
  try {
    return window.sessionStorage;
  } catch {
    return null;
  }
};

export function standaloneWorkbenchDraftStorageKey(
  workbenchKind: StandaloneWorkbenchDraftKind
): string {
  return `${STANDALONE_WORKBENCH_DRAFT_KEY_PREFIX}:${workbenchKind}`;
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === 'object' && !Array.isArray(value);

const hasExactKeys = (
  value: Record<string, unknown>,
  expected: readonly string[]
): boolean => {
  const keys = Object.keys(value);
  return keys.length === expected.length && expected.every((key) => keys.includes(key));
};

const boundedString = (
  value: unknown,
  maximumLength: number,
  options: { nonempty?: boolean; normalized?: boolean } = {}
): string | null => {
  if (typeof value !== 'string' || value.length > maximumLength) return null;
  if (options.nonempty && value.length === 0) return null;
  if (options.normalized && value.trim() !== value) return null;
  return value;
};

const safeInteger = (
  value: unknown,
  minimum: number,
  maximum: number
): number | null =>
  Number.isSafeInteger(value) && (value as number) >= minimum && (value as number) <= maximum
    ? (value as number)
    : null;

const nullableDimension = (value: unknown): number | null | undefined => {
  if (value === null) return null;
  return safeInteger(value, 1, 8192) ?? undefined;
};

const parseModel = (
  value: unknown
): StandaloneWorkbenchSessionDraft['model'] | undefined => {
  if (value === null) return null;
  if (!isRecord(value) || !hasExactKeys(value, ['providerId', 'model'])) return undefined;
  const model = boundedString(value.model, MAX_MODEL_LENGTH, {
    nonempty: true,
    normalized: true,
  });
  if (model === null) return undefined;
  try {
    return {
      providerId: parseProviderId(value.providerId),
      model,
    };
  } catch {
    return undefined;
  }
};

const parseReferenceAssetIds = (
  value: unknown,
  maximum: number
): string[] | null => {
  if (!Array.isArray(value) || value.length > maximum) return null;
  const parsed: string[] = [];
  try {
    for (const assetId of value) parsed.push(parseAssetId(assetId));
  } catch {
    return null;
  }
  return new Set(parsed).size === parsed.length ? parsed : null;
};

const parseBase = (
  value: unknown,
  expectedKind: StandaloneWorkbenchDraftKind,
  maximumReferences: number
): {
  record: Record<string, unknown>;
  prompt: string;
  model: StandaloneWorkbenchSessionDraft['model'];
  referenceAssetIds: string[];
} | null => {
  if (
    !isRecord(value) ||
    !hasExactKeys(value, [
      'version',
      'workbenchKind',
      'layout',
      'prompt',
      'model',
      'parameters',
      'referenceAssetIds',
    ]) ||
    value.version !== STANDALONE_WORKBENCH_DRAFT_VERSION ||
    value.workbenchKind !== expectedKind
  ) {
    return null;
  }
  const prompt = boundedString(
    value.prompt,
    STANDALONE_WORKBENCH_DRAFT_MAX_PROMPT_LENGTH
  );
  const model = parseModel(value.model);
  const referenceAssetIds = parseReferenceAssetIds(
    value.referenceAssetIds,
    maximumReferences
  );
  if (prompt === null || model === undefined || referenceAssetIds === null) return null;
  return { record: value, prompt, model, referenceAssetIds };
};

const parseImageDraft = (value: unknown): ImageWorkbenchSessionDraft | null => {
  const base = parseBase(value, 'image', MAX_IMAGE_REFERENCE_ASSETS);
  if (!base || (base.record.layout !== 'side' && base.record.layout !== 'bottom')) {
    return null;
  }
  const parameters = base.record.parameters;
  if (
    !isRecord(parameters) ||
    !hasExactKeys(parameters, [
      'interfaceMode',
      'quality',
      'width',
      'height',
      'aspectRatio',
      'count',
    ]) ||
    (parameters.interfaceMode !== 'images' && parameters.interfaceMode !== 'responses') ||
    (parameters.quality !== 'auto' &&
      parameters.quality !== 'high' &&
      parameters.quality !== 'medium' &&
      parameters.quality !== 'low')
  ) {
    return null;
  }
  const width = nullableDimension(parameters.width);
  const height = nullableDimension(parameters.height);
  const aspectRatio = boundedString(parameters.aspectRatio, MAX_ASPECT_RATIO_LENGTH, {
    nonempty: true,
    normalized: true,
  });
  const count = safeInteger(parameters.count, 1, 10);
  if (
    width === undefined ||
    height === undefined ||
    (width === null) !== (height === null) ||
    aspectRatio === null ||
    count === null
  ) {
    return null;
  }
  return {
    version: STANDALONE_WORKBENCH_DRAFT_VERSION,
    workbenchKind: 'image',
    layout: base.record.layout,
    prompt: base.prompt,
    model: base.model ? { ...base.model } : null,
    parameters: {
      interfaceMode: parameters.interfaceMode,
      quality: parameters.quality,
      width,
      height,
      aspectRatio,
      count,
    },
    referenceAssetIds: base.referenceAssetIds,
  };
};

const parseVideoDraft = (value: unknown): VideoWorkbenchSessionDraft | null => {
  const base = parseBase(value, 'video', 1);
  if (!base || (base.record.layout !== 'side' && base.record.layout !== 'bottom')) {
    return null;
  }
  const parameters = base.record.parameters;
  if (
    !isRecord(parameters) ||
    !hasExactKeys(parameters, ['resolution', 'aspect', 'duration', 'taskCount']) ||
    (parameters.resolution !== '720p' && parameters.resolution !== '1080p') ||
    (parameters.aspect !== '16:9' &&
      parameters.aspect !== '9:16' &&
      parameters.aspect !== '1:1') ||
    (parameters.duration !== '5' && parameters.duration !== '10') ||
    parameters.taskCount !== 1
  ) {
    return null;
  }
  return {
    version: STANDALONE_WORKBENCH_DRAFT_VERSION,
    workbenchKind: 'video',
    layout: base.record.layout,
    prompt: base.prompt,
    model: base.model ? { ...base.model } : null,
    parameters: {
      resolution: parameters.resolution,
      aspect: parameters.aspect,
      duration: parameters.duration,
      taskCount: 1,
    },
    referenceAssetIds: base.referenceAssetIds,
  };
};

export function parseStandaloneWorkbenchDraft(
  workbenchKind: 'image',
  value: unknown
): ImageWorkbenchSessionDraft | null;
export function parseStandaloneWorkbenchDraft(
  workbenchKind: 'video',
  value: unknown
): VideoWorkbenchSessionDraft | null;
export function parseStandaloneWorkbenchDraft(
  workbenchKind: StandaloneWorkbenchDraftKind,
  value: unknown
): StandaloneWorkbenchSessionDraft | null;
export function parseStandaloneWorkbenchDraft(
  workbenchKind: StandaloneWorkbenchDraftKind,
  value: unknown
): StandaloneWorkbenchSessionDraft | null {
  return workbenchKind === 'image' ? parseImageDraft(value) : parseVideoDraft(value);
}

const safelyRemove = (
  storage: StandaloneWorkbenchDraftStorage,
  workbenchKind: StandaloneWorkbenchDraftKind
): void => {
  try {
    storage.removeItem(standaloneWorkbenchDraftStorageKey(workbenchKind));
  } catch {
    // Embedded/privacy-restricted hosts may reject browser storage operations.
  }
};

export function readStandaloneWorkbenchDraft(
  workbenchKind: 'image',
  storage?: StandaloneWorkbenchDraftStorage | null
): ImageWorkbenchSessionDraft | null;
export function readStandaloneWorkbenchDraft(
  workbenchKind: 'video',
  storage?: StandaloneWorkbenchDraftStorage | null
): VideoWorkbenchSessionDraft | null;
export function readStandaloneWorkbenchDraft(
  workbenchKind: StandaloneWorkbenchDraftKind,
  storage?: StandaloneWorkbenchDraftStorage | null
): StandaloneWorkbenchSessionDraft | null;
export function readStandaloneWorkbenchDraft(
  workbenchKind: StandaloneWorkbenchDraftKind,
  storage: StandaloneWorkbenchDraftStorage | null = browserSessionStorage()
): StandaloneWorkbenchSessionDraft | null {
  if (!storage) return null;
  let raw: string | null;
  try {
    raw = storage.getItem(standaloneWorkbenchDraftStorageKey(workbenchKind));
  } catch {
    return null;
  }
  if (raw === null) return null;
  if (raw.length > STANDALONE_WORKBENCH_DRAFT_MAX_SERIALIZED_LENGTH) {
    safelyRemove(storage, workbenchKind);
    return null;
  }
  try {
    const draft = parseStandaloneWorkbenchDraft(workbenchKind, JSON.parse(raw));
    if (!draft) safelyRemove(storage, workbenchKind);
    return draft;
  } catch {
    safelyRemove(storage, workbenchKind);
    return null;
  }
}

export function writeStandaloneWorkbenchDraft(
  draft: StandaloneWorkbenchSessionDraft,
  storage: StandaloneWorkbenchDraftStorage | null = browserSessionStorage()
): boolean {
  if (!storage) return false;
  const workbenchKind = draft?.workbenchKind;
  if (workbenchKind !== 'image' && workbenchKind !== 'video') return false;
  const parsed = parseStandaloneWorkbenchDraft(workbenchKind, draft);
  if (!parsed) {
    safelyRemove(storage, workbenchKind);
    return false;
  }
  const serialized = JSON.stringify(parsed);
  if (serialized.length > STANDALONE_WORKBENCH_DRAFT_MAX_SERIALIZED_LENGTH) {
    safelyRemove(storage, workbenchKind);
    return false;
  }
  try {
    storage.setItem(standaloneWorkbenchDraftStorageKey(workbenchKind), serialized);
    return true;
  } catch {
    safelyRemove(storage, workbenchKind);
    return false;
  }
}

export function clearStandaloneWorkbenchDraft(
  workbenchKind: StandaloneWorkbenchDraftKind,
  storage: StandaloneWorkbenchDraftStorage | null = browserSessionStorage()
): void {
  if (storage) safelyRemove(storage, workbenchKind);
}
