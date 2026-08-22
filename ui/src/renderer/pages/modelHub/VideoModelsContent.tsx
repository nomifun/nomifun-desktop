/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import ModalityModelsPanel from './ModalityModelsPanel';

const VideoModelsContent: React.FC = () => (
  <ModalityModelsPanel
    modality='video'
    titleKey='settings.modelHub.creation.videoTitle'
    subtitleKey='settings.modelHub.creation.videoSubtitle'
  />
);

export default VideoModelsContent;
