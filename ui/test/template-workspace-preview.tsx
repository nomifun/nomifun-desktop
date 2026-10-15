/// <reference types="vite/client" />
/** Development-only visual harness. All template/config changes stay in memory. */
import { useMemo, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { Button, ConfigProvider, Message } from '@arco-design/web-react';
import '@arco-design/web-react/es/_util/react-19-adapter';
import zhCN from '@arco-design/web-react/es/locale/zh-CN';
import { MemoryRouter } from 'react-router-dom';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import 'virtual:uno.css';
import '@arco-design/web-react/dist/css/arco.css';
import '../src/renderer/styles/arco-override.css';
import '../src/renderer/styles/themes/index.css';
import '../src/renderer/styles/modal-contract.css';
import zh from '../src/renderer/services/i18n/locales/zh-CN';
import en from '../src/renderer/services/i18n/locales/en-US';
import { DEFAULT_THEME_ID, PRESET_THEMES } from '../src/renderer/pages/settings/DisplaySettings/presets';
import { ensureThemeControlContract, removeThemeControlContract } from '../src/renderer/utils/theme/themeControlContract';
import { parseProviderId } from '../src/common/types/ids';
import { uuidv7 } from '../src/common/utils/uuidv7';
import type { ModelTask } from '../src/common/config/storage';
import type { ProviderResponse } from '../src/common/types/provider/providerApi';
import type { CreativeModelCatalogSnapshot } from '../src/renderer/pages/creativeStudio/models';
import type { CreativeAsset } from '../src/renderer/pages/creativeStudio/assets';
import { cloneTemplateDefinition, type CreativeTemplateDefinitionV1, type CreativeTemplateRunAggregateV1 } from '../src/renderer/pages/creativeStudio/templates/domain';
import { createExecutableTemplateFixture, createTemplateRunFixture, IDS } from '../src/renderer/pages/creativeStudio/templates/domain/testFixtures';
import type { CreativeTemplateRepository } from '../src/renderer/pages/creativeStudio/templates/services';
import type { CreativeTemplateRunnerPort } from '../src/renderer/pages/creativeStudio/templates/page/TemplateRunModal';
import type { TemplateDraftPort } from '../src/renderer/pages/creativeStudio/templates/agent';

const params = new URLSearchParams(location.search);
type DataState = 'populated' | 'empty' | 'error';
const initialState: DataState = params.get('state') === 'empty' ? 'empty' : params.get('state') === 'error' ? 'error' : 'populated';
const previewConfig: Record<string, unknown> = {
  theme: params.get('theme') === 'dark' ? 'dark' : 'light',
  colorScheme: 'default', 'ui.zoomFactor': 1, 'system.keepAwake': false,
};
// The real theme hooks persist only into this document. Never forward API calls.
const originalFetch = globalThis.fetch;
globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
  const url = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url, location.origin);
  const method = (init?.method ?? (input instanceof Request ? input.method : 'GET')).toUpperCase();
  if (url.pathname === '/api/settings/client') {
    if (init?.method === 'PUT' && typeof init.body === 'string') Object.assign(previewConfig, JSON.parse(init.body));
    return new Response(JSON.stringify({ success: true, data: previewConfig }), { headers: { 'Content-Type': 'application/json' } });
  }
  // The connected production selector reads the authoritative nested catalog.
  if (method === 'GET' && url.pathname === '/api/providers') {
    return new Response(JSON.stringify({ success: true, data: providerResponses }), { headers: { 'Content-Type': 'application/json' } });
  }
  if (url.pathname.startsWith('/api/') || url.pathname === '/health') throw new Error(`Unsupported preview operation: ${url.pathname}`);
  return originalFetch(input, init);
}) as typeof fetch;

const { ipcBridge } = await import('../src/common');
const { configService } = await import('../src/common/config/configService');
ipcBridge.application.applyKeepAwake.invoke = async () => {};
ipcBridge.application.setZoomFactor.invoke = async ({ factor }) => factor;
ipcBridge.mode.onProvidersChanged.on = () => () => {};
ipcBridge.conversation.reconnected.on = () => () => {};
await configService.initialize();
const { ThemeProvider, useThemeContext } = await import('../src/renderer/hooks/context/ThemeContext');
const { default: CreativeTemplateWorkspacePage } = await import('../src/renderer/pages/creativeStudio/templates/page/CreativeTemplateWorkspacePage');
const { default: CreativeResourceDialog } = await import('../src/renderer/pages/creativeStudio/components/CreativeResourceDialog');

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: zh }, 'en-US': { translation: en } }, interpolation: { escapeValue: false } });
const presetStyle = document.createElement('style');
presetStyle.textContent = PRESET_THEMES.find((theme) => theme.id === DEFAULT_THEME_ID)?.css ?? '';
document.head.append(presetStyle);

function templates(): CreativeTemplateDefinitionV1[] {
  const names = [
    ['夏日产品海报', '品牌营销', '为产品创建干净、明亮的主视觉，突出质感与核心卖点。'],
    ['城市漫游视觉故事', '社交内容', '让每一张画面保持统一叙事，记录城市生活中的灵感与细节。'],
    ['轻盈生活方式', '品牌营销', '柔和自然光下的生活方式视觉，适用于商品详情页与社媒宣传。'],
    ['角色概念设计', '创意探索', '探索角色的气质、配色和服装细节，建立鲜明的视觉形象。'],
    ['咖啡品牌系列', '社交内容', '从咖啡豆到日常仪式，规划一组具有品牌辨识度的连贯画面。'],
    ['灵感草稿', '', '从简短的创意开始，快速验证新的视觉方向。'],
  ];
  return names.map(([name, category, description], index) => {
    const template = createExecutableTemplateFixture(index === 1 || index === 4);
    template.id = `018f0000-0000-7000-8000-${(0x101 + index).toString(16).padStart(12, '0')}`;
    template.metadata = { ...template.metadata, name, category, description, tags: [['产品摄影', '夏日'], ['城市', '叙事'], ['自然光', '生活方式'], ['角色', '概念设计'], ['咖啡', '品牌视觉', '系列创作'], []][index], createdAt: Date.UTC(2026, 9, 4 + index), updatedAt: Date.UTC(2026, 9, 10, 10 - index) };
    const topic = template.variables[0];
    topic.label = '创作主题';
    topic.description = '输入你希望表达的产品或主题。';
    if (topic.type === 'text') { topic.defaultValue = index === 0 ? 'NomiFun' : null; topic.placeholder = '例如：一杯夏日特调咖啡'; }
    template.variables[1].label = '参考图像';
    template.variables[1].description = '选择一张图片作为风格或构图参考。';
    template.templates[0].name = '视觉提示词';
    template.templates[0].segments = [{ kind: 'text', text: '为 ' }, { kind: 'variable', variableId: IDS.variable }, { kind: 'text', text: ' 创作精致的视觉画面。自然光，清晰主体，细腻材质，简洁构图。' }];
    return template;
  });
}

function repository(state: DataState): CreativeTemplateRepository {
  const values = new Map((state === 'empty' ? [] : templates()).map((template) => [template.id, template]));
  // Error mode fails once so the production retry interaction can recover.
  let failNextList = state === 'error';
  return {
    async list() {
      if (failNextList) { failNextList = false; throw new Error('预览模拟：模板加载失败，请重试。'); }
      return [...values.values()].map(cloneTemplateDefinition);
    },
    async create(template) {
      const value = cloneTemplateDefinition(template);
      value.metadata.createdAt = value.metadata.updatedAt = Date.now();
      values.set(value.id, value);
      return cloneTemplateDefinition(value);
    },
    async load(id) {
      const value = values.get(id);
      if (!value) throw new Error('模板不存在');
      return cloneTemplateDefinition(value);
    },
    async save(id, expectedRevision, template) {
      if (values.get(id)?.revision !== expectedRevision) throw new Error('模板版本已变化');
      const value = cloneTemplateDefinition(template);
      value.metadata.updatedAt = Date.now();
      values.set(id, value);
      return cloneTemplateDefinition(value);
    },
    async remove(id) { values.delete(id); },
  };
}

const providerId = parseProviderId(IDS.provider);
const capability = (task: ModelTask, protocol: string) => ({ task, traits: [], protocol, connection_role: 'default', allow_cross_origin_credentials: false, provider_params: {}, created_at: 1, updated_at: 1 });
const modelCatalog: CreativeModelCatalogSnapshot = {
  status: 'ready', error: null, providers: [{ id: providerId, name: 'Nomi 预览模型', platform: 'custom', base_url: 'https://example.invalid', auth_scheme: 'bearer', has_credentials: true, enabled: true,
    models: [
      { provider_id: providerId, model: 'nomifun-chat-test', display_name: 'Nomi 文本模型', enabled: true, sort_order: 0, capabilities: [capability('chat', 'openai.chat_text')], created_at: 1, updated_at: 1 },
      { provider_id: providerId, model: 'nomifun-image-test', display_name: 'Nomi 图像模型', enabled: true, sort_order: 1, capabilities: [capability('image_generation', 'openai.images'), capability('image_edit', 'openai.images_edit')], created_at: 1, updated_at: 1 },
    ],
  }],
};
const providerResponses: ProviderResponse[] = modelCatalog.providers.map((provider) => ({
  provider_id: provider.id, platform: provider.platform, name: provider.name,
  base_url: provider.base_url, auth_scheme: provider.auth_scheme, has_credentials: provider.has_credentials,
  models: provider.models, enabled: provider.enabled !== false, sort_order: provider.sort_order ?? 0,
  created_at: 1, updated_at: 1,
}));
const draftPort: TemplateDraftPort = { async draft({ prompt }) { return { text: JSON.stringify({ kind: 'nomifun.creative-studio.template-draft/v1', summary: '预览草稿已生成', draft: { mode: 'single-image', name: 'AI 灵感模板', category: '创意探索', description: prompt, promptTemplate: '围绕 {{theme}} 创作一张视觉作品，以 {{style}} 作为整体风格。' } }) }; } };

const samplePausedRunId = '018f0000-0000-7000-8000-000000000202';
const sampleAssets = new Map(Array.from({ length: 7 }, (_, index): [string, CreativeAsset] => {
  const id = `018f0000-0000-7000-8000-${String(300 + index).padStart(12, '0')}`;
  const deleted = index === 1;
  const hue = 130 + index * 21;
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="640" height="800" viewBox="0 0 640 800"><rect width="640" height="800" fill="hsl(${hue} 25% 92%)"/><circle cx="320" cy="350" r="180" fill="hsl(${hue} 30% 80%)"/><rect x="235" y="220" width="170" height="300" rx="24" fill="hsl(${hue} 35% 35%)"/><text x="320" y="310" text-anchor="middle" fill="white" font-size="24" font-family="sans-serif">NOMI</text><text x="320" y="395" text-anchor="middle" fill="white" font-size="58" font-family="sans-serif">${index + 1}</text><text x="320" y="650" text-anchor="middle" fill="hsl(${hue} 35% 25%)" font-size="24" font-family="sans-serif">RESULT ${index + 1} / 7</text></svg>`;
  const originalUrl = deleted ? '' : `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
  return [id, {
    id, kind: 'image', title: `咖啡品牌系列 · 结果 ${index + 1}`, collection: null, tags: [],
    mimeType: 'image/svg+xml', width: 640, height: 800, bytes: svg.length, inLibrary: !deleted,
    textContent: null, origin: null, originalUrl, thumbnailUrl: null,
    createdAt: Date.UTC(2026, 9, 15, 10, 49), updatedAt: Date.UTC(2026, 9, 15, 10, 50),
    deletedAt: deleted ? Date.UTC(2026, 9, 15, 10, 51) : null,
  }];
}));
const sampleAssetReader = { async get(id: string): Promise<CreativeAsset> {
  const asset = sampleAssets.get(id);
  if (!asset) throw new Error('预览素材不存在');
  return asset;
} };

function sampleRuns(): CreativeTemplateRunAggregateV1[] {
  const run = createTemplateRunFixture(true);
  run.templateSnapshot = templates()[1];
  run.request.templateId = run.record.templateId = run.templateSnapshot.id;
  run.request.requestedAt = Date.UTC(2026, 9, 15, 10, 56);
  run.record.status = 'awaiting-review';
  run.promptDrafts = [IDS.draft1, IDS.draft2].map((id, index) => ({ id, templateId: run.templateSnapshot.id, runRequestId: run.request.id, seriesIndex: index, title: index === 0 ? '清晨的街角' : '午后的咖啡馆', prompt: index === 0 ? '清晨阳光照亮的城市街角，温暖自然，细腻摄影质感。' : '午后咖啡馆窗边，柔和光线，延续自然温暖的城市故事。', status: 'pending-review', createdAt: run.request.requestedAt, reviewedAt: null, reviewNote: null }));
  run.record.promptDraftIds = run.promptDrafts.map((draft) => draft.id);
  const paused = createTemplateRunFixture(true);
  paused.request.id = paused.request.idempotencyKey = paused.record.requestId = samplePausedRunId;
  paused.templateSnapshot = templates()[1];
  paused.request.templateId = paused.record.templateId = paused.templateSnapshot.id;
  paused.request.requestedAt = Date.UTC(2026, 9, 15, 10, 53);
  paused.record.status = 'running';

  const succeeded = createTemplateRunFixture(true);
  succeeded.request.id = succeeded.request.idempotencyKey = succeeded.record.requestId = '018f0000-0000-7000-8000-000000000203';
  succeeded.templateSnapshot = templates()[4];
  if (succeeded.templateSnapshot.output.kind === 'multi-image-series') succeeded.templateSnapshot.output.targetCount = 7;
  succeeded.request.output = { ...succeeded.templateSnapshot.output };
  succeeded.request.templateId = succeeded.record.templateId = succeeded.templateSnapshot.id;
  succeeded.request.requestedAt = Date.UTC(2026, 9, 15, 10, 49);
  succeeded.record.status = 'succeeded';
  succeeded.record.completedAt = Date.UTC(2026, 9, 15, 10, 50);
  succeeded.record.taskIds = Array.from({ length: 7 }, (_, index) => `018f0000-0000-7000-8000-${String(400 + index).padStart(12, '0')}`);
  succeeded.record.resultAssetIds = [...sampleAssets.keys()];

  const failed = createTemplateRunFixture();
  failed.request.id = failed.request.idempotencyKey = failed.record.requestId = '018f0000-0000-7000-8000-000000000204';
  failed.templateSnapshot = templates()[0];
  failed.request.templateId = failed.record.templateId = failed.templateSnapshot.id;
  failed.request.requestedAt = Date.UTC(2026, 9, 15, 10, 41);
  failed.record.status = 'failed';
  failed.record.completedAt = Date.UTC(2026, 9, 15, 10, 42);
  failed.record.failure = { code: 'task-failed', message: '生成服务暂时不可用，请稍后重新运行。' };

  return [run, paused, succeeded, failed];
}

function Preview() {
  const { theme, setTheme } = useThemeContext();
  const [state, setState] = useState<DataState>(initialState);
  const [dialog, setDialog] = useState(params.get('context') === 'dialog');
  const [minimum, setMinimum] = useState(params.get('minimum') === 'true');
  const [runs, setRuns] = useState<CreativeTemplateRunAggregateV1[]>(params.get('runs') === 'true' ? sampleRuns() : []);
  const [reset, setReset] = useState(0);
  const memoryRepository = useMemo(() => repository(state), [state, reset]);
  const runner: CreativeTemplateRunnerPort = { async start(input) {
    const run = createTemplateRunFixture(input.template.output.kind === 'multi-image-series');
    run.request.id = run.request.idempotencyKey = uuidv7();
    run.templateSnapshot = cloneTemplateDefinition(input.template);
    run.request.templateId = run.record.templateId = input.template.id;
    run.record.requestId = run.request.id;
    run.request.inputs = input.inputs;
    run.request.referenceAssetIds = [...input.referenceAssetIds];
    run.request.output = input.template.output;
    run.request.requestedAt = Date.now();
    run.record.status = 'queued';
    setRuns((current) => [run, ...current]);
  } };
  const finish = async (id: string, status: 'cancelled' | 'running') => { setRuns((current) => current.map((run) => run.request.id === id ? { ...run, revision: run.revision + 1, record: { ...run.record, status, completedAt: status === 'cancelled' ? Date.now() : null } } : run)); };
  const paused = runs.find((run) => run.request.id === samplePausedRunId && run.revision === 1);
  const workspace = <CreativeTemplateWorkspacePage key={`${state}-${reset}`} repository={memoryRepository} runner={runner} agentDraftPort={draftPort} agentModelCatalog={modelCatalog}
    onOpenModelSettings={() => Message.info('预览仅展示模板工作台，模型设置导航已隔离。')}
    runCenter={{ snapshot: { loading: false, loadError: null, runs, activities: paused ? { [paused.request.id]: { state: 'paused', taskStatuses: {}, error: '预览模拟：连接中断，等待继续。' } } : {} }, assetUrl: (id) => sampleAssets.get(id)?.originalUrl ?? '', assetReader: sampleAssetReader, resume: (id) => finish(id, 'running'), cancel: (id) => finish(id, 'cancelled'), review: (id) => finish(id, 'running'), retry: (run) => runner.start({ template: run.templateSnapshot, inputs: run.request.inputs, referenceAssetIds: run.request.referenceAssetIds }) }} />;
  return <>
    <nav className='template-preview-tools' aria-label='预览控制'>
      <span>模板工作台 · {DEFAULT_THEME_ID} · 内存数据</span>
      <select aria-label='预览数据状态' value={state} onChange={(event) => { setState(event.target.value as DataState); setRuns([]); }}><option value='populated'>模板列表</option><option value='empty'>空列表</option><option value='error'>加载错误</option></select>
      <button onClick={() => void setTheme(theme === 'dark' ? 'light' : 'dark')}>{theme === 'dark' ? '切换浅色' : '切换深色'}</button>
      <button onClick={() => setDialog(!dialog)}>{dialog ? '独立页面' : '资源弹窗'}</button>
      <button onClick={() => setMinimum(!minimum)}>{minimum ? '完整窗口' : '880 × 600'}</button>
      <button onClick={() => setRuns(runs.length ? [] : sampleRuns())}>{runs.length ? '清空任务示例' : '载入任务示例'}</button>
      <button onClick={() => { setReset((value) => value + 1); setRuns([]); }}>重置数据</button>
    </nav>
    <div className={`template-preview-frame${minimum ? ' template-preview-frame--minimum' : ''}`}>
      {dialog ? <>
        <div className='template-preview-canvas'><h1>创作空间</h1><p>检查资源弹窗与模板编辑、执行、审核弹窗的层级与焦点。</p><Button type='primary' onClick={() => setDialog(true)}>打开模板工作台</Button></div>
        <CreativeResourceDialog kind='templates' scope='conversation' title='模板工作台' onClose={() => setDialog(false)}>{workspace}</CreativeResourceDialog>
      </> : workspace}
    </div>
  </>;
}

const style = document.createElement('style');
style.textContent = `html,body,#root{margin:0;height:100%;min-width:880px;font-family:Inter,"Microsoft YaHei",system-ui,sans-serif}body{background:var(--color-bg-1);color:var(--text-primary)}.template-preview-tools{box-sizing:border-box;min-height:38px;display:flex;align-items:center;gap:8px;flex-wrap:wrap;padding:6px 12px;font-size:11px;border-bottom:1px solid var(--color-border-2);color:var(--text-secondary)}.template-preview-tools button,.template-preview-tools select{font:inherit;border:1px solid var(--color-border-2);border-radius:5px;background:var(--color-bg-2);color:var(--text-primary);padding:3px 7px;cursor:pointer}.template-preview-tools button:focus-visible,.template-preview-tools select:focus-visible{outline:2px solid var(--color-primary);outline-offset:2px}.template-preview-frame{height:calc(100% - 38px);min-height:0;min-width:0;display:flex;flex-direction:column}.template-preview-frame--minimum{width:880px;height:600px;border:1px solid var(--color-border-2);margin:12px auto}.template-preview-canvas{height:100%;padding:48px;background:var(--color-bg-2)}.template-preview-canvas h1{font-size:24px}.template-preview-canvas p{font-size:13px;color:var(--text-secondary)}`;
document.head.append(style);
ensureThemeControlContract();
const root = createRoot(document.getElementById('root')!);
if (import.meta.hot) import.meta.hot.dispose(() => { root.unmount(); style.remove(); presetStyle.remove(); removeThemeControlContract(); globalThis.fetch = originalFetch; });
root.render(<I18nextProvider i18n={i18n}><ThemeProvider><ConfigProvider locale={zhCN}><MemoryRouter><Preview /></MemoryRouter></ConfigProvider></ThemeProvider></I18nextProvider>);
