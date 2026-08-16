/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import React from 'react';
import LocalAgents from '@/renderer/pages/settings/AgentSettings/LocalAgents';
import NomiScrollArea from '@/renderer/components/base/NomiScrollArea';

/**
 * Execution-engine settings. With the native `nomi` runtime as the only engine
 * there is a single surface here, so the former local/runtime tab strip is gone
 * — the runtime tab only ever hosted the retired engine's timeout sliders.
 */
const AgentModalContent: React.FC = () => {
  const [, agentMessageContext] = useArcoMessage({ maxCount: 10 });

  return (
    <div className='flex flex-col h-full w-full'>
      {agentMessageContext}
      <NomiScrollArea className='flex-1 min-h-0 pb-16px scrollbar-hide' disableOverflow>
        <LocalAgents />
      </NomiScrollArea>
    </div>
  );
};

export default AgentModalContent;
