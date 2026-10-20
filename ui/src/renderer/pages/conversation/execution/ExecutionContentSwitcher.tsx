/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useExecutionSafe } from './ExecutionContext';
import ProjectedAttemptView from './ProjectedAttemptView';
import { executionProjectionKey } from './projectedAttemptSelection';

type ConversationContentSwitcherProps = {
  /** The lead conversation stays mounted while a task transcript is projected. */
  children: React.ReactNode;
};

/**
 * Projects a selected task transcript into the main content area while keeping
 * the lead conversation mounted, preserving its stream and scroll state.
 */
const ExecutionContentSwitcher: React.FC<ConversationContentSwitcherProps> = ({ children }) => {
  const execution = useExecutionSafe();

  // Surfaces outside a Conversation route may intentionally render without
  // execution chrome. Ordinary Conversation and companion sessions mount the
  // shared provider at their outer boundary.
  if (!execution) {
    return <>{children}</>;
  }

  const { projectedPayload } = execution;
  const projecting = projectedPayload !== null;

  return (
    <div className='relative flex flex-1 flex-col min-h-0'>
      {/* Main agent content — ALWAYS mounted; only hidden while projecting so
          scroll + input draft are preserved (never conditionally unmounted). */}
      <div className='flex flex-1 flex-col min-h-0' style={projecting ? { display: 'none' } : undefined} aria-hidden={projecting}>
        {children}
      </div>

      {/* A semantic step edit creates a new immutable backend id. Preserve this
          projection key across that replacement, while a genuinely different
          selected task still remounts with clean task-local drafts. */}
      {projecting && projectedPayload && (
        <ProjectedAttemptView
          key={executionProjectionKey(projectedPayload)}
          payload={projectedPayload}
        />
      )}
    </div>
  );
};

export default ExecutionContentSwitcher;
