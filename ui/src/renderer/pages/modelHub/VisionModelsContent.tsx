/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import ModalityModelsPanel from './ModalityModelsPanel';

/** 视觉区：带 vision_input trait 的 chat 模型投影（视觉不是独立 ModelTask）。 */
const VisionModelsContent: React.FC = () => (
  <ModalityModelsPanel
    modality='vision'
    titleKey='settings.modelHub.modality.visionTitle'
    subtitleKey='settings.modelHub.modality.visionSubtitle'
  />
);

export default VisionModelsContent;
