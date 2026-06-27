/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { Suspense, useState } from 'react';
import AppLoader from '@/renderer/components/layout/AppLoader';
import WorkerTranscriptPanel from './WorkerTranscriptPanel';
import type { OpenTaskPayload } from './DagCanvas';

// The DAG canvas pulls in react-flow (heavy) and is only mounted while the
// 「编排」rail tab is active, so it is split into its own chunk and loaded on
// demand — same lazy import the standalone orchestrator page uses.
const DagCanvas = React.lazy(() => import('./DagCanvas'));

/**
 * DagRailTab — embeds the run's interactive DAG canvas inside a lead
 * conversation's workspace rail (the 「编排」extra tab). Reuses the standalone
 * page's {@link DagCanvas} (in `embedded` mode → no back button, run controls
 * kept) and {@link WorkerTranscriptPanel}; clicking a task node opens the
 * worker's transcript drawer.
 *
 * Sizing: the rail wraps tab content in an `overflow-y-auto` container, but
 * react-flow needs an explicitly-sized, non-scrolling parent or it collapses to
 * zero height. So the root is `h-full min-h-0 overflow-hidden` — it fills the
 * rail's `size-full` slot and neutralizes the inherited scroll, giving the
 * canvas a concrete pixel height.
 */
const DagRailTab: React.FC<{ runId: string }> = ({ runId }) => {
  const [selectedTask, setSelectedTask] = useState<OpenTaskPayload | null>(null);

  return (
    <div className='h-full min-h-0 overflow-hidden flex flex-col'>
      <Suspense fallback={<AppLoader />}>
        <DagCanvas runId={runId} embedded onBack={() => {}} onOpenTask={setSelectedTask} />
      </Suspense>
      <WorkerTranscriptPanel open={selectedTask} onClose={() => setSelectedTask(null)} />
    </div>
  );
};

export default DagRailTab;
