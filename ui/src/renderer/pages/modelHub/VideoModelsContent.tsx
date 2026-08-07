/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import CreationModelsPanel from './CreationModelsPanel';

/** 视频生成区：能出视频的模型（video_generation 投影）。 */
const VideoModelsContent: React.FC = () => (
  <CreationModelsPanel
    capability='video_generation'
    titleKey='settings.modelHub.creation.videoTitle'
    subtitleKey='settings.modelHub.creation.videoSubtitle'
  />
);

export default VideoModelsContent;
