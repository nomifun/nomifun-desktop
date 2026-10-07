import type { IProvider } from '../../src/common/config/storage';
import type { ModelTask } from '../../src/common/protocolBindings/ModelTask';
import type { ProviderModelCapabilityResponse } from '../../src/common/types/provider/providerModel';
import type { ProviderConnectionResponse } from '../../src/common/types/provider/providerConnection';
import type { ModelProtocolManifestMap } from '../../src/renderer/pages/settings/components/providerModelAdvanced';
import { aliasProtocolManifest, aliasProviderBaseUrl, aliasProviderId } from './modelAliasEditor';

export const scenarioModelId = 'scenario-model-id';
export const scenarioProviderId = aliasProviderId;
export const scenarioProviderBaseUrl = aliasProviderBaseUrl;

const taskRoutes: Array<{ task: ModelTask; protocol: string; endpoint: string }> = [
  { task: 'chat', protocol: 'openai.chat_text', endpoint: '/chat/completions' },
  { task: 'realtime_conversation', protocol: 'openai.realtime', endpoint: '/realtime' },
  { task: 'speech_recognition', protocol: 'openai.audio_transcriptions', endpoint: '/audio/transcriptions' },
  { task: 'speech_synthesis', protocol: 'openai.audio_speech', endpoint: '/audio/speech' },
  { task: 'image_generation', protocol: 'openai.images', endpoint: '/images/generations' },
  { task: 'image_edit', protocol: 'openai.images_edit', endpoint: '/images/edits' },
  { task: 'video_generation', protocol: 'fixture.video', endpoint: '/videos' },
  { task: 'music_generation', protocol: 'fixture.music', endpoint: '/music' },
  { task: 'embedding', protocol: 'openai.embeddings', endpoint: '/embeddings' },
  { task: 'rerank', protocol: 'fixture.rerank', endpoint: '/rerank' },
];

export const scenarioCapabilities: ProviderModelCapabilityResponse[] = taskRoutes.map(({ task, protocol, endpoint }, index) => ({
  task,
  traits: task === 'chat' ? ['vision_input'] : [],
  protocol,
  connection_role: task === 'speech_recognition' ? 'speech' : 'default',
  endpoint,
  allow_cross_origin_credentials: false,
  provider_params: task === 'chat' ? { temperature: 0.4, reasoning_effort: 'medium' } : { fixture_marker: index },
  context_limit: task === 'chat' ? 64_000 : 8_192 + index,
  output_limit: task === 'chat' ? 4_096 : 1_024 + index,
  ...(task === 'chat' ? { compaction_threshold_pct: 80 } : {}),
  created_at: 11,
  updated_at: 19,
}));

export const scenarioProvider: IProvider = {
  id: scenarioProviderId,
  platform: 'preview',
  name: '场景测试供应商',
  base_url: scenarioProviderBaseUrl,
  auth_scheme: 'bearer',
  has_credentials: false,
  enabled: true,
  models: [{
    provider_id: scenarioProviderId,
    model: scenarioModelId,
    display_name: '多用途模型',
    enabled: false,
    sort_order: 37,
    description: '保留的模型描述',
    capabilities: scenarioCapabilities,
    created_at: 11,
    updated_at: 19,
  }],
};

export const scenarioConnections: ProviderConnectionResponse[] = [{
  connection_id: '0190f5fe-7c00-7a00-8000-000000000212',
  provider_id: scenarioProviderId,
  role: 'speech',
  base_url: scenarioProviderBaseUrl,
  auth_scheme: 'bearer',
  has_credentials: false,
  extra: {},
  created_at: 11,
  updated_at: 19,
}];

export const scenarioManifests: ModelProtocolManifestMap = Object.fromEntries(taskRoutes.map(({ task, protocol, endpoint }) => [task, {
  ...aliasProtocolManifest,
  tasks: [task],
  requested_task: task,
  recommendation: { ...aliasProtocolManifest.recommendation!, protocol_id: protocol },
  protocols: [{
    ...aliasProtocolManifest.protocols[0],
    protocol_id: protocol,
    supported_tasks: [task],
    endpoints: [{ ...aliasProtocolManifest.protocols[0].endpoints[0], task, default_value: endpoint }],
  }],
}]));
