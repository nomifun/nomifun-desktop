/// <reference types="vite/client" />
/** Development-only harness: production resource pages, in-memory ports, no backend writes. */
import { useMemo, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { ConfigProvider, Message } from '@arco-design/web-react';
import '@arco-design/web-react/es/_util/react-19-adapter';
import zhCN from '@arco-design/web-react/es/locale/zh-CN';
import { MemoryRouter, Route, Routes, useLocation, useNavigate } from 'react-router-dom';
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
import { MATERIALS_PATH, PROMPTS_PATH, TEMPLATES_PATH, resourceSpaceSectionForPath } from '../src/renderer/pages/creativeStudio/app/resourceRoutes';
import { parseProviderId } from '../src/common/types/ids';
import { uuidv7 } from '../src/common/utils/uuidv7';
import type { ModelTask } from '../src/common/config/storage';
import type { ProviderResponse } from '../src/common/types/provider/providerApi';
import type { CreativeModelCatalogSnapshot } from '../src/renderer/pages/creativeStudio/models';
import type { CreativeAsset, CreativeAssetLibraryPort } from '../src/renderer/pages/creativeStudio/assets';
import type { PromptLibraryItem } from '../src/renderer/pages/creativeStudio/prompts';
import { cloneTemplateDefinition, type CreativeTemplateRunAggregateV1 } from '../src/renderer/pages/creativeStudio/templates/domain';
import { createExecutableTemplateFixture, createTemplateRunFixture, IDS } from '../src/renderer/pages/creativeStudio/templates/domain/testFixtures';
import type { CreativeTemplateRepository } from '../src/renderer/pages/creativeStudio/templates/services';
import type { CreativeTemplateRunnerPort } from '../src/renderer/pages/creativeStudio/templates/page/TemplateRunModal';
import type { TemplateDraftPort } from '../src/renderer/pages/creativeStudio/templates/agent';

const params = new URLSearchParams(location.search);
type DataState = 'populated' | 'empty' | 'error';
const initialState: DataState = params.get('state') === 'empty' ? 'empty' : params.get('state') === 'error' ? 'error' : 'populated';
const previewConfig: Record<string, unknown> = {
  theme: params.get('theme') === 'dark' ? 'dark' : 'light', colorScheme: 'default',
  'ui.zoomFactor': 1, 'system.keepAwake': false,
};
const originalFetch = globalThis.fetch;
globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
  const url = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url, location.origin);
  const method = (init?.method ?? (input instanceof Request ? input.method : 'GET')).toUpperCase();
  if (url.pathname === '/api/settings/client') {
    if (method === 'PUT' && typeof init?.body === 'string') Object.assign(previewConfig, JSON.parse(init.body));
    return Response.json({ success: true, data: previewConfig });
  }
  if (method === 'GET' && url.pathname === '/api/providers') return Response.json({ success: true, data: providerResponses });
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
const { default: ResourceSpacePage } = await import('../src/renderer/pages/creativeStudio/app/ResourceSpacePage');
const { default: ResourcePageBoundary } = await import('../src/renderer/pages/creativeStudio/app/ResourcePageBoundary');
const { default: CreativeAssetLibraryPage } = await import('../src/renderer/pages/creativeStudio/assets/page/CreativeAssetLibraryPage');
const { default: CreativeTemplateWorkspacePage } = await import('../src/renderer/pages/creativeStudio/templates/page/CreativeTemplateWorkspacePage');
const { StandalonePromptLibraryAppearance } = await import('../src/renderer/pages/creativeStudio/prompts/page/StandalonePromptLibraryPage');

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: zh }, 'en-US': { translation: en } }, interpolation: { escapeValue: false } });
const presetStyle = document.createElement('style');
presetStyle.textContent = PRESET_THEMES.find((theme) => theme.id === DEFAULT_THEME_ID)?.css ?? '';
document.head.append(presetStyle);

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
  provider_id: provider.id, platform: provider.platform, name: provider.name, base_url: provider.base_url,
  auth_scheme: provider.auth_scheme, has_credentials: provider.has_credentials, models: provider.models,
  enabled: true, sort_order: 0, created_at: 1, updated_at: 1,
}));
const draftPort: TemplateDraftPort = { async draft({ prompt }) { return { text: JSON.stringify({ kind: 'nomifun.creative-studio.template-draft/v1', summary: '预览草稿已生成', draft: { mode: 'single-image', name: 'AI 灵感模板', category: '创意探索', description: prompt, promptTemplate: '围绕 {{theme}} 创作一张视觉作品。' } }) }; } };

function sampleImage(index: number) {
  const hue = [35, 160, 215, 15, 270, 190][index % 6];
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600" viewBox="0 0 800 600"><defs><linearGradient id="b" x2="1" y2="1"><stop stop-color="hsl(${hue} 35% 94%)"/><stop offset="1" stop-color="hsl(${hue} 30% 75%)"/></linearGradient></defs><rect width="800" height="600" fill="url(#b)"/><ellipse cx="400" cy="490" rx="200" ry="30" fill="hsl(${hue} 20% 40%)" opacity=".15"/><circle cx="620" cy="130" r="170" fill="hsl(${hue} 45% 85%)"/><rect x="280" y="145" width="240" height="340" rx="35" fill="hsl(${hue} 25% 30%)"/><rect x="345" y="118" width="110" height="45" rx="10" fill="hsl(${hue} 25% 22%)"/><text x="400" y="300" text-anchor="middle" fill="white" font-family="sans-serif" font-size="32" letter-spacing="7">NOMI</text><path d="M340 355h120M360 375h80" stroke="white" opacity=".4" stroke-width="3"/><text x="400" y="430" text-anchor="middle" fill="white" opacity=".6" font-family="sans-serif" font-size="12">CREATIVE SERIES ${index + 1}</text></svg>`;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

function assetStore(state: DataState) {
  const names = ['夏日产品主视觉', '咖啡品牌静物', '城市故事 · 蓝调', '生活方式参考', '创意配色探索', '自然光产品摄影'];
  const values = new Map<string, CreativeAsset>((state === 'empty' ? [] : names).map((title, index) => {
    const id = `preview-asset-${index + 1}`;
    return [id, { id, kind: 'image', title, collection: index < 3 ? '品牌视觉' : '灵感参考', tags: ['产品摄影', '参考图'], mimeType: 'image/svg+xml', width: 800, height: 600, bytes: 1048576 + index * 150000, inLibrary: true, textContent: null, origin: null, originalUrl: sampleImage(index), thumbnailUrl: null, createdAt: Date.UTC(2026, 9, 15, 10, index), updatedAt: Date.UTC(2026, 9, 15, 10, index) }];
  }));
  let failNext = state === 'error';
  const client: CreativeAssetLibraryPort = {
    async list(query = {}) {
      if (failNext) { failNext = false; throw new Error('预览模拟：素材加载失败，请重试。'); }
      const filtered = [...values.values()].filter((asset) => (!query.kind || asset.kind === query.kind) && (!query.collection || asset.collection === query.collection) && (!query.ungrouped || !asset.collection) && (!query.search || `${asset.title} ${asset.tags.join(' ')}`.includes(query.search)));
      filtered.sort((a, b) => query.sort === 'name_asc' ? a.title.localeCompare(b.title) : query.sort === 'created_asc' ? a.createdAt - b.createdAt : query.sort === 'size_desc' ? (b.bytes ?? 0) - (a.bytes ?? 0) : b.createdAt - a.createdAt);
      const start = ((query.page ?? 1) - 1) * (query.pageSize ?? 10);
      return { items: filtered.slice(start, start + (query.pageSize ?? 10)), total: filtered.length };
    },
    async upload(file, metadata, signal, onProgress) {
      signal?.throwIfAborted();
      const asset: CreativeAsset = { id: uuidv7(), kind: file.type.startsWith('video/') ? 'video' : 'image', title: metadata?.title ?? file.name, collection: metadata?.collection ?? null, tags: metadata?.tags ?? [], mimeType: file.type, width: null, height: null, bytes: file.size, inLibrary: true, textContent: null, origin: null, originalUrl: URL.createObjectURL(file), thumbnailUrl: null, createdAt: Date.now(), updatedAt: Date.now() };
      values.set(asset.id, asset); onProgress?.(100); return asset;
    },
    async createText(input) {
      const asset: CreativeAsset = { id: uuidv7(), kind: 'text', title: input.title, collection: input.collection ?? null, tags: input.tags ?? [], mimeType: 'text/plain', width: null, height: null, bytes: input.textContent.length, inLibrary: true, textContent: input.textContent, origin: null, originalUrl: '', thumbnailUrl: null, createdAt: Date.now(), updatedAt: Date.now() };
      values.set(asset.id, asset); return asset;
    },
    async update(id, patch) { const current = values.get(id); if (!current) throw new Error('素材不存在'); const asset = { ...current, ...patch, updatedAt: Date.now() }; values.set(id, asset); return asset; },
    async remove(id) { values.delete(id); },
    async renameCollection(from, to) { let count = 0; for (const [id, asset] of values) if (asset.collection === from) { values.set(id, { ...asset, collection: to }); count += 1; } return count; },
    url: (id) => values.get(id)?.originalUrl ?? '',
  };
  return { client, reader: { async get(id: string) { const asset = values.get(id); if (!asset) throw new Error('素材不存在'); return asset; } } };
}

function sampleTemplates() {
  return [
    ['夏日产品海报', '品牌营销', '为产品创建明亮的主视觉，突出质感与核心卖点。'],
    ['城市漫游视觉故事', '社交内容', '让每一张画面保持统一叙事，记录生活中的灵感与细节。'],
    ['轻盈生活方式', '品牌营销', '柔和自然光下的生活方式视觉，适用于商品详情与社媒宣传。'],
    ['角色概念设计', '创意探索', '探索角色的气质、配色和服装细节，建立鲜明的视觉形象。'],
    ['咖啡品牌系列', '社交内容', '规划一组具有品牌辨识度的连贯画面。'],
    ['灵感草稿', '', '从简短的创意开始，快速验证新的视觉方向。'],
  ].map(([name, category, description], index) => {
    const template = createExecutableTemplateFixture(index === 1 || index === 4);
    template.id = `018f0000-0000-7000-8000-${(0x101 + index).toString(16).padStart(12, '0')}`;
    template.metadata = { ...template.metadata, name, category, description, tags: [['产品摄影', '夏日'], ['城市', '叙事'], ['自然光'], ['角色设计'], ['咖啡', '系列创作'], []][index], createdAt: Date.UTC(2026, 9, 10 + index), updatedAt: Date.UTC(2026, 9, 10 + index) };
    template.variables[0].label = '创作主题'; template.variables[1].label = '参考图像';
    return template;
  });
}

function templateRepository(state: DataState): CreativeTemplateRepository {
  const values = new Map((state === 'empty' ? [] : sampleTemplates()).map((template) => [template.id, template]));
  let failNext = state === 'error';
  return {
    async list() { if (failNext) { failNext = false; throw new Error('预览模拟：模板加载失败，请重试。'); } return [...values.values()].map(cloneTemplateDefinition); },
    async load(id) { const value = values.get(id); if (!value) throw new Error('模板不存在'); return cloneTemplateDefinition(value); },
    async create(template) { const value = cloneTemplateDefinition(template); value.metadata.createdAt = value.metadata.updatedAt = Date.now(); values.set(value.id, value); return cloneTemplateDefinition(value); },
    async save(id, revision, template) { if (values.get(id)?.revision !== revision) throw new Error('模板版本已变化'); const value = cloneTemplateDefinition(template); value.metadata.updatedAt = Date.now(); values.set(id, value); return cloneTemplateDefinition(value); },
    async remove(id) { values.delete(id); },
  };
}

const promptItems: PromptLibraryItem[] = ['产品摄影', '城市漫游', '角色概念'].map((title, index) => ({ id: `preview-prompt-${index}`, source: 'catalog', title: `${title}灵感`, description: '复用创作表达，寻找新的视觉方向。', prompt: `以${title}为主题构建清晰的画面，关注光线、构图与细节。`, category: index === 0 ? '品牌营销' : '创意探索', tags: ['视觉', '灵感'], knowledgeBaseIds: [], coverUrl: null, preview: null, sourceUrl: null, license: null, licenseUrl: null, createdAt: null, updatedAt: null, savedToAssets: false }));

function PreviewSpace({ state }: { state: DataState }) {
  const { pathname } = useLocation(); const navigate = useNavigate();
  const assets = useMemo(() => assetStore(state), [state]);
  const repository = useMemo(() => templateRepository(state), [state]);
  const [runs, setRuns] = useState<CreativeTemplateRunAggregateV1[]>([]);
  const runner: CreativeTemplateRunnerPort = { async start(input) {
    const run = createTemplateRunFixture(input.template.output.kind === 'multi-image-series');
    run.request.id = run.request.idempotencyKey = uuidv7(); run.record.requestId = run.request.id;
    run.templateSnapshot = cloneTemplateDefinition(input.template); run.request.templateId = run.record.templateId = input.template.id;
    run.request.inputs = input.inputs; run.request.referenceAssetIds = input.referenceAssetIds; run.request.output = input.template.output;
    run.request.requestedAt = Date.now(); run.record.status = 'queued'; setRuns((current) => [run, ...current]);
  } };
  const finish = async (id: string, status: 'cancelled' | 'running') => setRuns((current) => current.map((run) => run.request.id === id ? { ...run, revision: run.revision + 1, record: { ...run.record, status } } : run));
  return <ResourceSpacePage section={resourceSpaceSectionForPath(pathname) ?? 'assets'}
    onSectionChange={(section) => void navigate(section === 'assets' ? MATERIALS_PATH : TEMPLATES_PATH)}
    assets={<CreativeAssetLibraryPage client={assets.client} locale='zh-CN' />}
    templates={<CreativeTemplateWorkspacePage repository={repository} runner={runner} agentDraftPort={draftPort} agentModelCatalog={modelCatalog}
      onOpenModelSettings={() => Message.info('预览模型仅保存在内存中。')}
      onPickAssets={async () => ['preview-asset-1']} onPickReferenceAssets={async () => ['preview-asset-1']}
      onUploadReferenceImages={async (files, ids) => [...ids, ...await Promise.all(files.map(async (file) => (await assets.client.upload(file)).id))]}
      runCenter={{ snapshot: { loading: false, loadError: null, runs, activities: {} }, assetUrl: assets.client.url, assetReader: assets.reader,
        resume: (id) => finish(id, 'running'), cancel: (id) => finish(id, 'cancelled'), review: (id) => finish(id, 'running'),
        retry: (run) => runner.start({ template: run.templateSnapshot, inputs: run.request.inputs, referenceAssetIds: run.request.referenceAssetIds }) }} />} />;
}

function Preview() {
  const { theme, setTheme } = useThemeContext();
  const [state, setState] = useState<DataState>(initialState);
  const navigate = useNavigate(); const { pathname } = useLocation();
  return <>
    {params.get('tools') !== 'false' ? <nav className='resource-preview-tools' aria-label='预览控制'>
      <span>资源空间 · 内存数据</span><select aria-label='预览数据状态' value={state} onChange={(event) => setState(event.target.value as DataState)}><option value='populated'>有内容</option><option value='empty'>空列表</option><option value='error'>加载失败</option></select>
      <button onClick={() => void setTheme(theme === 'dark' ? 'light' : 'dark')}>{theme === 'dark' ? '切换浅色' : '切换深色'}</button>
    </nav> : null}
    <div className='resource-preview-titlebar'><span>◉ &nbsp; NomiFun</span><span>资源空间 · 桌面布局预览</span><span>— &nbsp; □ &nbsp; ×</span></div>
    <div className='resource-preview-workbench'>
      <aside className='resource-preview-sidebar' aria-label='主导航'><strong>NomiFun</strong><span className='resource-preview-group'>工作空间</span><button>◉ &nbsp; 我的伙伴</button><button>▧ &nbsp; 我的画布</button>
        <button className={pathname !== PROMPTS_PATH ? 'is-active' : ''} onClick={() => void navigate(MATERIALS_PATH)}>▦ &nbsp; 资源空间</button>
        <button className={pathname === PROMPTS_PATH ? 'is-active' : ''} onClick={() => void navigate(PROMPTS_PATH)}>✧ &nbsp; 提示词库</button>
        <span className='resource-preview-group'>能力</span><button>◇ &nbsp; 模型中心</button><button>⊞ &nbsp; 扩展能力</button><span className='resource-preview-sidebar-note'>预览数据与操作<br />仅保存在当前页面</span>
      </aside>
      <div className='resource-preview-content'><Routes><Route element={<ResourcePageBoundary />}>
        <Route element={<PreviewSpace key={state} state={state} />}><Route path={MATERIALS_PATH} element={<></>} /><Route path={TEMPLATES_PATH} element={<></>} /></Route>
        <Route path={PROMPTS_PATH} element={<StandalonePromptLibraryAppearance items={promptItems} onSelect={(item) => Message.info(`已选择：${item.title}`)} />} />
      </Route></Routes></div>
    </div>
  </>;
}

const style = document.createElement('style');
style.textContent = `html,body,#root{margin:0;height:100%;min-width:880px;font-family:Inter,"Microsoft YaHei",system-ui,sans-serif}#root{display:flex;flex-direction:column}body{background:var(--color-bg-1);color:var(--text-primary)}.resource-preview-tools{display:flex;flex:none;min-height:32px;align-items:center;gap:10px;padding:4px 14px;box-sizing:border-box;border-bottom:1px solid var(--color-border-2);font-size:11px;color:var(--text-secondary)}.resource-preview-tools button,.resource-preview-tools select{font:inherit;border:1px solid var(--color-border-2);border-radius:5px;background:var(--color-bg-2);color:var(--text-primary);padding:3px 7px;cursor:pointer}.resource-preview-titlebar{box-sizing:border-box;height:38px;flex:none;display:flex;align-items:center;justify-content:space-between;padding:0 16px;border-bottom:1px solid var(--color-border-2);font-size:11px;color:var(--text-secondary)}.resource-preview-workbench{display:flex;flex:1;min-height:0}.resource-preview-sidebar{box-sizing:border-box;width:184px;flex:none;display:flex;flex-direction:column;gap:5px;padding:22px 12px 12px;background:var(--color-bg-2);border-right:1px solid var(--color-border-2)}.resource-preview-sidebar strong{padding:0 10px 14px;font-size:17px;letter-spacing:-.4px}.resource-preview-group{padding:12px 10px 4px;color:var(--text-secondary);font-size:11px}.resource-preview-sidebar button{text-align:left;border:0;background:transparent;border-radius:7px;color:var(--text-primary);font:inherit;font-size:12px;padding:8px 10px;cursor:pointer}.resource-preview-sidebar button.is-active{background:var(--color-fill-2);font-weight:600}.resource-preview-sidebar-note{margin-top:auto;padding:12px 10px;color:var(--text-secondary);font-size:10px;line-height:18px}.resource-preview-content{flex:1;min-width:0;min-height:0;height:100%}`;
document.head.append(style);
ensureThemeControlContract();
const root = createRoot(document.getElementById('root')!);
if (import.meta.hot) import.meta.hot.dispose(() => { root.unmount(); style.remove(); presetStyle.remove(); removeThemeControlContract(); globalThis.fetch = originalFetch; });
root.render(<I18nextProvider i18n={i18n}><ThemeProvider><ConfigProvider locale={zhCN}><MemoryRouter initialEntries={[params.get('tab') === 'templates' ? TEMPLATES_PATH : params.get('tab') === 'prompts' ? PROMPTS_PATH : MATERIALS_PATH]}><Preview /></MemoryRouter></ConfigProvider></ThemeProvider></I18nextProvider>);
