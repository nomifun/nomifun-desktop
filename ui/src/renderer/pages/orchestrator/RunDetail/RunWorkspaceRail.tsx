/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useMemo } from 'react';
import { ipcBridge } from '@/common';
import type { TRun } from '@/common/types/orchestrator/orchestratorTypes';
import WorkspaceRailBody from '@/renderer/pages/conversation/Workspace/WorkspaceRailBody';
import type { WorkspaceSource } from '@/renderer/pages/conversation/Workspace/types';

/**
 * RunWorkspaceRail — the orchestration run's working-directory right rail (run
 * source binding). A thin adapter that maps a run into a source-agnostic
 * {@link WorkspaceSource} and renders the shared {@link WorkspaceRailBody}, so
 * the body itself knows nothing about runs. The run counterpart of
 * `TerminalWorkspaceRail`.
 *
 * Read-only by design (a run's files are an artifact to inspect, not edit from
 * here):
 * - tree listing goes through `orchestrator.runs.getWorkspace` (server resolves
 *   the run's `work_dir`; `..`/depth guards live in `list_workspace_level`);
 * - `lazyChanges: true` — a run's work_dir may be a large arbitrary directory,
 *   so the file-snapshot baseline is deferred until the Changes tab is opened;
 * - no `onSelectFiles` / `onAppendFiles` / `subscribeRefresh` / `upload`
 *   (file-click still opens the preview via the body's `usePreviewContext`).
 *
 * Must render inside a `PreviewProvider` (the run view mounts a run-scoped one
 * wrapping this rail and the preview column) so file-click → preview resolves.
 */
const RunWorkspaceRail: React.FC<{ run: TRun }> = ({ run }) => {
  const workDir = run.work_dir ?? '';
  const runId = run.id;

  const source = useMemo<WorkspaceSource>(
    () => ({
      workspace: workDir,
      tree: {
        key: runId,
        listRoot: (search?: string) =>
          ipcBridge.orchestrator.runs.getWorkspace.invoke({ id: runId, work_dir: workDir, path: workDir, search }),
        listChildren: (node: { fullPath: string; relativePath: string }) =>
          ipcBridge.orchestrator.runs.getWorkspace.invoke({ id: runId, work_dir: workDir, path: node.fullPath }),
      },
      // A run's work_dir is an arbitrary directory; defer the snapshot baseline
      // until the Changes tab is first opened (see WorkspaceSource.lazyChanges).
      lazyChanges: true,
      isTemporary: false,
      // Intentionally omit onSelectFiles / onAppendFiles / subscribeRefresh /
      // upload — this rail is a read-only artifact viewer.
    }),
    [runId, workDir]
  );

  return <WorkspaceRailBody source={source} />;
};

export default RunWorkspaceRail;
