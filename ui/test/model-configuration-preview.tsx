/** Local visual acceptance of the actual shared editor; no credentials or backend writes. */
import React, { useMemo, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { Button, ConfigProvider, Modal } from '@arco-design/web-react';
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
import { capabilityInputsFromDefinition, createModelDefinitionDraft, validateModelDefinition, type ModelDefinitionDraft } from '../src/renderer/pages/settings/components/providerModelAdvanced';
import type { ModelCatalogSuggestion } from '../src/renderer/pages/settings/components/ModelDefinitionEditor';
import { aliasProviderBaseUrl } from './fixtures/modelAliasEditor';
import { purposeManifests } from './fixtures/modelPurposeEditor';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { settings, common } } }, interpolation: { escapeValue: false } });
const manifests = purposeManifests;
const previewParams = new URLSearchParams(window.location.search);
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
  const [value, setValue] = useState<ModelDefinitionDraft>(() => ({ ...createModelDefinitionDraft(previewPurpose), model: previewPurpose ? 'chat-example' : 'new-model-no-metadata' }));
  const [catalog, setCatalog] = useState(INITIAL_CATALOG);
  const [saved, setSaved] = useState('');
  const validation = useMemo(() => validateModelDefinition(value, manifests, aliasProviderBaseUrl, [], [], [], 'bearer'), [value]);
  return <main>
    <div className='preview-note'>共享模型编辑器 · 测试数据 · 不连接供应商、不写入实际配置</div>
    <Modal visible title='添加模型' style={{ width: 760 }} bodyStyle={{ maxHeight: 'calc(92vh - 120px)', overflow: 'auto' }} mask={false} closable={false} footer={
      <Button type='primary' disabled={!validation.valid} onClick={() => setSaved(JSON.stringify({ model: value.model, capabilities: capabilityInputsFromDefinition(value) }))}>确认</Button>
    }>
      <ModelDefinitionEditor value={value} onChange={setValue} providerBaseUrl={aliasProviderBaseUrl} providerAuthScheme='bearer' manifests={manifests}
        validationErrors={value.model ? validation.errors : EMPTY_ERRORS} catalogSuggestions={catalog}
        catalogSource={previewReferenceCatalog ? 'official_documentation' : undefined}
        onRefreshCatalog={() => setCatalog([...INITIAL_CATALOG, { value: 'latest-refreshed-model', label: 'latest-refreshed-model', tasks: [], traits: [] }])} />
      {saved && <output aria-label='预览保存结果' style={{ display: 'block', overflowWrap: 'anywhere', marginTop: 12 }}>已确认：{saved}</output>}
    </Modal>
  </main>;
}
const style = document.createElement('style');
style.textContent = 'html,body,#root{margin:0;min-width:880px;min-height:600px;font-family:Inter,"Microsoft YaHei",system-ui,sans-serif;background:var(--color-bg-1)}.preview-note{text-align:center;padding:12px;font-size:12px;color:var(--color-text-3)}';
document.head.append(style);
createRoot(document.getElementById('root')!).render(<I18nextProvider i18n={i18n}><ConfigProvider locale={zhCN} theme={{ primaryColor: '#ef2355' }}><Preview /></ConfigProvider></I18nextProvider>);
