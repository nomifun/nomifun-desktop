/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/** Stable list-item contract; the canvas document stays behind the project port. */
export interface CreativeStudioProjectSummary {
  id: string;
  title: string;
  /** Unix epoch milliseconds. */
  createdAt: number;
  /** Unix epoch milliseconds. */
  updatedAt: number;
  nodeCount: number;
  connectionCount: number;
}

export type CreativeStudioProjectsLoadState = 'loading' | 'ready' | 'error';

export interface CreativeStudioProjectsSnapshot {
  status: CreativeStudioProjectsLoadState;
  projects: readonly CreativeStudioProjectSummary[];
  error?: string;
}

export interface CreativeStudioProjectArchiveCapabilities {
  canImport: boolean;
  canExport: boolean;
}

/**
 * Product-facing persistence port. Implementations may use HTTP, IPC, or local
 * storage without leaking those choices into the project-center UI.
 */
export interface CreativeStudioProjectsService {
  readonly archiveCapabilities: CreativeStudioProjectArchiveCapabilities;
  listProjects(signal?: AbortSignal): Promise<readonly CreativeStudioProjectSummary[]>;
  createProject(title: string): Promise<CreativeStudioProjectSummary>;
  importProjectArchive(file: File): Promise<readonly CreativeStudioProjectSummary[]>;
  renameProject(id: string, title: string): Promise<CreativeStudioProjectSummary>;
  deleteProjects(ids: readonly string[]): Promise<void>;
  exportProjects(ids: readonly string[]): Promise<void>;
}
