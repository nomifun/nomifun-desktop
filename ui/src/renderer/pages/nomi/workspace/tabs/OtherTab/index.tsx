/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect } from 'react';
import { Spin } from '@arco-design/web-react';
import type { WorkspaceTabProps } from '../../types';
import MigrationSection from './MigrationSection';
import DangerZoneSection from './DangerZoneSection';

/**
 * 其他 — the rare, careful drawer: move this companion between machines, or
 * destroy it. Two sections only, and nothing here fires without a confirm or a
 * native file dialog. No detail pane: there is no list to scan alongside.
 *
 * The tab never raises the attention dot — nothing in it is ever "waiting".
 */
const OtherTab: React.FC<WorkspaceTabProps> = ({ companionId, companion, onAttentionChange }) => {
  const { profile } = companion;

  useEffect(() => {
    onAttentionChange?.(false);
  }, [onAttentionChange]);

  if (!profile) {
    return (
      <div className='flex justify-center py-40px'>
        <Spin />
      </div>
    );
  }

  return (
    <div className='flex flex-col gap-16px py-8px'>
      <MigrationSection companionId={companionId} companionName={profile.name} />
      <DangerZoneSection companionId={companionId} companionName={profile.name} />
    </div>
  );
};

export default OtherTab;
