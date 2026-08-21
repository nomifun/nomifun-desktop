/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { CANONICAL_UUID_V7 } from '@/common/types/ids';

import type {
  CreativeStandaloneWorkbenchKind,
  CreativeTaskOwner,
} from '../../tasks';

export const STANDALONE_VIDEO_MAX_CONCURRENT_TASKS = 1;

export type StandaloneProjectQuery =
  | { state: 'missing'; projectId: null }
  | { state: 'invalid'; projectId: null; message: string }
  | { state: 'valid'; projectId: string };

export function parseStandaloneProjectQuery(search: string): StandaloneProjectQuery {
  const params = new URLSearchParams(search);
  const values = params.getAll('projectId');
  if (values.length === 0) return { state: 'missing', projectId: null };
  if (values.length !== 1 || !CANONICAL_UUID_V7.test(values[0] ?? '')) {
    return {
      state: 'invalid',
      projectId: null,
      message: 'projectId 必须是唯一、规范的小写 UUIDv7。',
    };
  }
  return { state: 'valid', projectId: values[0] as string };
}

export function standaloneProjectSearch(search: string, projectId: string | null): string {
  const params = new URLSearchParams(search);
  params.delete('projectId');
  if (projectId) params.set('projectId', projectId);
  const encoded = params.toString();
  return encoded ? `?${encoded}` : '';
}

export function standaloneWorkbenchOwner(
  projectId: string,
  workbenchKind: CreativeStandaloneWorkbenchKind
): CreativeTaskOwner {
  if (!CANONICAL_UUID_V7.test(projectId)) {
    throw new Error('Standalone workbench owner requires a canonical project UUIDv7');
  }
  return {
    kind: 'standalone_workbench',
    projectId,
    workbenchKind,
  };
}
