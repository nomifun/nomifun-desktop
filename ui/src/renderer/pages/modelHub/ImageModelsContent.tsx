/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { Pic } from '@icon-park/react';
import ModalityModelsPanel from './ModalityModelsPanel';

/** Image generation is independent from image editing. */
const ImageModelsContent: React.FC = () => (
  <ModalityModelsPanel
    modality='image'
    icon={<Pic theme='outline' size='18' strokeWidth={3} />}
    titleKey='settings.modelHub.creation.imageTitle'
    subtitleKey='settings.modelHub.creation.imageSubtitle'
  />
);

export default ImageModelsContent;
