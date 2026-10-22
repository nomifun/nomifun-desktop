import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, Route, Routes, useLocation, useNavigate } from 'react-router-dom';

import { ResourceSpaceHeaderActions, ResourceSpaceTaskActions, useResourceSpaceChrome } from './ResourceSpaceChrome';
import ResourceSpacePage from './ResourceSpacePage';
import { MATERIALS_PATH, PROMPTS_PATH, TEMPLATES_PATH, resourceSpaceSectionForPath } from './resourceRoutes';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'zh-CN', resources: { 'zh-CN': { translation: { creativeStudio: { resources: {
    title: '资源空间', description: '管理创作素材与模板', materials: '素材', templates: '模板',
    tabsLabel: '资源分类', templateTasks: '模板任务', tasksLoadFailed: '模板任务加载失败', loading: '加载资源中…',
  } } } } }, interpolation: { escapeValue: false },
});

afterEach(() => { cleanup(); mock.restore(); });

function fixture(initialPath = MATERIALS_PATH, failTemplates = false) {
  const mounts = { assets: 0, templates: 0 };
  function Panel({ section }: { section: 'assets' | 'templates' }) {
    const chrome = useResourceSpaceChrome();
    const [search, setSearch] = useState('');
    const [tasksOpen, setTasksOpen] = useState(chrome?.openTaskCenterOnMount ?? false);
    useEffect(() => { mounts[section] += 1; }, [section]);
    if (section === 'templates' && failTemplates) throw new Error('模板预览加载失败');
    return <>
      <ResourceSpaceHeaderActions>
        <input aria-label={`${section}搜索`} value={search} onChange={(event) => setSearch(event.target.value)} />
        <button type='button'>{section}新建</button>
      </ResourceSpaceHeaderActions>
      <div data-panel-scroll={section} style={{ overflow: 'auto', height: 300 }}><div style={{ height: 1200 }}>{section}内容</div></div>
      {section === 'templates' ? <ResourceSpaceTaskActions><button type='button' onClick={() => setTasksOpen(true)}>模板任务</button></ResourceSpaceTaskActions> : null}
      {section === 'templates' && tasksOpen ? createPortal(<div role='dialog' aria-label='模板任务详情'><button onClick={() => setTasksOpen(false)}>关闭任务</button></div>, document.body) : null}
    </>;
  }
  function Space() {
    const { pathname } = useLocation();
    const navigate = useNavigate();
    return <ResourceSpacePage section={resourceSpaceSectionForPath(pathname) ?? 'assets'}
      onSectionChange={(section) => void navigate(section === 'assets' ? MATERIALS_PATH : TEMPLATES_PATH)}
      assets={<Panel section='assets' />} templates={<Panel section='templates' />} />;
  }
  function Navigation() {
    const navigate = useNavigate();
    return <button onClick={() => void navigate(PROMPTS_PATH)}>打开独立提示词库</button>;
  }
  const page = render(<I18nextProvider i18n={i18n}><MemoryRouter initialEntries={[initialPath]}>
    <Navigation />
    <Routes>
      <Route element={<Space />}><Route path={MATERIALS_PATH} element={<></>} /><Route path={TEMPLATES_PATH} element={<></>} /></Route>
      <Route path={PROMPTS_PATH} element={<h1>提示词库</h1>} />
    </Routes>
  </MemoryRouter></I18nextProvider>);
  return { ...page, mounts };
}

describe('ResourceSpacePage interaction', () => {
  test('shows only material/template tabs and does not mount template runtime on initial materials visit', async () => {
    const page = fixture();
    expect(page.getAllByRole('tab').map((tab) => tab.textContent)).toEqual(['素材', '模板']);
    expect(page.getByRole('heading', { name: '资源空间' })).toBeTruthy();
    await waitFor(() => expect(page.mounts.assets).toBe(1));
    expect(page.mounts.templates).toBe(0);
    expect(page.queryByLabelText('templates搜索')).toBeNull();
    expect(page.getByRole('tab', { name: '素材' }).getAttribute('aria-selected')).toBe('true');
  });

  test('mounts hidden templates only after explicitly opening tasks and opens tasks on that first mount', async () => {
    const page = fixture();
    fireEvent.click(page.getByRole('button', { name: '模板任务' }));
    expect(await page.findByRole('dialog', { name: '模板任务详情' })).toBeTruthy();
    expect(page.mounts.templates).toBe(1);
    expect(page.getByRole('tab', { name: '素材' }).getAttribute('aria-selected')).toBe('true');
    const templatePanel = page.getByLabelText('templates搜索').closest('[data-resource-header-actions]') as HTMLElement;
    expect(templatePanel.hidden).toBe(true);
    expect(page.getAllByRole('button', { name: '模板任务' })).toHaveLength(1);
    fireEvent.click(page.getByRole('button', { name: '关闭任务' }));
    fireEvent.click(page.getByRole('tab', { name: '模板' }));
    expect(page.queryByRole('dialog', { name: '模板任务详情' })).toBeNull();
    expect(page.mounts.templates).toBe(1);
  });

  test('retains each search, header portal and scrolling element across route tab changes', async () => {
    const page = fixture();
    const assetSearch = page.getByLabelText('assets搜索') as HTMLInputElement;
    const assetScroll = page.container.querySelector('[data-panel-scroll="assets"]') as HTMLElement;
    fireEvent.change(assetSearch, { target: { value: '海报' } });
    assetScroll.scrollTop = 155;
    fireEvent.click(page.getByRole('tab', { name: '模板' }));
    const templateSearch = await page.findByLabelText('templates搜索') as HTMLInputElement;
    const templateScroll = page.container.querySelector('[data-panel-scroll="templates"]') as HTMLElement;
    fireEvent.change(templateSearch, { target: { value: '品牌' } });
    templateScroll.scrollTop = 87;
    expect((assetSearch.closest('[data-resource-header-actions]') as HTMLElement).hidden).toBe(true);
    fireEvent.click(page.getByRole('tab', { name: '素材' }));
    expect(page.getByLabelText('assets搜索')).toBe(assetSearch);
    expect(assetSearch.value).toBe('海报');
    expect(assetScroll.scrollTop).toBe(155);
    expect(page.container.querySelector('[data-panel-scroll="assets"]')).toBe(assetScroll);
    expect((templateSearch.closest('[data-resource-header-actions]') as HTMLElement).hidden).toBe(true);
    fireEvent.click(page.getByRole('tab', { name: '模板' }));
    expect(page.getByLabelText('templates搜索')).toBe(templateSearch);
    expect(templateSearch.value).toBe('品牌');
    expect(templateScroll.scrollTop).toBe(87);
    expect(page.mounts).toEqual({ assets: 1, templates: 1 });
  });

  test('surfaces a hidden template load failure through the task entry and opens its recovery panel', async () => {
    spyOn(console, 'error').mockImplementation(() => {});
    const page = fixture(MATERIALS_PATH, true);
    fireEvent.click(page.getByRole('button', { name: '模板任务' }));
    const failure = await page.findByRole('button', { name: '模板任务加载失败' });
    expect(page.getByRole('tab', { name: '素材' }).getAttribute('aria-selected')).toBe('true');
    expect(failure.getAttribute('aria-busy')).not.toBe('true');
    fireEvent.click(failure);
    expect(page.getByRole('tab', { name: '模板' }).getAttribute('aria-selected')).toBe('true');
    expect(page.getByRole('alert').textContent).toContain('模板预览加载失败');
    expect(page.getByRole('button', { name: '重试' })).toBeTruthy();
  });

  test('supports keyboard tabs and leaves prompts outside the resource space parent', () => {
    const page = fixture();
    fireEvent.keyDown(page.getByRole('tab', { name: '素材' }), { key: 'End' });
    expect(page.getByRole('tab', { name: '模板' }).getAttribute('aria-selected')).toBe('true');
    fireEvent.keyDown(page.getByRole('tab', { name: '模板' }), { key: 'ArrowRight' });
    expect(page.getByRole('tab', { name: '素材' }).getAttribute('aria-selected')).toBe('true');
    fireEvent.click(page.getByRole('button', { name: '打开独立提示词库' }));
    expect(page.getByRole('heading', { name: '提示词库' })).toBeTruthy();
    expect(page.queryByRole('tablist')).toBeNull();
    expect(page.container.querySelector('[data-resource-space]')).toBeNull();
    expect(page.container.querySelector('[data-resource-header-actions]')).toBeNull();
  });
});
