/** One management destination; skills and MCP retain their independent owners. */
import type { IMcpServer } from '@/common/config/storage';
import { CONNECTORS } from '@/common/connectors';
import HubPageShell from '@/renderer/components/layout/HubPageShell';
import { useMcpServers } from '@/renderer/hooks/mcp';
import McpMarketSettings from '@/renderer/pages/mcp/McpMarketSettings';
import SkillMarketSettings from '@/renderer/pages/settings/SkillMarketSettings';
import { Alert, Button, Drawer, Dropdown, Input, Menu, Spin } from '@arco-design/web-react';
import { Down, FileZip, FolderOpen, ImportAndExport, Refresh, Search, Shop } from '@icon-park/react';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSearchParams } from 'react-router-dom';
import McpManagementPanel from './McpManagementPanel';
import ConnectorsPanel from './ConnectorsPanel';
import SkillsLibraryPanel from './SkillsLibraryPanel';
import { type EnhancedToolsCategory } from './routes';
import { useSkillsLibrary } from './useSkillsLibrary';
import './styles.css';

const CATEGORIES: EnhancedToolsCategory[] = ['mcp', 'skills', 'connectors'];

const getCategory = (params: URLSearchParams): EnhancedToolsCategory => {
  const value = params.get('category');
  return value === 'skills' || value === 'connectors' ? value : 'mcp';
};

export default function EnhancedToolsPage() {
  const { t } = useTranslation();
  const [searchParams, setSearchParams] = useSearchParams();
  const [importMode, setImportMode] = useState<'json' | 'oneclick' | null>(null);
  const category = getCategory(searchParams);
  const isConnectors = category === 'connectors';
  const marketVisible = !isConnectors && searchParams.get('view') === 'market';
  const viewOwner = useMemo(() => ({}), [category, marketVisible, importMode]);
  const currentViewOwner = useRef(viewOwner);
  currentViewOwner.current = viewOwner;
  const mounted = useRef(true);
  const currentParams = useRef(searchParams);
  currentParams.current = searchParams;
  const [mcpSearch, setMcpSearch] = useState('');
  const library = useSkillsLibrary();
  const { mcpServers, isMcpServersLoading, mcpServersLoadFailed, retryMcpServers, saveMcpServers } = useMcpServers();
  const panels = useRef<Partial<Record<EnhancedToolsCategory, HTMLDivElement | null>>>({});
  const scrollPositions = useRef<Record<EnhancedToolsCategory, number>>({ mcp: 0, skills: 0, connectors: 0 });
  const tabRefs = useRef<Partial<Record<EnhancedToolsCategory, HTMLButtonElement | null>>>({});
  const isMcp = category === 'mcp';
  const mcpUnavailable = isMcpServersLoading || mcpServersLoadFailed;
  const searchQuery = isMcp ? mcpSearch : library.searchQuery;
  const searching = isMcp ? isMcpServersLoading : library.loading;
  const marketTitle = isMcp ? t('settings.mcpMarket.title') : t('settings.skillsMarket.title');

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  useLayoutEffect(() => {
    const panel = panels.current[category];
    if (panel) panel.scrollTop = scrollPositions.current[category];
  }, [category]);

  const selectCategory = (next: EnhancedToolsCategory) => {
    if (next === category) return;
    scrollPositions.current[category] = panels.current[category]?.scrollTop ?? 0;
    setSearchParams((previous) => {
      const params = new URLSearchParams(previous);
      params.set('category', next);
      params.delete('view');
      params.delete('highlight');
      return params;
    });
  };

  const setMarketVisible = (visible: boolean) => {
    setSearchParams((previous) => {
      const params = new URLSearchParams(previous);
      params.set('category', category);
      if (visible) params.set('view', 'market');
      else params.delete('view');
      return params;
    });
  };

  const finishMarket = useCallback((expectedCategory: EnhancedToolsCategory, highlight?: string) => {
    // Late installation completion still refreshes its library, but must not
    // replace a destination the user reached while that request was pending.
    if (!mounted.current || currentViewOwner.current !== viewOwner) return;
    const current = currentParams.current;
    if (getCategory(current) !== expectedCategory || current.get('view') !== 'market') return;
    if (expectedCategory === 'mcp') setMcpSearch('');
    else library.setSearchQuery('');
    setSearchParams((previous) => {
      if (getCategory(previous) !== expectedCategory || previous.get('view') !== 'market') return previous;
      const params = new URLSearchParams(previous);
      params.delete('view');
      if (highlight) params.set('highlight', highlight);
      else params.delete('highlight');
      return params;
    }, { replace: true });
  }, [library.setSearchQuery, viewOwner, setSearchParams]);

  const handleMarketInstalled = useCallback((skillNames: string[]) => {
    void library.refresh();
    finishMarket('skills', skillNames[0]);
  }, [finishMarket, library.refresh]);
  const handleMcpImported = useCallback((servers: IMcpServer[]) => {
    finishMarket('mcp', servers[0]?.mcp_server_id);
  }, [finishMarket]);
  const handleManagementImported = useCallback((servers: IMcpServer[]) => {
    if (!mounted.current || currentViewOwner.current !== viewOwner || getCategory(currentParams.current) !== 'mcp') return;
    setMcpSearch('');
    const id = servers[0]?.mcp_server_id;
    if (!id) return;
    setSearchParams((previous) => {
      const params = new URLSearchParams(previous);
      params.set('highlight', id);
      return params;
    }, { replace: true });
  }, [setSearchParams, viewOwner]);

  const addMenu = (
    <Menu className='enhanced-tools-add-menu'>
      {isMcp ? [
        <Menu.Item key='json' disabled={mcpUnavailable} aria-disabled={mcpUnavailable} data-testid='enhanced-tools-import-json' onClick={() => setImportMode('json')}>
          <ImportAndExport size={16} />{t('settings.mcpImportFromJSON')}
        </Menu.Item>,
        <Menu.Item key='oneclick' disabled={mcpUnavailable} aria-disabled={mcpUnavailable} data-testid='enhanced-tools-import-mcp-agent' onClick={() => setImportMode('oneclick')}>
          <FolderOpen size={16} />{t('settings.mcpOneKeyImport')}
        </Menu.Item>,
      ] : [
        <Menu.Item key='agent' data-testid='btn-import-agent-skills' onClick={library.importFromAgent}>
          <ImportAndExport size={16} />{t('settings.agentSkillImport.shortAction')}
        </Menu.Item>,
        <Menu.Item key='folder' data-testid='btn-manual-import' onClick={() => void library.importFolder()}>
          <FolderOpen size={16} />{t('settings.skillsHub.manualImport')}
        </Menu.Item>,
        <Menu.Item key='zip' data-testid='btn-import-zip' onClick={() => void library.importZip()}>
          <FileZip size={16} />{t('settings.skillsHub.importZip')}
        </Menu.Item>,
      ]}
      <Menu.Item key='market' className='enhanced-tools-market-entry' data-testid='enhanced-tools-open-market' onClick={() => setMarketVisible(true)}>
        <Shop size={16} />{marketTitle}
      </Menu.Item>
    </Menu>
  );

  return (
    <HubPageShell
      title={t('settings.enhancedTools.title')}
      className='enhanced-tools-page'
      maxWidthClass='enhanced-tools-container'
      actions={isConnectors ? undefined : (
        <>
          <Input
            allowClear
            value={searchQuery}
            onChange={isMcp ? setMcpSearch : library.setSearchQuery}
            prefix={<Search size={15} />}
            placeholder={isMcp ? t('settings.enhancedTools.searchMcp') : t('settings.skillsHub.searchPlaceholder')}
            aria-label={isMcp ? t('settings.enhancedTools.searchMcp') : t('settings.skillsHub.searchPlaceholder')}
            className='enhanced-tools-search'
            data-testid={isMcp ? 'input-search-mcp' : 'input-search-skills'}
          />
          <Button
            type='text'
            icon={<Refresh size={16} />}
            aria-label={t('common.refresh')}
            title={t('common.refresh')}
            loading={searching}
            onClick={() => isMcp ? retryMcpServers() : void library.refresh()}
            data-testid={isMcp ? 'btn-refresh-mcp' : 'btn-refresh-skills'}
          />
          <Dropdown trigger='click' droplist={addMenu} position='br' key={category}>
            <Button type='primary' className='enhanced-tools-add' data-testid='enhanced-tools-add' aria-haspopup='menu'>
              {t('settings.enhancedTools.add')}<Down size={13} fill='currentColor' />
            </Button>
          </Dropdown>
        </>
      )}
    >
      <div className='enhanced-tools-tabs' role='tablist' aria-label={t('settings.enhancedTools.title')}>
        {CATEGORIES.map((item) => (
          <button
            key={item}
            type='button'
            role='tab'
            id={`enhanced-tools-tab-${item}`}
            aria-controls={`enhanced-tools-panel-${item}`}
            aria-selected={category === item}
            tabIndex={category === item ? 0 : -1}
            ref={(element) => { tabRefs.current[item] = element; }}
            onClick={() => selectCategory(item)}
            onKeyDown={(event) => {
              if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
              event.preventDefault();
              const index = CATEGORIES.indexOf(item);
              const next = event.key === 'Home' ? CATEGORIES[0] : event.key === 'End' ? CATEGORIES[CATEGORIES.length - 1] : CATEGORIES[event.key === 'ArrowLeft' ? Math.max(0, index - 1) : Math.min(CATEGORIES.length - 1, index + 1)];
              selectCategory(next);
              tabRefs.current[next]?.focus();
            }}
          >
            {item === 'mcp' ? 'MCP' : item === 'skills' ? t('settings.enhancedTools.skills') : t('settings.enhancedTools.connectors')}
            <span className='enhanced-tools-count'>
              {item === 'mcp' ? isMcpServersLoading || mcpServersLoadFailed ? '—' : mcpServers.length : item === 'skills' ? library.loading || library.error ? '—' : library.librarySkills.length : CONNECTORS.length}
            </span>
          </button>
        ))}
      </div>
      <div className='enhanced-tools-section-heading'>
        <h2>{isMcp ? t('settings.enhancedTools.servers') : isConnectors ? t('settings.enhancedTools.connectors') : t('settings.enhancedTools.installedSkills')}</h2>
        <span>{isMcp ? t('settings.enhancedTools.mcpHint') : isConnectors ? t('settings.enhancedTools.connectorsHint') : t('settings.enhancedTools.skillsHint')}</span>
      </div>
      <div className='enhanced-tools-panels'>
        <div
          className='enhanced-tools-panel'
          id='enhanced-tools-panel-mcp'
          role='tabpanel'
          aria-labelledby='enhanced-tools-tab-mcp'
          hidden={!isMcp}
          ref={(element) => { panels.current.mcp = element; }}
          onScroll={(event) => { if (isMcp) scrollPositions.current.mcp = event.currentTarget.scrollTop; }}
        >
          {mcpServersLoadFailed && (
            <Alert type='error' content={t('settings.mcpPage.loadFailed')} action={<Button size='mini' onClick={retryMcpServers}>{t('common.retry')}</Button>} />
          )}
          {isMcpServersLoading ? <div className='enhanced-tools-empty'><Spin /></div> : !mcpServersLoadFailed && (
            <McpManagementPanel
              mcpServers={mcpServers}
              saveMcpServers={saveMcpServers}
              searchQuery={mcpSearch}
              importMode={importMode}
              onImportClose={() => {
                if (currentViewOwner.current === viewOwner) setImportMode(null);
              }}
              onImported={handleManagementImported}
              visible={isMcp && !marketVisible}
              highlightServerId={isMcp && !marketVisible ? searchParams.get('highlight') : null}
            />
          )}
        </div>
        <div
          className='enhanced-tools-panel'
          id='enhanced-tools-panel-skills'
          role='tabpanel'
          aria-labelledby='enhanced-tools-tab-skills'
          hidden={category !== 'skills'}
          ref={(element) => { panels.current.skills = element; }}
          onScroll={(event) => { if (category === 'skills') scrollPositions.current.skills = event.currentTarget.scrollTop; }}
        >
          <SkillsLibraryPanel library={library} visible={category === 'skills' && !marketVisible} />
        </div>
        <div
          className='enhanced-tools-panel'
          id='enhanced-tools-panel-connectors'
          role='tabpanel'
          aria-labelledby='enhanced-tools-tab-connectors'
          hidden={!isConnectors}
          ref={(element) => { panels.current.connectors = element; }}
          onScroll={(event) => { if (isConnectors) scrollPositions.current.connectors = event.currentTarget.scrollTop; }}
        >
          {isConnectors && <ConnectorsPanel />}
        </div>
      </div>
      <Drawer
        title={marketTitle}
        visible={marketVisible}
        onCancel={() => setMarketVisible(false)}
        width='min(1100px, calc(100% - 48px))'
        className='enhanced-tools-market-drawer'
        unmountOnExit
        footer={<Button onClick={() => setMarketVisible(false)}>{t('settings.enhancedTools.backToList')}</Button>}
      >
        {marketVisible && (isMcp ? (
          <McpMarketSettings
            mcpServers={mcpServers}
            saveMcpServers={saveMcpServers}
            addedStateLoading={isMcpServersLoading || mcpServersLoadFailed}
            onImported={handleMcpImported}
          />
        ) : <SkillMarketSettings addedStateLoading={library.loading || Boolean(library.error)} onInstalled={handleMarketInstalled} />)}
      </Drawer>
    </HubPageShell>
  );
}
