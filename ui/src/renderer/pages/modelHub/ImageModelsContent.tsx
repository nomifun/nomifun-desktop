/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import CreationModelsPanel from './CreationModelsPanel';

/** 图像生成区：能出图的模型（image_generation ∪ image_edit 的投影）。 */
const ImageModelsContent: React.FC = () => (
  <CreationModelsPanel
    capability='image_generation'
    titleKey='settings.modelHub.creation.imageTitle'
    subtitleKey='settings.modelHub.creation.imageSubtitle'
    defaultModelPreferenceKey='models.default.imageGeneration'
  />
);

export default ImageModelsContent;
