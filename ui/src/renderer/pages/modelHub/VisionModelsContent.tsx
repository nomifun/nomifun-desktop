/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import ModalityModelsPanel from './ModalityModelsPanel';

/** 视觉区：可编码图片输入的 Chat 协议投影（视觉不是独立 ModelTask）。 */
const VisionModelsContent: React.FC = () => (
  <ModalityModelsPanel
    modality='vision'
    titleKey='settings.modelHub.modality.visionTitle'
    subtitleKey='settings.modelHub.modality.visionSubtitle'
    defaultModelPreferenceKey='models.default.vision'
  />
);

export default VisionModelsContent;
