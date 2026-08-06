/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { SafeRetrieval } from '@icon-park/react';
import ModalityModelsPanel from './ModalityModelsPanel';

/** 嵌入与检索区：embedding + rerank 两个任务的合并投影。 */
const EmbeddingModelsContent: React.FC = () => (
  <ModalityModelsPanel
    modality='embedding'
    icon={<SafeRetrieval theme='outline' size='18' strokeWidth={3} />}
    titleKey='settings.modelHub.modality.embeddingTitle'
    subtitleKey='settings.modelHub.modality.embeddingSubtitle'
  />
);

export default EmbeddingModelsContent;
