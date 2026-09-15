/** One-way import. Never writes the retired workbench storage format. */
import { parseAssetId, parseProviderId } from '@/common/types/ids';
import type { CreativeModelSelectionRef } from '@renderer/pages/creativeStudio/models';
const STANDALONE_WORKBENCH_DRAFT_MAX_PROMPT_LENGTH = 20_000;
const STANDALONE_WORKBENCH_DRAFT_VERSION = 1 as const;

export type LegacyDraftKind = 'image' | 'video';

export interface LegacyImageParameters {
  interfaceMode: 'images' | 'responses';
  quality: 'auto' | 'high' | 'medium' | 'low';
  width: number | null;
  height: number | null;
  aspectRatio: string;
  count: number;
}

export interface LegacyVideoParameters {
  resolution: '720p' | '1080p';
  aspect: '16:9' | '9:16' | '1:1' | 'auto';
  duration: '5' | '10';
  taskCount: 1;
}

interface LegacySessionDraftBase {
  version: typeof STANDALONE_WORKBENCH_DRAFT_VERSION;
  prompt: string;
  model: CreativeModelSelectionRef | null;
  referenceAssetIds: string[];
}

export interface LegacyImageDraft
  extends LegacySessionDraftBase {
  workbenchKind: 'image';
  layout: 'side' | 'bottom';
  parameters: LegacyImageParameters;
}

export interface LegacyVideoDraft
  extends LegacySessionDraftBase {
  workbenchKind: 'video';
  layout: 'side' | 'bottom';
  parameters: LegacyVideoParameters;
}

export type LegacySessionDraft =
  | LegacyImageDraft
  | LegacyVideoDraft;


const MAX_MODEL_LENGTH = 512;
const MAX_ASPECT_RATIO_LENGTH = 32;
const MAX_IMAGE_REFERENCE_ASSETS = 100;

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
): LegacySessionDraft['model'] | undefined => {
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
  expectedKind: LegacyDraftKind,
  maximumReferences: number
): {
  record: Record<string, unknown>;
  prompt: string;
  model: LegacySessionDraft['model'];
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

const parseImageDraft = (value: unknown): LegacyImageDraft | null => {
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

const parseVideoDraft = (value: unknown): LegacyVideoDraft | null => {
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
      parameters.aspect !== '1:1' &&
      parameters.aspect !== 'auto') ||
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

export function parseLegacyCreationDraft(
  workbenchKind: 'image',
  value: unknown
): LegacyImageDraft | null;
export function parseLegacyCreationDraft(
  workbenchKind: 'video',
  value: unknown
): LegacyVideoDraft | null;
export function parseLegacyCreationDraft(
  workbenchKind: LegacyDraftKind,
  value: unknown
): LegacySessionDraft | null;
export function parseLegacyCreationDraft(
  workbenchKind: LegacyDraftKind,
  value: unknown
): LegacySessionDraft | null {
  return workbenchKind === 'image' ? parseImageDraft(value) : parseVideoDraft(value);
}


export type LegacyCreationDraft = LegacySessionDraft & { source: string };
export type LegacyDraftStorage = Pick<Storage, 'getItem' | 'removeItem'>;
const legacyDraftKey = (kind: LegacyDraftKind): string => 'nomifun:creative-studio:standalone-workbench-draft:' + kind;
const sessionStorageOrNull = (): LegacyDraftStorage | null => {
  try { return typeof window === 'undefined' ? null : window.sessionStorage; } catch { return null; }
};
export function readLegacyCreationDraft(kind: LegacyDraftKind, storage: LegacyDraftStorage | null = sessionStorageOrNull()): LegacyCreationDraft | null {
  try {
    const source = storage?.getItem(legacyDraftKey(kind));
    if (!source || source.length > 128 * 1024) return null;
    const draft = parseLegacyCreationDraft(kind, JSON.parse(source));
    return draft ? { ...draft, source } : null;
  } catch { return null; }
}
/** Acknowledge only after the destination draft is saved, and only the value read. */
export function acknowledgeLegacyCreationDraft(kind: LegacyDraftKind, source: string, storage: LegacyDraftStorage | null = sessionStorageOrNull()): boolean {
  try {
    if (!storage || storage.getItem(legacyDraftKey(kind)) !== source) return false;
    storage.removeItem(legacyDraftKey(kind));
    return true;
  } catch { return false; }
}
