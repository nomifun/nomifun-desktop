/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import ModalityModelsPanel from './ModalityModelsPanel';

const ImageEditModelsContent: React.FC = () => (
  <ModalityModelsPanel
    modality='image_edit'
    titleKey='settings.modelHub.modality.imageEditTitle'
    subtitleKey='settings.modelHub.modality.imageEditSubtitle'
  />
);

export default ImageEditModelsContent;
