/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { VideoTwo } from '@icon-park/react';
import ModalityModelsPanel from './ModalityModelsPanel';

const VideoModelsContent: React.FC = () => (
  <ModalityModelsPanel
    modality='video'
    icon={<VideoTwo theme='outline' size='18' strokeWidth={3} />}
    titleKey='settings.modelHub.creation.videoTitle'
    subtitleKey='settings.modelHub.creation.videoSubtitle'
  />
);

export default VideoModelsContent;
