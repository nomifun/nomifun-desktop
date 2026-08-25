/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  CREATIVE_STUDIO_DOCUMENT_SCHEMA,
  type CreateCreativeProjectRequest,
  type CreativeAudioComposerDraft,
  type CreativeAudioNodeData,
  type CreativeBottomPanelView,
  type CreativeCanvasBackground,
  type CreativeCanvasConnection,
  type CreativeCanvasNode,
  type CreativeCanvasNodeKind,
  type CreativeChatModelReference,
  type CreativeChatPendingTurn,
  type CreativeChatSessionReference,
  type CreativeComposerModel,
  type CreativeConfigNodeData,
  type CreativeConfigOperation,
  type CreativeDirectorNodeData,
  type CreativeGenerationStatus,
  type CreativeGroupNodeData,
  type CreativeImageComposerDraft,
  type CreativeImageNodeData,
  type CreativeJsonObject,
  type CreativeJsonValue,
  type CreativeLeftPanelView,
  type CreativeModelTask,
  type CreativePanoramaNodeData,
  type CreativeProjectDetail,
  type CreativeProjectDocument,
  type CreativeProjectListResponse,
  type CreativeProjectResponse,
  type CreativeProjectSummary,
  type CreativeRightPanelView,
  type CreativeStudioPanelState,
  type CreativeTextNodeData,
  type CreativeVideoNodeData,
  type CreativeVideoComposerDraft,
  type RenameCreativeProjectRequest,
  type SaveCreativeProjectRequest,
} from './schema';

export type CreativeStudioContractErrorCode =
  | 'INVALID_REQUEST'
  | 'INVALID_RESPONSE'
  | 'INVALID_DOCUMENT'
  | 'SCHEMA_MISMATCH'
  | 'CANVAS_MISMATCH'
  | 'PROJECT_MISMATCH';

/** A deterministic runtime-contract error safe for UI branching and tests. */
export class CreativeStudioContractError extends TypeError {
  readonly code: CreativeStudioContractErrorCode;
  readonly path: string;
  readonly expected: string;

  constructor(code: CreativeStudioContractErrorCode, path: string, expected: string) {
    super(`Creative Studio contract violation at ${path}: expected ${expected}`);
    this.name = 'CreativeStudioContractError';
    this.code = code;
    this.path = path;
    this.expected = expected;
  }
}

export function isCreativeStudioContractError(error: unknown): error is CreativeStudioContractError {
  return (
    error instanceof CreativeStudioContractError ||
    (!!error &&
      typeof error === 'object' &&
      'name' in error &&
      (error as { name: unknown }).name === 'CreativeStudioContractError')
  );
}

type UnknownRecord = Record<string, unknown>;

const fail = (
  code: CreativeStudioContractErrorCode,
  path: string,
  expected: string
): never => {
  throw new CreativeStudioContractError(code, path, expected);
};

const asRecord = (
  value: unknown,
  path: string,
  code: CreativeStudioContractErrorCode
): UnknownRecord => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return fail(code, path, 'object');
  }
  return value as UnknownRecord;
};

const exactKeys = (
  value: UnknownRecord,
  required: readonly string[],
  optional: readonly string[],
  path: string,
  code: CreativeStudioContractErrorCode
): void => {
  const allowed = new Set([...required, ...optional]);
  for (const key of required) {
    if (!Object.prototype.hasOwnProperty.call(value, key)) {
      fail(code, `${path}.${key}`, 'present');
    }
  }
  for (const key of Object.keys(value)) {
    if (!allowed.has(key)) fail(code, `${path}.${key}`, 'no unknown fields');
  }
};

const asString = (
  value: unknown,
  path: string,
  code: CreativeStudioContractErrorCode,
  options: { allowEmpty?: boolean; maxLength?: number } = {}
): string => {
  const { allowEmpty = false, maxLength = 4096 } = options;
  if (typeof value !== 'string' || (!allowEmpty && value.length === 0) || value.length > maxLength) {
    return fail(code, path, allowEmpty ? `string <= ${maxLength} chars` : `non-empty string <= ${maxLength} chars`);
  }
  return value;
};

const asId = (value: unknown, path: string, code: CreativeStudioContractErrorCode): string =>
  asString(value, path, code, { maxLength: 256 });

const UUID_V7_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

const asUuidV7Id = (
  value: unknown,
  path: string,
  code: CreativeStudioContractErrorCode
): string => {
  const id = asId(value, path, code);
  if (!UUID_V7_PATTERN.test(id) || id !== id.toLowerCase()) {
    return fail(code, path, 'canonical lowercase UUIDv7');
  }
  return id;
};

const asProjectId = (
  value: unknown,
  path: string,
  code: CreativeStudioContractErrorCode
): string => {
  const projectId = asId(value, path, code);
  if (!UUID_V7_PATTERN.test(projectId)) return fail(code, path, 'UUIDv7 project id');
  return projectId;
};

const asNullableId = (
  value: unknown,
  path: string,
  code: CreativeStudioContractErrorCode
): string | null => (value === null ? null : asId(value, path, code));

const asNumber = (
  value: unknown,
  path: string,
  code: CreativeStudioContractErrorCode,
  options: { min?: number; max?: number; integer?: boolean } = {}
): number => {
  if (typeof value !== 'number' || !Number.isFinite(value)) return fail(code, path, 'finite number');
  if (options.integer && !Number.isInteger(value)) return fail(code, path, 'integer');
  if (options.min !== undefined && value < options.min) return fail(code, path, `number >= ${options.min}`);
  if (options.max !== undefined && value > options.max) return fail(code, path, `number <= ${options.max}`);
  return value;
};

const asBoolean = (value: unknown, path: string, code: CreativeStudioContractErrorCode): boolean => {
  if (typeof value !== 'boolean') return fail(code, path, 'boolean');
  return value;
};

const asLiteral = <T extends string>(
  value: unknown,
  literals: readonly T[],
  path: string,
  code: CreativeStudioContractErrorCode
): T => {
  if (typeof value !== 'string' || !literals.includes(value as T)) {
    return fail(code, path, literals.map((item) => JSON.stringify(item)).join(' | '));
  }
  return value as T;
};

const asArray = <T>(
  value: unknown,
  path: string,
  code: CreativeStudioContractErrorCode,
  parseItem: (item: unknown, itemPath: string) => T
): T[] => {
  if (!Array.isArray(value)) return fail(code, path, 'array');
  return value.map((item, index) => parseItem(item, `${path}[${index}]`));
};

const asIdArray = (
  value: unknown,
  path: string,
  code: CreativeStudioContractErrorCode
): string[] => asArray(value, path, code, (item, itemPath) => asId(item, itemPath, code));

const assertUnique = (
  values: readonly string[],
  path: string,
  code: CreativeStudioContractErrorCode
): void => {
  if (new Set(values).size !== values.length) fail(code, path, 'unique values');
};

const parseSize = (
  value: unknown,
  path: string,
  code: CreativeStudioContractErrorCode
): { width: number; height: number } => {
  const record = asRecord(value, path, code);
  exactKeys(record, ['width', 'height'], [], path, code);
  return {
    width: asNumber(record.width, `${path}.width`, code, { min: 1 }),
    height: asNumber(record.height, `${path}.height`, code, { min: 1 }),
  };
};

const parseNullableNaturalSize = (
  value: unknown,
  path: string,
  code: CreativeStudioContractErrorCode
): { width: number; height: number } | null => (value === null ? null : parseSize(value, path, code));

const parseJsonValue = (
  value: unknown,
  path: string,
  code: CreativeStudioContractErrorCode,
  seen: Set<object>,
  depth: number
): CreativeJsonValue => {
  if (depth > 40) return fail(code, path, 'JSON value no deeper than 40 levels');
  if (value === null || typeof value === 'string' || typeof value === 'boolean') return value;
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) return fail(code, path, 'finite JSON number');
    return value;
  }
  if (typeof value !== 'object') return fail(code, path, 'JSON value');
  if (seen.has(value)) return fail(code, path, 'acyclic JSON value');
  seen.add(value);
  if (Array.isArray(value)) {
    const parsed = value.map((item, index) => parseJsonValue(item, `${path}[${index}]`, code, seen, depth + 1));
    seen.delete(value);
    return parsed;
  }
  const output: CreativeJsonObject = {};
  for (const [key, entry] of Object.entries(value as UnknownRecord)) {
    output[key] = parseJsonValue(entry, `${path}.${key}`, code, seen, depth + 1);
  }
  seen.delete(value);
  return output;
};

const parseJsonObject = (
  value: unknown,
  path: string,
  code: CreativeStudioContractErrorCode
): CreativeJsonObject => {
  const record = asRecord(value, path, code);
  return parseJsonValue(record, path, code, new Set(), 0) as CreativeJsonObject;
};

const parseComposerModel = (
  value: unknown,
  path: string
): CreativeComposerModel | null => {
  const code = 'INVALID_DOCUMENT';
  if (value === null) return null;
  const record = asRecord(value, path, code);
  exactKeys(record, ['providerId', 'model'], [], path, code);
  const model = asString(record.model, `${path}.model`, code, { maxLength: 512 });
  if (model !== model.trim()) {
    fail(code, `${path}.model`, 'trimmed non-empty model id');
  }
  return {
    providerId: asUuidV7Id(record.providerId, `${path}.providerId`, code),
    model,
  };
};

const parseImageData = (value: unknown, path: string): CreativeImageNodeData => {
  const code = 'INVALID_DOCUMENT';
  const record = asRecord(value, path, code);
  exactKeys(
    record,
    ['assetId', 'caption', 'alt', 'fit', 'naturalSize'],
    ['composer'],
    path,
    code
  );
  return {
    assetId: asNullableId(record.assetId, `${path}.assetId`, code),
    caption: asString(record.caption, `${path}.caption`, code, { allowEmpty: true, maxLength: 20_000 }),
    alt: asString(record.alt, `${path}.alt`, code, { allowEmpty: true, maxLength: 2_000 }),
    fit: asLiteral(record.fit, ['contain', 'cover'], `${path}.fit`, code),
    naturalSize: parseNullableNaturalSize(record.naturalSize, `${path}.naturalSize`, code),
    composer:
      record.composer === undefined || record.composer === null
        ? null
        : parseImageComposerDraft(record.composer, `${path}.composer`),
  };
};

const parseImageComposerDraft = (
  value: unknown,
  path: string
): CreativeImageComposerDraft => {
  const code = 'INVALID_DOCUMENT';
  const record = asRecord(value, path, code);
  exactKeys(
    record,
    [
      'prompt',
      'model',
      'interfaceMode',
      'quality',
      'width',
      'height',
      'aspectRatio',
      'count',
    ],
    ['mentions'],
    path,
    code
  );
  const prompt = asString(record.prompt, `${path}.prompt`, code, {
    allowEmpty: true,
    maxLength: 1_000_000,
  });
  const mentions =
    record.mentions === undefined
      ? []
      : asArray(
          record.mentions,
          `${path}.mentions`,
          code,
          (entry, indexPath) => {
            const mention = asRecord(entry, indexPath, code);
            exactKeys(
              mention,
              ['id', 'sourceNodeId', 'fallbackLabel', 'start', 'end'],
              [],
              indexPath,
              code
            );
            const fallbackLabel = asString(
              mention.fallbackLabel,
              `${indexPath}.fallbackLabel`,
              code,
              {
              maxLength: 128,
              }
            );
            if (
              fallbackLabel !== fallbackLabel.trim() ||
              fallbackLabel.startsWith('@') ||
              /[\r\n]/u.test(fallbackLabel)
            ) {
              fail(
                code,
                `${indexPath}.fallbackLabel`,
                'trimmed single-line label without @ prefix'
              );
            }
            const start = asNumber(mention.start, `${indexPath}.start`, code, {
              min: 0,
              max: 1_000_000,
              integer: true,
            });
            const end = asNumber(mention.end, `${indexPath}.end`, code, {
              min: start + 1,
              max: 1_000_000,
              integer: true,
            });
            return {
              id: asString(mention.id, `${indexPath}.id`, code, { maxLength: 128 }),
              sourceNodeId: asId(
                mention.sourceNodeId,
                `${indexPath}.sourceNodeId`,
                code
              ),
              fallbackLabel,
              start,
              end,
            };
          }
        );
  assertUnique(
    mentions.map((mention) => mention.id),
    `${path}.mentions[].id`,
    code
  );
  const sortedMentions = [...mentions].sort(
    (left, right) => left.start - right.start || left.end - right.end
  );
  for (const [index, mention] of sortedMentions.entries()) {
    const token = `@${mention.fallbackLabel}`;
    if (mention.end > prompt.length || prompt.slice(mention.start, mention.end) !== token) {
      fail(code, `${path}.mentions[${index}]`, 'range matching the authored @label token');
    }
    const previous = sortedMentions[index - 1];
    if (previous && mention.start < previous.end) {
      fail(code, `${path}.mentions[${index}].start`, 'range not overlapping another mention');
    }
  }
  const model = parseComposerModel(record.model, `${path}.model`);
  const nullableDimension = (entry: unknown, entryPath: string): number | null =>
    entry === null
      ? null
      : asNumber(entry, entryPath, code, {
          min: 1,
          max: 8192,
          integer: true,
        });
  const width = nullableDimension(record.width, `${path}.width`);
  const height = nullableDimension(record.height, `${path}.height`);
  if ((width === null) !== (height === null)) {
    fail(code, path, 'width and height both null or both positive integers');
  }
  const aspectRatio = asString(
    record.aspectRatio,
    `${path}.aspectRatio`,
    code,
    { maxLength: 128 }
  );
  if (aspectRatio !== aspectRatio.trim()) {
    fail(code, `${path}.aspectRatio`, 'trimmed non-empty aspect ratio');
  }
  return {
    prompt,
    ...(record.mentions === undefined ? {} : { mentions }),
    model,
    interfaceMode: asLiteral(
      record.interfaceMode,
      ['images', 'responses'],
      `${path}.interfaceMode`,
      code
    ),
    quality: asLiteral(
      record.quality,
      ['auto', 'high', 'medium', 'low'],
      `${path}.quality`,
      code
    ),
    width,
    height,
    aspectRatio,
    count: asNumber(record.count, `${path}.count`, code, {
      min: 1,
      max: 10,
      integer: true,
    }),
  };
};

const parsePanoramaData = (value: unknown, path: string): CreativePanoramaNodeData => {
  const code = 'INVALID_DOCUMENT';
  const record = asRecord(value, path, code);
  exactKeys(record, ['assetId', 'projection', 'yaw', 'pitch', 'fieldOfView'], [], path, code);
  return {
    assetId: asNullableId(record.assetId, `${path}.assetId`, code),
    projection: asLiteral(record.projection, ['equirectangular'], `${path}.projection`, code),
    yaw: asNumber(record.yaw, `${path}.yaw`, code, { min: -360, max: 360 }),
    pitch: asNumber(record.pitch, `${path}.pitch`, code, { min: -90, max: 90 }),
    fieldOfView: asNumber(record.fieldOfView, `${path}.fieldOfView`, code, { min: 10, max: 150 }),
  };
};

const parseTextData = (value: unknown, path: string): CreativeTextNodeData => {
  const code = 'INVALID_DOCUMENT';
  const record = asRecord(value, path, code);
  exactKeys(record, ['text', 'format', 'fontSize', 'textAlign'], [], path, code);
  return {
    text: asString(record.text, `${path}.text`, code, { allowEmpty: true, maxLength: 1_000_000 }),
    format: asLiteral(record.format, ['plain', 'markdown'], `${path}.format`, code),
    fontSize: asNumber(record.fontSize, `${path}.fontSize`, code, { min: 8, max: 256 }),
    textAlign: asLiteral(record.textAlign, ['left', 'center', 'right'], `${path}.textAlign`, code),
  };
};

const MODEL_TASKS: readonly CreativeModelTask[] = [
  'chat',
  'image_generation',
  'image_edit',
  'video_generation',
  'speech_synthesis',
];
const GENERATION_STATUSES: readonly CreativeGenerationStatus[] = [
  'idle',
  'queued',
  'running',
  'succeeded',
  'failed',
  'canceled',
];

const parseConfigOperation = (
  value: unknown,
  path: string
): CreativeConfigOperation | null => {
  const code = 'INVALID_DOCUMENT';
  if (value === undefined || value === null) return null;
  const record = asRecord(value, path, code);
  const kind = asLiteral(
    record.kind,
    [
      'image-node-compose',
      'image-mask-edit',
      'video-node-compose',
      'audio-node-compose',
    ],
    `${path}.kind`,
    code
  );
  if (kind === 'image-mask-edit') {
    exactKeys(
      record,
      ['kind', 'sourceNodeId', 'sourceAssetId', 'markedReferenceAssetId'],
      [],
      path,
      code
    );
    return {
      kind,
      sourceNodeId: asId(record.sourceNodeId, `${path}.sourceNodeId`, code),
      sourceAssetId: asId(record.sourceAssetId, `${path}.sourceAssetId`, code),
      markedReferenceAssetId: asId(
        record.markedReferenceAssetId,
        `${path}.markedReferenceAssetId`,
        code
      ),
    };
  }
  exactKeys(
    record,
    ['kind', 'sourceNodeId', 'sourceAssetId'],
    [],
    path,
    code
  );
  return {
    kind,
    sourceNodeId: asId(record.sourceNodeId, `${path}.sourceNodeId`, code),
    sourceAssetId: asNullableId(
      record.sourceAssetId,
      `${path}.sourceAssetId`,
      code
    ),
  };
};

const LEGACY_CONFIG_OPERATION_KEYS = [
  'canvasOperation',
  'sourceNodeId',
  'sourceAssetId',
  'markedReferenceAssetId',
  'userPrompt',
  'referenceWidth',
  'referenceHeight',
] as const;

const normalizeConfigOperation = (
  explicit: CreativeConfigOperation | null,
  parameters: CreativeJsonObject,
  path: string
): CreativeConfigOperation | null => {
  const code = 'INVALID_DOCUMENT';
  const legacyKind = parameters.canvasOperation;
  if (legacyKind === undefined) return explicit;
  if (explicit) {
    fail(code, `${path}.parameters.canvasOperation`, 'absent when operation is present');
  }
  if (
    legacyKind !== 'image-node-compose' &&
    legacyKind !== 'image-mask-edit' &&
    legacyKind !== 'video-node-compose' &&
    legacyKind !== 'audio-node-compose'
  ) {
    fail(code, `${path}.parameters.canvasOperation`, 'known canvas operation');
  }
  const kind = legacyKind as CreativeConfigOperation['kind'];
  const sourceNodeId = asId(
    parameters.sourceNodeId,
    `${path}.parameters.sourceNodeId`,
    code
  );
  const sourceAssetId =
    parameters.sourceAssetId === null
      ? null
      : asId(
          parameters.sourceAssetId,
          `${path}.parameters.sourceAssetId`,
          code
        );
  let operation: CreativeConfigOperation;
  if (kind === 'image-mask-edit') {
    if (sourceAssetId === null) {
      fail(code, `${path}.parameters.sourceAssetId`, 'non-null asset id');
    }
    operation = {
      kind,
      sourceNodeId,
      sourceAssetId: sourceAssetId as string,
      markedReferenceAssetId: asId(
        parameters.markedReferenceAssetId,
        `${path}.parameters.markedReferenceAssetId`,
        code
      ),
    };
  } else {
    operation = { kind, sourceNodeId, sourceAssetId };
  }
  for (const key of LEGACY_CONFIG_OPERATION_KEYS) delete parameters[key];
  return operation;
};

const parseConfigData = (value: unknown, path: string): CreativeConfigNodeData => {
  const code = 'INVALID_DOCUMENT';
  const record = asRecord(value, path, code);
  exactKeys(
    record,
    [
      'task',
      'capability',
      'providerId',
      'model',
      'prompt',
      'negativePrompt',
      'parameters',
      'inputAssetIds',
      'taskId',
      'resultAssetIds',
      'status',
      'errorMessage',
    ],
    ['operation'],
    path,
    code
  );
  const inputAssetIds = asIdArray(record.inputAssetIds, `${path}.inputAssetIds`, code);
  const resultAssetIds = asIdArray(record.resultAssetIds, `${path}.resultAssetIds`, code);
  const parameters = parseJsonObject(record.parameters, `${path}.parameters`, code);
  const operation = normalizeConfigOperation(
    parseConfigOperation(record.operation, `${path}.operation`),
    parameters,
    path
  );
  assertUnique(inputAssetIds, `${path}.inputAssetIds`, code);
  assertUnique(resultAssetIds, `${path}.resultAssetIds`, code);
  return {
    task: asLiteral(record.task, MODEL_TASKS, `${path}.task`, code),
    capability: asString(record.capability, `${path}.capability`, code, { maxLength: 128 }),
    providerId: asNullableId(record.providerId, `${path}.providerId`, code),
    model: record.model === null ? null : asString(record.model, `${path}.model`, code, { maxLength: 512 }),
    prompt: asString(record.prompt, `${path}.prompt`, code, { allowEmpty: true, maxLength: 1_000_000 }),
    negativePrompt: asString(record.negativePrompt, `${path}.negativePrompt`, code, {
      allowEmpty: true,
      maxLength: 1_000_000,
    }),
    operation,
    parameters,
    inputAssetIds,
    taskId: asNullableId(record.taskId, `${path}.taskId`, code),
    resultAssetIds,
    status: asLiteral(record.status, GENERATION_STATUSES, `${path}.status`, code),
    errorMessage:
      record.errorMessage === null
        ? null
        : asString(record.errorMessage, `${path}.errorMessage`, code, { allowEmpty: true, maxLength: 20_000 }),
  };
};

const parseVideoComposerDraft = (
  value: unknown,
  path: string
): CreativeVideoComposerDraft => {
  const code = 'INVALID_DOCUMENT';
  const record = asRecord(value, path, code);
  exactKeys(
    record,
    ['prompt', 'model', 'resolution', 'aspectRatio', 'seconds'],
    [],
    path,
    code
  );
  const trimmed = (entry: unknown, entryPath: string): string => {
    const parsed = asString(entry, entryPath, code, { maxLength: 128 });
    if (parsed !== parsed.trim()) {
      fail(code, entryPath, 'trimmed non-empty string');
    }
    return parsed;
  };
  return {
    prompt: asString(record.prompt, `${path}.prompt`, code, {
      allowEmpty: true,
      maxLength: 1_000_000,
    }),
    model: parseComposerModel(record.model, `${path}.model`),
    resolution: trimmed(record.resolution, `${path}.resolution`),
    aspectRatio: trimmed(record.aspectRatio, `${path}.aspectRatio`),
    seconds: asNumber(record.seconds, `${path}.seconds`, code, {
      min: 1,
      max: 3_600,
      integer: true,
    }),
  };
};

const parseVideoData = (value: unknown, path: string): CreativeVideoNodeData => {
  const code = 'INVALID_DOCUMENT';
  const record = asRecord(value, path, code);
  exactKeys(
    record,
    ['assetId', 'posterAssetId', 'autoplay', 'loop', 'muted', 'trimStartMs', 'trimEndMs'],
    ['composer'],
    path,
    code
  );
  const trimStartMs = asNumber(record.trimStartMs, `${path}.trimStartMs`, code, { min: 0 });
  const trimEndMs =
    record.trimEndMs === null
      ? null
      : asNumber(record.trimEndMs, `${path}.trimEndMs`, code, { min: trimStartMs });
  return {
    assetId: asNullableId(record.assetId, `${path}.assetId`, code),
    posterAssetId: asNullableId(record.posterAssetId, `${path}.posterAssetId`, code),
    autoplay: asBoolean(record.autoplay, `${path}.autoplay`, code),
    loop: asBoolean(record.loop, `${path}.loop`, code),
    muted: asBoolean(record.muted, `${path}.muted`, code),
    trimStartMs,
    trimEndMs,
    composer:
      record.composer === undefined || record.composer === null
        ? null
        : parseVideoComposerDraft(record.composer, `${path}.composer`),
  };
};

const parseAudioComposerDraft = (
  value: unknown,
  path: string
): CreativeAudioComposerDraft => {
  const code = 'INVALID_DOCUMENT';
  const record = asRecord(value, path, code);
  exactKeys(record, ['prompt', 'model', 'voice', 'format'], [], path, code);
  const voice = asString(record.voice, `${path}.voice`, code, {
    allowEmpty: true,
    maxLength: 256,
  });
  if (voice !== voice.trim()) {
    fail(code, `${path}.voice`, 'trimmed string');
  }
  return {
    prompt: asString(record.prompt, `${path}.prompt`, code, {
      allowEmpty: true,
      maxLength: 1_000_000,
    }),
    model: parseComposerModel(record.model, `${path}.model`),
    voice,
    format: asLiteral(record.format, ['mp3', 'wav'], `${path}.format`, code),
  };
};

const parseAudioData = (value: unknown, path: string): CreativeAudioNodeData => {
  const code = 'INVALID_DOCUMENT';
  const record = asRecord(value, path, code);
  exactKeys(
    record,
    ['assetId', 'title', 'loop', 'volume', 'trimStartMs', 'trimEndMs'],
    ['composer'],
    path,
    code
  );
  const trimStartMs = asNumber(record.trimStartMs, `${path}.trimStartMs`, code, { min: 0 });
  const trimEndMs =
    record.trimEndMs === null
      ? null
      : asNumber(record.trimEndMs, `${path}.trimEndMs`, code, { min: trimStartMs });
  return {
    assetId: asNullableId(record.assetId, `${path}.assetId`, code),
    title: asString(record.title, `${path}.title`, code, { allowEmpty: true, maxLength: 1_000 }),
    loop: asBoolean(record.loop, `${path}.loop`, code),
    volume: asNumber(record.volume, `${path}.volume`, code, { min: 0, max: 1 }),
    trimStartMs,
    trimEndMs,
    composer:
      record.composer === undefined || record.composer === null
        ? null
        : parseAudioComposerDraft(record.composer, `${path}.composer`),
  };
};

const parseDirectorData = (value: unknown, path: string): CreativeDirectorNodeData => {
  const code = 'INVALID_DOCUMENT';
  const record = asRecord(value, path, code);
  exactKeys(record, ['sceneId', 'cameraId', 'timelineMs', 'durationMs'], [], path, code);
  const durationMs = asNumber(record.durationMs, `${path}.durationMs`, code, { min: 0 });
  return {
    sceneId: asNullableId(record.sceneId, `${path}.sceneId`, code),
    cameraId: asNullableId(record.cameraId, `${path}.cameraId`, code),
    timelineMs: asNumber(record.timelineMs, `${path}.timelineMs`, code, { min: 0, max: durationMs }),
    durationMs,
  };
};

const parseGroupData = (value: unknown, path: string): CreativeGroupNodeData => {
  const code = 'INVALID_DOCUMENT';
  const record = asRecord(value, path, code);
  exactKeys(record, ['title', 'color', 'collapsed'], [], path, code);
  return {
    title: asString(record.title, `${path}.title`, code, { maxLength: 1_000 }),
    color:
      record.color === null
        ? null
        : asString(record.color, `${path}.color`, code, { maxLength: 128 }),
    collapsed: asBoolean(record.collapsed, `${path}.collapsed`, code),
  };
};

const NODE_KINDS: readonly CreativeCanvasNodeKind[] = [
  'image',
  'panorama',
  'text',
  'config',
  'video',
  'audio',
  'director',
  'group',
];

const parseNode = (value: unknown, path: string): CreativeCanvasNode => {
  const code = 'INVALID_DOCUMENT';
  const record = asRecord(value, path, code);
  exactKeys(record, ['id', 'type', 'position', 'size', 'groupId', 'zIndex', 'locked', 'data'], [], path, code);
  const type = asLiteral(record.type, NODE_KINDS, `${path}.type`, code);
  const position = asRecord(record.position, `${path}.position`, code);
  exactKeys(position, ['x', 'y'], [], `${path}.position`, code);
  const base = {
    id: asId(record.id, `${path}.id`, code),
    position: {
      x: asNumber(position.x, `${path}.position.x`, code),
      y: asNumber(position.y, `${path}.position.y`, code),
    },
    size: parseSize(record.size, `${path}.size`, code),
    groupId: asNullableId(record.groupId, `${path}.groupId`, code),
    zIndex: asNumber(record.zIndex, `${path}.zIndex`, code, { integer: true }),
    locked: asBoolean(record.locked, `${path}.locked`, code),
  };

  switch (type) {
    case 'image':
      return { ...base, type, data: parseImageData(record.data, `${path}.data`) };
    case 'panorama':
      return { ...base, type, data: parsePanoramaData(record.data, `${path}.data`) };
    case 'text':
      return { ...base, type, data: parseTextData(record.data, `${path}.data`) };
    case 'config':
      return { ...base, type, data: parseConfigData(record.data, `${path}.data`) };
    case 'video':
      return { ...base, type, data: parseVideoData(record.data, `${path}.data`) };
    case 'audio':
      return { ...base, type, data: parseAudioData(record.data, `${path}.data`) };
    case 'director':
      return { ...base, type, data: parseDirectorData(record.data, `${path}.data`) };
    case 'group':
      return { ...base, type, data: parseGroupData(record.data, `${path}.data`) };
  }
};

const parseConnection = (value: unknown, path: string): CreativeCanvasConnection => {
  const code = 'INVALID_DOCUMENT';
  const record = asRecord(value, path, code);
  exactKeys(record, ['id', 'sourceNodeId', 'targetNodeId', 'sourceHandle', 'targetHandle'], [], path, code);
  return {
    id: asId(record.id, `${path}.id`, code),
    sourceNodeId: asId(record.sourceNodeId, `${path}.sourceNodeId`, code),
    targetNodeId: asId(record.targetNodeId, `${path}.targetNodeId`, code),
    sourceHandle: asNullableId(record.sourceHandle, `${path}.sourceHandle`, code),
    targetHandle: asNullableId(record.targetHandle, `${path}.targetHandle`, code),
  };
};

const parseChatSession = (value: unknown, path: string): CreativeChatSessionReference => {
  const code = 'INVALID_DOCUMENT';
  const record = asRecord(value, path, code);
  exactKeys(
    record,
    ['id', 'title', 'messageIds', 'model', 'pendingTurn', 'createdAt', 'updatedAt'],
    [],
    path,
    code
  );
  const messageIds = asArray(record.messageIds, `${path}.messageIds`, code, (item, itemPath) =>
    asUuidV7Id(item, itemPath, code)
  );
  assertUnique(messageIds, `${path}.messageIds`, code);
  if (messageIds.length % 2 !== 0) {
    fail(code, `${path}.messageIds`, 'completed user/assistant id pairs');
  }
  let model: CreativeChatModelReference | null = null;
  if (record.model !== null) {
    const modelRecord = asRecord(record.model, `${path}.model`, code);
    exactKeys(modelRecord, ['providerId', 'model'], [], `${path}.model`, code);
    const providerId = asUuidV7Id(modelRecord.providerId, `${path}.model.providerId`, code);
    const modelId = asString(modelRecord.model, `${path}.model.model`, code, { maxLength: 512 });
    if (modelId !== modelId.trim()) fail(code, `${path}.model.model`, 'trimmed model id');
    model = { providerId, model: modelId };
  }
  let pendingTurn: CreativeChatPendingTurn | null = null;
  if (record.pendingTurn !== null) {
    const pendingRecord = asRecord(record.pendingTurn, `${path}.pendingTurn`, code);
    exactKeys(
      pendingRecord,
      ['idempotencyKey', 'prompt', 'createdAt'],
      ['modelInput', 'skillIds'],
      `${path}.pendingTurn`,
      code
    );
    const prompt = asString(pendingRecord.prompt, `${path}.pendingTurn.prompt`, code, {
      maxLength: 65_536,
    });
    if (prompt !== prompt.trim()) {
      fail(code, `${path}.pendingTurn.prompt`, 'trimmed non-empty prompt');
    }
    const hasModelInput = Object.prototype.hasOwnProperty.call(pendingRecord, 'modelInput');
    const modelInput =
      !hasModelInput || pendingRecord.modelInput === null
        ? prompt
        : asString(pendingRecord.modelInput, `${path}.pendingTurn.modelInput`, code, {
            maxLength: 262_144,
          });
    if (modelInput !== modelInput.trim()) {
      fail(code, `${path}.pendingTurn.modelInput`, 'trimmed non-empty model input');
    }
    let skillIds: string[] = [];
    if (Object.prototype.hasOwnProperty.call(pendingRecord, 'skillIds')) {
      const rawSkillIds = Array.isArray(pendingRecord.skillIds)
        ? pendingRecord.skillIds
        : fail(code, `${path}.pendingTurn.skillIds`, 'array');
      if (rawSkillIds.length > 8) {
        fail(code, `${path}.pendingTurn.skillIds`, 'array with at most 8 skill ids');
      }
      skillIds = asArray(
        rawSkillIds,
        `${path}.pendingTurn.skillIds`,
        code,
        (item, itemPath) => {
          const skillId = asString(item, itemPath, code, { maxLength: 128 });
          if (skillId !== skillId.trim() || !/^[A-Za-z0-9._-]+$/.test(skillId)) {
            fail(code, itemPath, 'trimmed ASCII skill id matching [A-Za-z0-9._-]');
          }
          return skillId;
        }
      );
      assertUnique(skillIds, `${path}.pendingTurn.skillIds`, code);
    }
    pendingTurn = {
      idempotencyKey: asUuidV7Id(
        pendingRecord.idempotencyKey,
        `${path}.pendingTurn.idempotencyKey`,
        code
      ),
      prompt,
      modelInput,
      skillIds,
      createdAt: asNumber(pendingRecord.createdAt, `${path}.pendingTurn.createdAt`, code, {
        min: 0,
        integer: true,
      }),
    };
  }
  if ((messageIds.length > 0 || pendingTurn !== null) && model === null) {
    fail(code, `${path}.model`, 'selected model for persisted or pending Agent turns');
  }
  return {
    id: asUuidV7Id(record.id, `${path}.id`, code),
    title: asString(record.title, `${path}.title`, code, { maxLength: 1_000 }),
    messageIds,
    model,
    pendingTurn,
    createdAt: asNumber(record.createdAt, `${path}.createdAt`, code, { min: 0, integer: true }),
    updatedAt: asNumber(record.updatedAt, `${path}.updatedAt`, code, { min: 0, integer: true }),
  };
};

const parsePanels = (value: unknown, path: string): CreativeStudioPanelState => {
  const code = 'INVALID_DOCUMENT';
  const record = asRecord(value, path, code);
  exactKeys(record, ['left', 'right', 'bottom'], [], path, code);
  const left = asRecord(record.left, `${path}.left`, code);
  const right = asRecord(record.right, `${path}.right`, code);
  const bottom = asRecord(record.bottom, `${path}.bottom`, code);
  exactKeys(left, ['open', 'width', 'activeView'], [], `${path}.left`, code);
  exactKeys(right, ['open', 'width', 'activeView'], [], `${path}.right`, code);
  exactKeys(bottom, ['open', 'height', 'activeView'], [], `${path}.bottom`, code);
  return {
    left: {
      open: asBoolean(left.open, `${path}.left.open`, code),
      width: asNumber(left.width, `${path}.left.width`, code, { min: 180, max: 800 }),
      activeView: asLiteral<CreativeLeftPanelView>(
        left.activeView,
        ['canvas', 'assets', 'prompts', 'templates'],
        `${path}.left.activeView`,
        code
      ),
    },
    right: {
      open: asBoolean(right.open, `${path}.right.open`, code),
      width: asNumber(right.width, `${path}.right.width`, code, { min: 240, max: 960 }),
      activeView: asLiteral<CreativeRightPanelView>(
        right.activeView,
        ['assistant', 'properties'],
        `${path}.right.activeView`,
        code
      ),
    },
    bottom: {
      open: asBoolean(bottom.open, `${path}.bottom.open`, code),
      height: asNumber(bottom.height, `${path}.bottom.height`, code, { min: 120, max: 800 }),
      activeView: asLiteral<CreativeBottomPanelView>(
        bottom.activeView,
        ['timeline', 'history'],
        `${path}.bottom.activeView`,
        code
      ),
    },
  };
};

const asRevision = (
  value: unknown,
  path: string,
  code: CreativeStudioContractErrorCode
): string => {
  const revision = asString(value, path, code, { maxLength: 40 });
  if (!/^(0|[1-9]\d*)$/.test(revision)) return fail(code, path, 'decimal revision string');
  return revision;
};

const parseSummaryAt = (
  value: unknown,
  path: string,
  code: CreativeStudioContractErrorCode
): CreativeProjectSummary => {
  const record = asRecord(value, path, code);
  exactKeys(
    record,
    ['projectId', 'title', 'revision', 'nodeCount', 'connectionCount', 'createdAt', 'updatedAt'],
    [],
    path,
    code
  );
  return {
    projectId: asProjectId(record.projectId, `${path}.projectId`, code),
    title: asString(record.title, `${path}.title`, code, { maxLength: 1_000 }),
    revision: asRevision(record.revision, `${path}.revision`, code),
    nodeCount: asNumber(record.nodeCount, `${path}.nodeCount`, code, { min: 0, integer: true }),
    connectionCount: asNumber(record.connectionCount, `${path}.connectionCount`, code, {
      min: 0,
      integer: true,
    }),
    createdAt: asNumber(record.createdAt, `${path}.createdAt`, code, { min: 0, integer: true }),
    updatedAt: asNumber(record.updatedAt, `${path}.updatedAt`, code, { min: 0, integer: true }),
  };
};

export function parseCreativeProjectSummary(value: unknown): CreativeProjectSummary {
  return parseSummaryAt(value, '$', 'INVALID_RESPONSE');
}

export function parseCreativeProjectDocument(
  value: unknown,
  expectedProjectId?: string
): CreativeProjectDocument {
  const code = 'INVALID_DOCUMENT';
  const record = asRecord(value, '$', code);
  exactKeys(
    record,
    [
      'schema',
      'projectId',
      'viewport',
      'background',
      'nodes',
      'connections',
      'chatSessions',
      'activeChatId',
      'panels',
      'pendingTaskIds',
    ],
    [],
    '$',
    code
  );
  if (record.schema !== CREATIVE_STUDIO_DOCUMENT_SCHEMA) {
    fail('SCHEMA_MISMATCH', '$.schema', JSON.stringify(CREATIVE_STUDIO_DOCUMENT_SCHEMA));
  }
  const projectId = asProjectId(record.projectId, '$.projectId', code);
  if (expectedProjectId !== undefined && projectId !== expectedProjectId) {
    fail('PROJECT_MISMATCH', '$.projectId', JSON.stringify(expectedProjectId));
  }
  const viewport = asRecord(record.viewport, '$.viewport', code);
  exactKeys(viewport, ['x', 'y', 'zoom'], [], '$.viewport', code);
  const nodes = asArray(record.nodes, '$.nodes', code, parseNode);
  const connections = asArray(record.connections, '$.connections', code, parseConnection);
  const chatSessions = asArray(record.chatSessions, '$.chatSessions', code, parseChatSession);
  const pendingTaskIds = asIdArray(record.pendingTaskIds, '$.pendingTaskIds', code);
  const nodeIds = nodes.map((node) => node.id);
  const nodeById = new Map(nodes.map((node) => [node.id, node]));
  const chatIds = chatSessions.map((chat) => chat.id);
  assertUnique(nodeIds, '$.nodes[].id', code);
  assertUnique(connections.map((connection) => connection.id), '$.connections[].id', code);
  assertUnique(chatIds, '$.chatSessions[].id', code);
  assertUnique(pendingTaskIds, '$.pendingTaskIds', code);
  for (const [index, node] of nodes.entries()) {
    if (node.groupId === null) continue;
    const group = nodeById.get(node.groupId);
    if (node.type === 'group' || !group || group.type !== 'group' || group.id === node.id) {
      fail(code, `$.nodes[${index}].groupId`, 'id of another group node');
    }
  }
  const directedConnections = new Set<string>();
  for (const [index, connection] of connections.entries()) {
    const source =
      nodeById.get(connection.sourceNodeId) ??
      fail(code, `$.connections[${index}].sourceNodeId`, 'existing node id');
    const target =
      nodeById.get(connection.targetNodeId) ??
      fail(code, `$.connections[${index}].targetNodeId`, 'existing node id');
    if (source.id === target.id) {
      fail(code, `$.connections[${index}].targetNodeId`, 'node id different from sourceNodeId');
    }
    const directedKey = `${source.id}\u0000${target.id}`;
    if (directedConnections.has(directedKey)) {
      fail(code, `$.connections[${index}]`, 'unique directed node pair');
    }
    directedConnections.add(directedKey);
    if (source.type === 'group' || target.type === 'group') {
      fail(code, `$.connections[${index}]`, 'connection between non-group nodes');
    }
    if (source.type === 'config' && target.type === 'config') {
      fail(code, `$.connections[${index}]`, 'connection other than config to config');
    }
    if (source.type === 'director') {
      fail(code, `$.connections[${index}].sourceNodeId`, 'non-director source node');
    }
    if (target.type === 'director' && source.type !== 'image' && source.type !== 'panorama') {
      fail(code, `$.connections[${index}].sourceNodeId`, 'image or panorama source for director');
    }
  }
  const pendingChatSessions: CreativeChatSessionReference[] = [];
  for (const [index, chat] of chatSessions.entries()) {
    if (chat.updatedAt < chat.createdAt) {
      fail(code, `$.chatSessions[${index}].updatedAt`, 'timestamp not earlier than createdAt');
    }
    if (chat.pendingTurn) {
      if (chat.pendingTurn.createdAt < chat.createdAt || chat.pendingTurn.createdAt > chat.updatedAt) {
        fail(
          code,
          `$.chatSessions[${index}].pendingTurn.createdAt`,
          'timestamp within the owning chat session lifetime'
        );
      }
      pendingChatSessions.push(chat);
    }
  }
  const activeChatId =
    record.activeChatId === null
      ? null
      : asUuidV7Id(record.activeChatId, '$.activeChatId', code);
  if (activeChatId !== null && !chatIds.includes(activeChatId)) {
    fail(code, '$.activeChatId', 'existing chat session id or null');
  }
  if (pendingChatSessions.length > 1) {
    fail(code, '$.chatSessions', 'at most one pending Agent turn');
  }
  if (pendingChatSessions.length === 1 && pendingChatSessions[0]?.id !== activeChatId) {
    fail(code, '$.activeChatId', 'the session owning the pending Agent turn');
  }
  return {
    schema: CREATIVE_STUDIO_DOCUMENT_SCHEMA,
    projectId,
    viewport: {
      x: asNumber(viewport.x, '$.viewport.x', code),
      y: asNumber(viewport.y, '$.viewport.y', code),
      zoom: asNumber(viewport.zoom, '$.viewport.zoom', code, { min: 0.05, max: 5 }),
    },
    background: asLiteral<CreativeCanvasBackground>(
      record.background,
      ['dots', 'lines', 'blank'],
      '$.background',
      code
    ),
    nodes,
    connections,
    chatSessions,
    activeChatId,
    panels: parsePanels(record.panels, '$.panels'),
    pendingTaskIds,
  };
}

export function parseCreativeProjectListResponse(value: unknown): CreativeProjectListResponse {
  const code = 'INVALID_RESPONSE';
  const record = asRecord(value, '$', code);
  exactKeys(record, ['projects'], [], '$', code);
  const projects = asArray(record.projects, '$.projects', code, (entry, path) => parseSummaryAt(entry, path, code));
  assertUnique(projects.map((project) => project.projectId), '$.projects[].projectId', code);
  return { projects };
}

export function parseCreativeProjectResponse(value: unknown): CreativeProjectResponse {
  const code = 'INVALID_RESPONSE';
  const record = asRecord(value, '$', code);
  exactKeys(record, ['project'], [], '$', code);
  return { project: parseSummaryAt(record.project, '$.project', code) };
}

export function parseCreativeProjectDetailResponse(value: unknown): CreativeProjectDetail {
  const code = 'INVALID_RESPONSE';
  const record = asRecord(value, '$', code);
  exactKeys(record, ['project', 'document'], [], '$', code);
  const project = parseSummaryAt(record.project, '$.project', code);
  const document = parseCreativeProjectDocument(record.document, project.projectId);
  if (project.nodeCount !== document.nodes.length) {
    fail(code, '$.project.nodeCount', 'document.nodes.length');
  }
  if (project.connectionCount !== document.connections.length) {
    fail(code, '$.project.connectionCount', 'document.connections.length');
  }
  return { project, document };
}

export function parseCreateCreativeProjectRequest(value: unknown): CreateCreativeProjectRequest {
  const code = 'INVALID_REQUEST';
  const record = asRecord(value, '$', code);
  exactKeys(record, [], ['title', 'agentKickoff'], '$', code);
  const request: CreateCreativeProjectRequest = {};

  if (record.title !== undefined) {
    request.title = asString(record.title, '$.title', code, { maxLength: 1_000 });
  }
  if (record.agentKickoff !== undefined) {
    const kickoff = asRecord(record.agentKickoff, '$.agentKickoff', code);
    exactKeys(kickoff, ['prompt', 'model'], [], '$.agentKickoff', code);
    const prompt = asString(kickoff.prompt, '$.agentKickoff.prompt', code, {
      maxLength: 65_536,
    }).trim();
    if (!prompt) fail(code, '$.agentKickoff.prompt', 'trimmed non-empty string <= 65536 chars');

    const model = asRecord(kickoff.model, '$.agentKickoff.model', code);
    exactKeys(model, ['providerId', 'model'], [], '$.agentKickoff.model', code);
    const modelName = asString(model.model, '$.agentKickoff.model.model', code, {
      maxLength: 512,
    });
    if (modelName !== modelName.trim()) {
      fail(code, '$.agentKickoff.model.model', 'trimmed non-empty string <= 512 chars');
    }
    request.agentKickoff = {
      prompt,
      model: {
        providerId: asUuidV7Id(
          model.providerId,
          '$.agentKickoff.model.providerId',
          code
        ),
        model: modelName,
      },
    };
  }

  return request;
}

export function parseRenameCreativeProjectRequest(value: unknown): RenameCreativeProjectRequest {
  const code = 'INVALID_REQUEST';
  const record = asRecord(value, '$', code);
  exactKeys(record, ['title'], [], '$', code);
  return { title: asString(record.title, '$.title', code, { maxLength: 1_000 }) };
}

export function parseSaveCreativeProjectRequest(
  value: unknown,
  expectedProjectId?: string
): SaveCreativeProjectRequest {
  const code = 'INVALID_REQUEST';
  const record = asRecord(value, '$', code);
  exactKeys(record, ['expectedRevision', 'document'], [], '$', code);
  return {
    expectedRevision: asRevision(record.expectedRevision, '$.expectedRevision', code),
    document: parseCreativeProjectDocument(record.document, expectedProjectId),
  };
}
