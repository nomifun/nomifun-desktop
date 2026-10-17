/** Local visual acceptance of the actual shared editor; no credentials or backend writes. */
import React, { useMemo, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { Button, ConfigProvider } from '@arco-design/web-react';
import '@arco-design/web-react/es/_util/react-19-adapter';
import zhCN from '@arco-design/web-react/es/locale/zh-CN';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import 'virtual:uno.css';
import '@arco-design/web-react/dist/css/arco.css';
import '../src/renderer/styles/arco-override.css';
import '../src/renderer/styles/themes/index.css';
import '../src/renderer/styles/modal-contract.css';
import settings from '../src/renderer/services/i18n/locales/zh-CN/settings.json';
import common from '../src/renderer/services/i18n/locales/zh-CN/common.json';
import ModelDefinitionEditor from '../src/renderer/pages/settings/components/ModelDefinitionEditor';
import ModelConfigurationPanel, { ModelConfigurationWorkspace } from '../src/renderer/pages/modelHub/ModelConfigurationPanel';
import { capabilityInputsFromDefinition, createModelDefinitionDraft, validateModelDefinition, type ModelDefinitionDraft } from '../src/renderer/pages/settings/components/providerModelAdvanced';
import type { ModelCatalogSuggestion } from '../src/renderer/pages/settings/components/ModelDefinitionEditor';
import { aliasProviderBaseUrl } from './fixtures/modelAliasEditor';
import { purposeManifests } from './fixtures/modelPurposeEditor';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { settings, common } } }, interpolation: { escapeValue: false } });
const previewParams = new URLSearchParams(window.location.search);
const previewModelReferences = previewParams.get('reference') === 'stepfun';
const manifests = previewModelReferences ? { ...purposeManifests, chat: {
  ...purposeManifests.chat!,
  recommendation: { ...purposeManifests.chat!.recommendation!, protocol_id: 'openai.responses' },
  protocols: purposeManifests.chat!.protocols.map(protocol => protocol.protocol_id === 'openai.chat_text' ? {
    ...protocol, protocol_id: 'openai.responses', endpoints: protocol.endpoints.map(endpoint =>
      endpoint.field === 'endpoint' ? { ...endpoint, default_value: '/responses' } : endpoint),
  } : protocol),
  model_defaults: {
    model: 'step-5-preview', protocol_id: 'openai.responses', context_limit: 1_000_000,
    provider_params: { reasoning_effort: 'medium' }, checked_at: '2026-10-15',
    sources: [
      'https://platform.stepfun.com/docs/zh/guides/models/step-5-preview',
      'https://platform.stepfun.com/docs/zh/api-reference/responses/responses-create',
      'https://platform.stepfun.com/docs/zh/guides/developer/reasoning',
      'https://platform.stepfun.com/docs/zh/guides/developer/integrations/codex',
    ],
  },
} } : purposeManifests;
const previewPurpose = previewParams.get('purpose') === 'chat' ? 'chat' : undefined;
const previewReferenceCatalog = previewParams.get('source') === 'official_documentation';
const EMPTY_ERRORS: never[] = [];
const INITIAL_CATALOG: ModelCatalogSuggestion[] = [
  { value: 'chat-example', label: 'chat-example', tasks: ['chat'], tasksSource: 'provider_declared', traits: [] },
  { value: 'image-example', label: 'image-example', tasks: ['image_generation'], tasksSource: 'provider_declared', traits: [] },
  { value: 'opaque-audio-model', label: 'opaque-audio-model', tasks: ['speech_recognition'], tasksSource: 'provider_declared', traits: [] },
  { value: 'name-only-asr-model', label: 'name-only-asr-model', tasks: ['speech_recognition'], tasksSource: 'inferred', traits: [] },
  { value: 'new-model-no-metadata', label: 'new-model-no-metadata', tasks: [], traits: [] },
];
function Preview() {
  const [value, setValue] = useState<ModelDefinitionDraft>(() => ({ ...createModelDefinitionDraft(previewModelReferences ? 'chat' : previewPurpose),
    model: previewModelReferences ? 'step-5-preview' : previewPurpose ? 'chat-example' : 'new-model-no-metadata' }));
  const [catalog, setCatalog] = useState(INITIAL_CATALOG);
  const [saved, setSaved] = useState('');
  const validation = useMemo(() => validateModelDefinition(value, manifests, aliasProviderBaseUrl, [], [], [], 'bearer'), [value]);
  return <ModelConfigurationWorkspace>
    <ModelConfigurationPanel visible title='添加模型' subtitle='共享模型编辑器 · 测试数据' onBack={() => setSaved('已返回')} footer={
      <Button type='primary' disabled={!validation.valid} onClick={() => setSaved(JSON.stringify({ model: value.model, capabilities: capabilityInputsFromDefinition(value) }))}>确认</Button>
    }>
      <ModelDefinitionEditor value={value} onChange={setValue} providerBaseUrl={aliasProviderBaseUrl} providerAuthScheme='bearer' manifests={manifests}
        validationErrors={value.model ? validation.errors : EMPTY_ERRORS} catalogSuggestions={catalog}
        catalogSource={previewReferenceCatalog ? 'official_documentation' : undefined}
        onRefreshCatalog={() => setCatalog([...INITIAL_CATALOG, { value: 'latest-refreshed-model', label: 'latest-refreshed-model', tasks: [], traits: [] }])} />
      {saved && <output aria-label='预览保存结果' style={{ display: 'block', overflowWrap: 'anywhere', marginTop: 12 }}>已确认：{saved}</output>}
    </ModelConfigurationPanel>
  </ModelConfigurationWorkspace>;
}
const style = document.createElement('style');
style.textContent = 'html,body,#root{margin:0;height:100%;min-width:880px;min-height:600px;font-family:Inter,"Microsoft YaHei",system-ui,sans-serif;background:var(--color-bg-1)}';
document.head.append(style);
createRoot(document.getElementById('root')!).render(<I18nextProvider i18n={i18n}><ConfigProvider locale={zhCN} theme={{ primaryColor: '#ef2355' }}><Preview /></ConfigProvider></I18nextProvider>);
