import React from 'react';
import ModalityModelsPanel from './ModalityModelsPanel';

const MusicModelsContent: React.FC = () => (
  <ModalityModelsPanel
    modality='music'
    defaultModelPreferenceKey='models.default.musicGeneration'
    titleKey='settings.modelHub.creation.musicTitle'
    subtitleKey='settings.modelHub.creation.musicSubtitle'
  />
);

export default MusicModelsContent;
