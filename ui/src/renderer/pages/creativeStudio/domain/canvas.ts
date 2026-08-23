/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  createEmptyCreativeProjectDocument,
  type CreativeChatModelReference,
  type CreativeProjectDetail,
  type CreativeProjectDocument,
  type CreativeProjectSummary,
} from './schema';

/** Canonical product document for one persisted Creative Studio canvas. */
export interface CreativeCanvasDocument
  extends Omit<CreativeProjectDocument, 'projectId'> {
  canvasId: string;
}

/** Canonical list and mutation summary for one persisted canvas. */
export interface CreativeCanvasSummary
  extends Omit<CreativeProjectSummary, 'projectId'> {
  canvasId: string;
}

/** Canonical aggregate returned when a canvas document is loaded. */
export interface CreativeCanvasDetail {
  canvas: CreativeCanvasSummary;
  document: CreativeCanvasDocument;
}

export interface CreativeCanvasAgentKickoff {
  prompt: string;
  model: CreativeChatModelReference;
}

export interface CreateCreativeCanvasRequest {
  title?: string;
  agentKickoff?: CreativeCanvasAgentKickoff;
}

export interface RenameCreativeCanvasRequest {
  title: string;
}

export interface SaveCreativeCanvasRequest {
  expectedRevision: string;
  document: CreativeCanvasDocument;
}

export interface CreativeCanvasListResponse {
  canvases: CreativeCanvasSummary[];
}

export interface CreativeCanvasResponse {
  canvas: CreativeCanvasSummary;
}

export interface CreativeCanvasDetailResponse extends CreativeCanvasDetail {}

export type SaveCreativeCanvasResponse = CreativeCanvasResponse;

/** Build the canonical empty-document shape for a newly created canvas. */
export function createEmptyCreativeCanvasDocument(
  canvasId: string
): CreativeCanvasDocument {
  const { projectId: _legacyProjectId, ...document } =
    createEmptyCreativeProjectDocument(canvasId);
  return { ...document, canvasId };
}

/**
 * Convert the canonical product shape to the historical in-process shape.
 *
 * Canvas/editor modules are migrated independently. This adapter is the only
 * place where the retired product name should be reintroduced for them.
 */
export function creativeCanvasDocumentToLegacyProject(
  document: CreativeCanvasDocument
): CreativeProjectDocument {
  const { canvasId, ...legacyDocument } = document;
  return { ...legacyDocument, projectId: canvasId };
}

/** @deprecated Compatibility adapter for canvas/editor modules not migrated yet. */
export function legacyProjectDocumentToCreativeCanvas(
  document: CreativeProjectDocument
): CreativeCanvasDocument {
  const { projectId, ...canvasDocument } = document;
  return { ...canvasDocument, canvasId: projectId };
}

/** @deprecated Compatibility adapter for canvas/editor modules not migrated yet. */
export function creativeCanvasSummaryToLegacyProject(
  canvas: CreativeCanvasSummary
): CreativeProjectSummary {
  const { canvasId, ...legacySummary } = canvas;
  return { ...legacySummary, projectId: canvasId };
}

/** @deprecated Compatibility adapter for canvas/editor modules not migrated yet. */
export function legacyProjectSummaryToCreativeCanvas(
  project: CreativeProjectSummary
): CreativeCanvasSummary {
  const { projectId, ...canvasSummary } = project;
  return { ...canvasSummary, canvasId: projectId };
}

/** @deprecated Compatibility adapter for canvas/editor modules not migrated yet. */
export function creativeCanvasDetailToLegacyProject(
  detail: CreativeCanvasDetail
): CreativeProjectDetail {
  return {
    project: creativeCanvasSummaryToLegacyProject(detail.canvas),
    document: creativeCanvasDocumentToLegacyProject(detail.document),
  };
}

/** @deprecated Compatibility adapter for canvas/editor modules not migrated yet. */
export function legacyProjectDetailToCreativeCanvas(
  detail: CreativeProjectDetail
): CreativeCanvasDetail {
  return {
    canvas: legacyProjectSummaryToCreativeCanvas(detail.project),
    document: legacyProjectDocumentToCreativeCanvas(detail.document),
  };
}
