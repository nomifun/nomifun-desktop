/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import ModalityModelsPanel from './ModalityModelsPanel';

/** Image generation is independent from image editing. */
const ImageModelsContent: React.FC = () => (
  <ModalityModelsPanel
    modality='image'
    titleKey='settings.modelHub.creation.imageTitle'
    subtitleKey='settings.modelHub.creation.imageSubtitle'
    defaultModelPreferenceKey='models.default.imageGeneration'
  />
);

export default ImageModelsContent;
