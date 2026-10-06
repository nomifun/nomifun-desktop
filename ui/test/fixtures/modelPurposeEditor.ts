import type { ModelTask } from '../../src/common/protocolBindings/ModelTask';
import type { ModelProtocolManifestMap } from '../../src/renderer/pages/settings/components/providerModelAdvanced';
import { aliasProtocolManifest } from './modelAliasEditor';

const audioManifest = (task: ModelTask, protocol: string, endpoint: string) => ({
  ...aliasProtocolManifest,
  requested_task: task,
  recommendation: { ...aliasProtocolManifest.recommendation!, protocol_id: protocol },
  protocols: [{ ...aliasProtocolManifest.protocols[0], protocol_id: protocol, supported_tasks: [task],
    endpoints: [{ ...aliasProtocolManifest.protocols[0].endpoints[0], task, default_value: endpoint }] }],
});

export const purposeManifests: ModelProtocolManifestMap = {
  chat: aliasProtocolManifest,
  speech_recognition: audioManifest('speech_recognition', 'openai.audio_transcriptions', '/audio/transcriptions'),
  speech_synthesis: audioManifest('speech_synthesis', 'openai.audio_speech', '/audio/speech'),
  image_generation: audioManifest('image_generation', 'openai.images', '/images/generations'),
};
