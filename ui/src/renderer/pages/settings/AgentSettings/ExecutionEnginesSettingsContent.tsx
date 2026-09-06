/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import NomiScrollArea from '@/renderer/components/base/NomiScrollArea';
import LocalAgents from './LocalAgents';

/** Runtime Manager content; Agent authoring belongs to the public `/agent` workbench. */
const ExecutionEnginesSettingsContent: React.FC = () => (
  <div className='flex flex-col h-full w-full'>
    <NomiScrollArea className='flex-1 min-h-0 pb-16px scrollbar-hide' disableOverflow>
      <LocalAgents />
    </NomiScrollArea>
  </div>
);

export default ExecutionEnginesSettingsContent;
