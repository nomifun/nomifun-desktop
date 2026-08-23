/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  CreativeStudioContractError,
  type CreativeStudioContractErrorCode,
} from '../../domain';
import { assertStrictJsonWithoutDuplicateKeys } from '../../canvas/product/agent/artifacts';

export const CREATIVE_TEMPLATE_DRAFT_ARTIFACT_KIND =
  'nomifun.creative-studio.template-draft/v1' as const;
export const MAX_CREATIVE_TEMPLATE_DRAFT_JSON_BYTES = 262_144;

export type CreativeTemplateDraftMode = 'single-image' | 'multi-image-series';

export interface CreativeTemplateDraft {
  mode: CreativeTemplateDraftMode;
  name: string;
  description: string;
  category: string;
  promptTemplate: string;
}

export interface CreativeTemplateDraftArtifact {
  kind: typeof CREATIVE_TEMPLATE_DRAFT_ARTIFACT_KIND;
  summary: string;
  draft: CreativeTemplateDraft;
}

type UnknownRecord = Record<string, unknown>;

const CONTROL = /[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/u;

const fail = (
  path: string,
  expected: string,
  code: CreativeStudioContractErrorCode = 'INVALID_RESPONSE'
): never => {
  throw new CreativeStudioContractError(code, path, expected);
};

const asRecord = (value: unknown, path: string): UnknownRecord => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    return fail(path, 'object');
  }
  return value as UnknownRecord;
};

const exactKeys = (
  value: UnknownRecord,
  required: readonly string[],
  path: string
): void => {
  for (const key of required) {
    if (!Object.prototype.hasOwnProperty.call(value, key)) {
      fail(`${path}.${key}`, 'present');
    }
  }
  const allowed = new Set(required);
  for (const key of Object.keys(value)) {
    if (!allowed.has(key)) fail(`${path}.${key}`, 'no unknown fields');
  }
};

const assertPairedSurrogates = (value: string, path: string): void => {
  for (let index = 0; index < value.length; index += 1) {
    const unit = value.charCodeAt(index);
    if (unit >= 0xd800 && unit <= 0xdbff) {
      const next = value.charCodeAt(index + 1);
      if (!(next >= 0xdc00 && next <= 0xdfff)) {
        fail(path, 'Unicode string without unpaired surrogates');
      }
      index += 1;
    } else if (unit >= 0xdc00 && unit <= 0xdfff) {
      fail(path, 'Unicode string without unpaired surrogates');
    }
  }
};

const asTrimmedString = (
  value: unknown,
  path: string,
  maximum: number,
  allowEmpty = false
): string => {
  if (typeof value !== 'string') return fail(path, 'string');
  assertPairedSurrogates(value, path);
  if (
    value !== value.trim() ||
    value.length > maximum ||
    (!allowEmpty && value.length === 0) ||
    CONTROL.test(value)
  ) {
    return fail(
      path,
      `${allowEmpty ? '' : 'non-empty '}trimmed string <= ${maximum} characters`
    );
  }
  return value;
};

const parseDraft = (value: unknown): CreativeTemplateDraft => {
  const draft = asRecord(value, '$.draft');
  exactKeys(
    draft,
    ['mode', 'name', 'description', 'category', 'promptTemplate'],
    '$.draft'
  );
  const mode =
    draft.mode === 'single-image' || draft.mode === 'multi-image-series'
      ? draft.mode
      : fail('$.draft.mode', 'single-image | multi-image-series');
  return {
    mode,
    name: asTrimmedString(draft.name, '$.draft.name', 120),
    description: asTrimmedString(draft.description, '$.draft.description', 2_000, true),
    category: asTrimmedString(draft.category, '$.draft.category', 80, true),
    promptTemplate: asTrimmedString(
      draft.promptTemplate,
      '$.draft.promptTemplate',
      200_000
    ),
  };
};

const artifactFromValue = (value: unknown): CreativeTemplateDraftArtifact | null => {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
  const artifact = value as UnknownRecord;
  if (artifact.kind !== CREATIVE_TEMPLATE_DRAFT_ARTIFACT_KIND) return null;
  exactKeys(artifact, ['kind', 'summary', 'draft'], '$');
  return {
    kind: CREATIVE_TEMPLATE_DRAFT_ARTIFACT_KIND,
    summary: asTrimmedString(artifact.summary, '$.summary', 500),
    draft: parseDraft(artifact.draft),
  };
};

/**
 * Parse only one unique, final lowercase-json fence. Ordinary conversation and
 * artifacts owned by another feature remain assistant text and return null.
 */
export function parseCreativeTemplateDraftArtifact(
  text: string
): CreativeTemplateDraftArtifact | null {
  const candidate = text.trimEnd();
  const opening = '```json\n';
  const closing = '\n```';
  const openingIndex = candidate.lastIndexOf(opening);
  const hasCanonicalOpening =
    openingIndex >= 0 && (openingIndex === 0 || candidate[openingIndex - 1] === '\n');

  if (!hasCanonicalOpening || !candidate.endsWith(closing)) {
    if (
      text.includes('```') &&
      text.includes(CREATIVE_TEMPLATE_DRAFT_ARTIFACT_KIND)
    ) {
      return fail('$', 'one final lowercase json template-draft fence');
    }
    return null;
  }

  const jsonText = candidate.slice(openingIndex + opening.length, -closing.length);
  if (
    jsonText.length > MAX_CREATIVE_TEMPLATE_DRAFT_JSON_BYTES ||
    new TextEncoder().encode(jsonText).byteLength >
      MAX_CREATIVE_TEMPLATE_DRAFT_JSON_BYTES
  ) {
    return fail(
      '$',
      `template-draft JSON <= ${MAX_CREATIVE_TEMPLATE_DRAFT_JSON_BYTES} UTF-8 bytes`
    );
  }

  let decoded: unknown;
  try {
    decoded = JSON.parse(jsonText) as unknown;
  } catch {
    if (jsonText.includes(CREATIVE_TEMPLATE_DRAFT_ARTIFACT_KIND)) {
      return fail('$', 'well-formed template-draft JSON');
    }
    return null;
  }

  const decodedKind =
    decoded && typeof decoded === 'object' && !Array.isArray(decoded)
      ? (decoded as UnknownRecord).kind
      : undefined;
  try {
    assertStrictJsonWithoutDuplicateKeys(jsonText);
  } catch {
    if (
      decodedKind === CREATIVE_TEMPLATE_DRAFT_ARTIFACT_KIND ||
      jsonText.includes(CREATIVE_TEMPLATE_DRAFT_ARTIFACT_KIND)
    ) {
      return fail('$', 'JSON without duplicate decoded object keys');
    }
    return null;
  }

  if (decodedKind !== CREATIVE_TEMPLATE_DRAFT_ARTIFACT_KIND) return null;
  if ((candidate.match(/```/g)?.length ?? 0) !== 2) {
    return fail('$', 'one final lowercase json template-draft fence');
  }
  return artifactFromValue(decoded);
}
