/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { SafeRetrieval } from '@icon-park/react';
import ModalityModelsPanel from './ModalityModelsPanel';

const RerankModelsContent: React.FC = () => (
  <ModalityModelsPanel
    modality='rerank'
    icon={<SafeRetrieval theme='outline' size='18' strokeWidth={3} />}
    titleKey='settings.modelHub.modality.rerankTitle'
    subtitleKey='settings.modelHub.modality.rerankSubtitle'
  />
);

export default RerankModelsContent;
