/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  creativeCanvasDocumentToLegacyProject,
  legacyProjectDocumentToCreativeCanvas,
  legacyProjectSummaryToCreativeCanvas,
  type CreateCreativeCanvasRequest,
  type CreativeCanvasDetail,
  type CreativeCanvasDocument,
  type CreativeCanvasListResponse,
  type CreativeCanvasResponse,
  type CreativeCanvasSummary,
  type RenameCreativeCanvasRequest,
  type SaveCreativeCanvasRequest,
} from './canvas';
import {
  CreativeStudioContractError,
  isCreativeStudioContractError,
  parseCreateCreativeProjectRequest,
  parseCreativeProjectDetailResponse,
  parseCreativeProjectDocument,
  parseCreativeProjectListResponse,
  parseCreativeProjectResponse,
  parseCreativeProjectSummary,
  parseRenameCreativeProjectRequest,
  parseSaveCreativeProjectRequest,
} from './validation';

type UnknownRecord = Record<string, unknown>;

const isRecord = (value: unknown): value is UnknownRecord =>
  !!value && typeof value === 'object' && !Array.isArray(value);

const canonicalField = (
  value: unknown,
  canonicalKey: string,
  legacyKey: string,
  path: string,
  code: 'INVALID_RESPONSE' | 'INVALID_DOCUMENT'
): UnknownRecord => {
  if (!isRecord(value)) {
    throw new CreativeStudioContractError(code, path, 'object');
  }
  if (!Object.prototype.hasOwnProperty.call(value, canonicalKey)) {
    throw new CreativeStudioContractError(code, `${path}.${canonicalKey}`, 'present');
  }
  if (Object.prototype.hasOwnProperty.call(value, legacyKey)) {
    throw new CreativeStudioContractError(
      code,
      `${path}.${legacyKey}`,
      'no retired product fields'
    );
  }
  const mapped = { ...value, [legacyKey]: value[canonicalKey] };
  delete mapped[canonicalKey];
  return mapped;
};

const remapContractError = (error: unknown): never => {
  if (!isCreativeStudioContractError(error)) throw error;
  throw new CreativeStudioContractError(
    error.code === 'PROJECT_MISMATCH' ? 'CANVAS_MISMATCH' : error.code,
    error.path
      .replaceAll('projectId', 'canvasId')
      .replaceAll('projects', 'canvases')
      .replaceAll('project', 'canvas'),
    error.expected
      .replaceAll('Project', 'Canvas')
      .replaceAll('project', 'canvas')
  );
};

const parseWithCanvasVocabulary = <T>(parse: () => T): T => {
  try {
    return parse();
  } catch (error) {
    return remapContractError(error);
  }
};

const canvasSummaryWireToLegacy = (
  value: unknown,
  path = '$',
  code: 'INVALID_RESPONSE' | 'INVALID_DOCUMENT' = 'INVALID_RESPONSE'
): unknown => canonicalField(value, 'canvasId', 'projectId', path, code);

const canvasDocumentWireToLegacy = (
  value: unknown,
  code: 'INVALID_RESPONSE' | 'INVALID_DOCUMENT' = 'INVALID_DOCUMENT'
): unknown => canonicalField(value, 'canvasId', 'projectId', '$', code);

export function parseCreativeCanvasSummary(value: unknown): CreativeCanvasSummary {
  return parseWithCanvasVocabulary(() =>
    legacyProjectSummaryToCreativeCanvas(
      parseCreativeProjectSummary(canvasSummaryWireToLegacy(value))
    )
  );
}

export function parseCreativeCanvasDocument(
  value: unknown,
  expectedCanvasId?: string
): CreativeCanvasDocument {
  return parseWithCanvasVocabulary(() =>
    legacyProjectDocumentToCreativeCanvas(
      parseCreativeProjectDocument(
        canvasDocumentWireToLegacy(value),
        expectedCanvasId
      )
    )
  );
}

export function parseCreativeCanvasListResponse(
  value: unknown
): CreativeCanvasListResponse {
  return parseWithCanvasVocabulary(() => {
    const record = canonicalField(
      value,
      'canvases',
      'projects',
      '$',
      'INVALID_RESPONSE'
    );
    const entries = Array.isArray(record.projects)
      ? record.projects.map((entry, index) =>
          canvasSummaryWireToLegacy(
            entry,
            `$.canvases[${index}]`,
            'INVALID_RESPONSE'
          )
        )
      : record.projects;
    const parsed = parseCreativeProjectListResponse({
      ...record,
      projects: entries,
    });
    return {
      canvases: parsed.projects.map(legacyProjectSummaryToCreativeCanvas),
    };
  });
}

export function parseCreativeCanvasResponse(value: unknown): CreativeCanvasResponse {
  return parseWithCanvasVocabulary(() => {
    const record = canonicalField(
      value,
      'canvas',
      'project',
      '$',
      'INVALID_RESPONSE'
    );
    const parsed = parseCreativeProjectResponse({
      ...record,
      project: canvasSummaryWireToLegacy(
        record.project,
        '$.canvas',
        'INVALID_RESPONSE'
      ),
    });
    return { canvas: legacyProjectSummaryToCreativeCanvas(parsed.project) };
  });
}

export function parseCreativeCanvasDetailResponse(
  value: unknown
): CreativeCanvasDetail {
  return parseWithCanvasVocabulary(() => {
    const record = canonicalField(
      value,
      'canvas',
      'project',
      '$',
      'INVALID_RESPONSE'
    );
    const parsed = parseCreativeProjectDetailResponse({
      ...record,
      project: canvasSummaryWireToLegacy(
        record.project,
        '$.canvas',
        'INVALID_RESPONSE'
      ),
      document: canvasDocumentWireToLegacy(
        record.document,
        'INVALID_RESPONSE'
      ),
    });
    return {
      canvas: legacyProjectSummaryToCreativeCanvas(parsed.project),
      document: legacyProjectDocumentToCreativeCanvas(parsed.document),
    };
  });
}

export function parseCreateCreativeCanvasRequest(
  value: unknown
): CreateCreativeCanvasRequest {
  return parseWithCanvasVocabulary(() => parseCreateCreativeProjectRequest(value));
}

export function parseRenameCreativeCanvasRequest(
  value: unknown
): RenameCreativeCanvasRequest {
  return parseWithCanvasVocabulary(() => parseRenameCreativeProjectRequest(value));
}

export function parseSaveCreativeCanvasRequest(
  value: unknown,
  expectedCanvasId?: string
): SaveCreativeCanvasRequest {
  return parseWithCanvasVocabulary(() => {
    if (!isRecord(value)) {
      throw new CreativeStudioContractError('INVALID_REQUEST', '$', 'object');
    }
    const record = value;
    const parsed = parseSaveCreativeProjectRequest(
      {
        ...record,
        document: creativeCanvasDocumentToLegacyProject(
          parseCreativeCanvasDocument(record.document, expectedCanvasId)
        ),
      },
      expectedCanvasId
    );
    return {
      expectedRevision: parsed.expectedRevision,
      document: legacyProjectDocumentToCreativeCanvas(parsed.document),
    };
  });
}
