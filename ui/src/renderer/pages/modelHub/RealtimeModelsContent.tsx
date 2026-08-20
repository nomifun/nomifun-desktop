/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import ModalityModelsPanel from './ModalityModelsPanel';

/** Realtime is a standalone task, never a trait-filtered ordinary chat list. */
const RealtimeModelsContent: React.FC = () => (
  <ModalityModelsPanel
    modality='realtime'
    titleKey='settings.modelHub.modality.realtimeTitle'
    subtitleKey='settings.modelHub.modality.realtimeSubtitle'
  />
);

export default RealtimeModelsContent;
