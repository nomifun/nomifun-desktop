/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { Comment } from '@icon-park/react';
import ModalityModelsPanel from './ModalityModelsPanel';

/** 对话区：chat 任务的模型投影 + 该模态的全局默认对话模型。 */
const ChatModelsContent: React.FC = () => (
  <ModalityModelsPanel
    modality='chat'
    icon={<Comment theme='outline' size='18' strokeWidth={3} />}
    titleKey='settings.modelHub.modality.chatTitle'
    subtitleKey='settings.modelHub.modality.chatSubtitle'
    showDefaultModel
  />
);

export default ChatModelsContent;
