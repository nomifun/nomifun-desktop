/**
 * SkillsHubSettings — The Skills Hub page. Every skill (built-in, custom,
 * extension, auto-injected) lives in one responsive searchable card grid.
 *
 * The page owns import, inspection, and deletion. Saved Agent authoring stays
 * in the Agent Workbench.
 * Theme variables only; `<div onClick>`/Arco controls (no <button>).
 */
import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { resolveLocaleKey } from '@/common/utils';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import type { SkillInfo } from '@/common/types/skill';
import AgentSkillImportDrawer from './skill/AgentSkillImportDrawer';
import type { ExternalAgentSkillSource } from './skill/agentSkillImportUtils';
import SkillCard from './skill/SkillCard';
import SkillDetailDrawer from './skill/SkillDetailDrawer';
import { resolveSkillDisplay } from './skill/skillDisplay';
import {
  ENHANCED_TOOLS_EMPTY_STATE_CLASS,
  ENHANCED_TOOLS_GRID_CLASS,
  ENHANCED_TOOLS_HEADER_CLASS,
  ENHANCED_TOOLS_PAGE_STACK_CLASS,
  ENHANCED_TOOLS_SURFACE_CLASS,
} from './enhancedToolsLayout';
import { Button, Input, Modal } from '@arco-design/web-react';
import { CloseSmall, FileZip, FolderOpen, Info, Refresh, Search } from '@icon-park/react';
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSearchParams } from 'react-router-dom';

/**
 * 卡片网格按「内容容器实际宽度」自动定列(auto-fill),而非视口断点 —— 设置内容
 * 面板被一级 rail + 二级 ContentSider 占去宽度。镜像 PresetListPanel 的常量。
 * Card grid auto-fits columns to the actual container width (not viewport
 * breakpoints); copied from PresetListPanel so both surfaces sit on the same
 * 232px lower bound.
 */
const CARD_GRID_COLS = 'repeat(auto-fill, minmax(min(232px, 100%), 1fr))';
const IMPORT_ACTION_BUTTON_CLASS =
  '!rounded-[100px] !h-34px !px-14px !text-t-primary flex items-center gap-6px';

const SkillsHubSettings: React.FC = () => {
  const { t, i18n } = useTranslation();
  const localeKey = resolveLocaleKey(i18n.language);
  const [message, messageContext] = useArcoMessage({ maxCount: 10 });

  const [searchParams, setSearchParams] = useSearchParams();
  const highlightName = searchParams.get('highlight');
  const [highlightedSkill, setHighlightedSkill] = useState<string | null>(null);
  const skillRefs = useRef<Record<string, HTMLDivElement | null>>({});

  const [loading, setLoading] = useState(false);
  const [availableSkills, setAvailableSkills] = useState<SkillInfo[]>([]);
  const [skillPaths, setSkillPaths] = useState<{ user_skills_dir: string; builtin_skills_dir: string } | null>(null);
  const [builtinAutoSkills, setBuiltinAutoSkills] = useState<Array<{ name: string; description: string }>>([]);

  const [search_query, setSearchQuery] = useState('');
  const [searchExpanded, setSearchExpanded] = useState(false);
  const [agentImportVisible, setAgentImportVisible] = useState(false);
  const [detailSkill, setDetailSkill] = useState<SkillInfo | null>(null);

  // Name set of built-in auto-inject skills → drives the "Auto" badge.
  const autoInjectedNames = useMemo(
    () => new Set(builtinAutoSkills.map((s) => s.name)),
    [builtinAutoSkills]
  );

  const fetchData = useCallback(async () => {
    setLoading(true);
    try {
      const [skills, paths, autoSkills] = await Promise.all([
        ipcBridge.fs.listAvailableSkills.invoke(),
        ipcBridge.fs.getSkillPaths.invoke(),
        ipcBridge.fs.listBuiltinAutoSkills.invoke(),
      ]);
      setAvailableSkills(skills as SkillInfo[]);
      setSkillPaths(paths);
      setBuiltinAutoSkills(autoSkills);
    } catch (error) {
      console.error('Failed to fetch skills:', error);
      message.error(t('settings.skillsHub.fetchError', { defaultValue: 'Failed to fetch skills' }));
    } finally {
      setLoading(false);
    }
  }, [t, message]);

  useEffect(() => {
    void fetchData();
  }, [fetchData]);

  const filteredSkills = useMemo(() => {
    const query = search_query.trim().toLowerCase();
    if (!query) return availableSkills;
    return availableSkills.filter((skill) => {
      const display = resolveSkillDisplay(skill, localeKey);
      return `${skill.name} ${skill.description} ${display.name} ${display.description}`
        .toLowerCase()
        .includes(query);
    });
  }, [availableSkills, search_query, localeKey]);

  // Scroll to and highlight a skill when navigated with ?highlight=skillName.
  useEffect(() => {
    if (!highlightName || loading) return;
    const el = skillRefs.current[highlightName];
    if (!el) return;
    requestAnimationFrame(() => {
      el.scrollIntoView({ behavior: 'smooth', block: 'center' });
      setHighlightedSkill(highlightName);
      const timer = setTimeout(() => setHighlightedSkill(null), 2000);
      const next = new URLSearchParams(searchParams);
      next.delete('highlight');
      setSearchParams(next, { replace: true });
      return () => clearTimeout(timer);
    });
  }, [highlightName, loading, filteredSkills, searchParams, setSearchParams]);

  const handleImport = async (skillPath: string) => {
    try {
      const result = await ipcBridge.fs.importSkillWithSymlink.invoke({ skill_path: skillPath });
      const importedNames = result.skill_names?.length
        ? result.skill_names
        : result.skill_name
          ? [result.skill_name]
          : [];
      const count = importedNames.length;
      const names = importedNames.join(', ');
      message.success(
        t('settings.skillsHub.importSuccessDetailed', {
          count,
          names,
          defaultValue: count > 1 ? `Imported ${count} skills: ${names}` : `Imported skill: ${names}`,
        })
      );
      setSearchQuery('');
      void fetchData();
    } catch (error) {
      console.error('Failed to import skill:', error);
      const detail = isBackendHttpError(error) ? error.backendMessage : '';
      message.error(
        detail
          ? t('settings.skillsHub.importErrorDetailed', { detail, defaultValue: `Error importing skill: ${detail}` })
          : t('settings.skillsHub.importError', { defaultValue: 'Error importing skill' })
      );
    }
  };

  const loadAgentSkillSources = useCallback(async (): Promise<ExternalAgentSkillSource[]> => {
    return (await ipcBridge.fs.detectAndCountExternalSkills.invoke()) as ExternalAgentSkillSource[];
  }, []);

  const handleAgentSkillsImported = useCallback(async () => {
    setSearchQuery('');
    await fetchData();
  }, [fetchData]);

  const handleDelete = async (skillName: string) => {
    try {
      await ipcBridge.fs.deleteSkill.invoke({ skill_name: skillName });
      message.success(t('settings.skillsHub.deleteSuccess', { defaultValue: 'Skill deleted' }));
      void fetchData();
    } catch (error) {
      console.error('Failed to delete skill:', error);
      const detail = isBackendHttpError(error) ? error.backendMessage : '';
      message.error(
        detail
          ? t('settings.skillsHub.deleteErrorDetailed', { detail, defaultValue: `Error deleting skill: ${detail}` })
          : t('settings.skillsHub.deleteError', { defaultValue: 'Error deleting skill' })
      );
    }
  };

  const confirmDelete = (skill: SkillInfo) => {
    const display = resolveSkillDisplay(skill, localeKey);
    Modal.confirm({
      title: t('settings.skillsHub.deleteConfirmTitle', { defaultValue: 'Delete Skill' }),
      content: t('settings.skillsHub.deleteConfirmContent', {
        name: display.name,
        defaultValue: `Are you sure you want to delete "${display.name}"?`,
      }),
      okButtonProps: { status: 'danger' },
      okText: t('common.delete', { defaultValue: 'Delete' }),
      onOk: () => void handleDelete(skill.name),
      wrapClassName: 'modal-delete-skill',
    });
  };

  // Tauri's open() cannot offer file + directory selection in one dialog, so
  // folder import and .zip import are split into two explicit actions. Both
  // feed handleImport; the backend's is_zip_path routes by extension.
  const handleImportFolder = async () => {
    try {
      const result = await ipcBridge.dialog.showOpen.invoke({ properties: ['openDirectory'] });
      if (result && result.length > 0) await handleImport(result[0]);
    } catch (error) {
      console.error('Failed to open folder dialog:', error);
    }
  };

  const handleImportZip = async () => {
    try {
      const result = await ipcBridge.dialog.showOpen.invoke({
        properties: ['openFile'],
        filters: [{ name: 'Skill zip archives', extensions: ['zip'] }],
      });
      if (result && result.length > 0) await handleImport(result[0]);
    } catch (error) {
      console.error('Failed to open zip dialog:', error);
    }
  };

  const isSearchVisible = searchExpanded || search_query.length > 0;

  const mainContent = (
    <div className='flex flex-col h-full w-full'>
      {messageContext}
      <div className={ENHANCED_TOOLS_PAGE_STACK_CLASS}>
        <div
          data-testid='skills-library-surface'
          className={ENHANCED_TOOLS_SURFACE_CLASS}
        >
          {/* Header: description + actions */}
          <div className={ENHANCED_TOOLS_HEADER_CLASS}>
            <div
              data-testid='skills-library-header-row'
              className='flex items-center justify-between gap-12px'
            >
              <div className='min-w-0'>
                <p
                  data-testid='skills-library-description'
                  className='m-0 max-w-[680px] text-14px text-t-secondary leading-relaxed'
                >
                  {t('settings.skillsHub.gridDescription', {
                    defaultValue:
                      'Reusable Skill packages that Agents can use during a Session.',
                  })}
                </p>
              </div>
              <div
                data-testid='skills-library-actions'
                className='flex flex-shrink-0 items-center gap-10px'
              >
                <Button
                  type={isSearchVisible ? 'secondary' : 'text'}
                  size='small'
                  data-testid='btn-search-toggle'
                  className='!rounded-10px !h-34px !w-34px !p-0 flex items-center justify-center !text-t-secondary hover:!bg-fill-1 hover:!text-t-primary'
                  icon={
                    isSearchVisible ? <CloseSmall size={16} fill='currentColor' /> : <Search size={16} fill='currentColor' />
                  }
                  onClick={() => {
                    if (isSearchVisible) {
                      setSearchExpanded(false);
                      setSearchQuery('');
                      return;
                    }
                    setSearchExpanded(true);
                  }}
                />
                <Button
                  type='text'
                  size='small'
                  data-testid='btn-refresh-skills'
                  className='!rounded-10px !h-34px !w-34px !p-0 flex items-center justify-center !text-t-secondary hover:!bg-fill-1 hover:!text-t-primary'
                  icon={<Refresh size={16} fill='currentColor' className={loading ? 'animate-spin' : ''} />}
                  onClick={async () => {
                    await fetchData();
                    message.success(t('common.refreshSuccess', { defaultValue: 'Refreshed' }));
                  }}
                  title={t('common.refresh', { defaultValue: 'Refresh' })}
                />
              </div>
            </div>

            {isSearchVisible && (
              <Input
                allowClear
                autoFocus
                value={search_query}
                onChange={setSearchQuery}
                data-testid='input-search-skills'
                className='!bg-[var(--color-bg-2)]'
                placeholder={t('settings.skillsHub.searchPlaceholder', { defaultValue: 'Search skills...' })}
                prefix={<Search size={14} fill='currentColor' />}
              />
            )}

            <div
              data-testid='skills-import-actions'
              className='flex items-center justify-end gap-8px'
            >
              <Button
                size='small'
                data-testid='btn-import-agent-skills'
                className={IMPORT_ACTION_BUTTON_CLASS}
                icon={<FolderOpen size={14} fill='currentColor' />}
                onClick={() => setAgentImportVisible(true)}
              >
                {t('settings.agentSkillImport.shortAction', { defaultValue: 'Import from Agent' })}
              </Button>
              <Button
                size='small'
                data-testid='btn-manual-import'
                className={IMPORT_ACTION_BUTTON_CLASS}
                icon={<FolderOpen size={14} fill='currentColor' />}
                onClick={handleImportFolder}
              >
                {t('settings.skillsHub.manualImport', { defaultValue: 'Import Skills' })}
              </Button>
              <Button
                size='small'
                data-testid='btn-import-zip'
                className={IMPORT_ACTION_BUTTON_CLASS}
                icon={<FileZip size={14} fill='currentColor' />}
                onClick={handleImportZip}
              >
                {t('settings.skillsHub.importZip', { defaultValue: 'Import .zip' })}
              </Button>
            </div>
          </div>

          {/* Single card grid for all skills */}
          {filteredSkills.length > 0 ? (
            <div className={ENHANCED_TOOLS_GRID_CLASS} style={{ gridTemplateColumns: CARD_GRID_COLS }}>
              {filteredSkills.map((skill) => (
                <SkillCard
                  key={skill.name}
                  skill={skill}
                  localeKey={localeKey}
                  isAutoInjected={autoInjectedNames.has(skill.name)}
                  onOpenDetails={setDetailSkill}
                  onDelete={confirmDelete}
                  highlighted={highlightedSkill === skill.name}
                  cardRef={(el) => {
                    skillRefs.current[skill.name] = el;
                  }}
                />
              ))}
            </div>
          ) : (
            <div className={ENHANCED_TOOLS_EMPTY_STATE_CLASS}>
              {loading
                ? t('common.loading', { defaultValue: 'Please wait...' })
                : availableSkills.length === 0
                  ? t('settings.skillsHub.noSkills', { defaultValue: 'No skills found. Import some to get started.' })
                  : t('settings.skillsHub.noMatch', { defaultValue: 'No skills match the current filters.' })}
            </div>
          )}

          {/* Skill directory path */}
          {skillPaths && (
            <div className='mt-12px flex items-center gap-8px text-12px text-t-tertiary font-mono'>
              <FolderOpen size={14} className='shrink-0' />
              <span className='truncate' title={skillPaths.user_skills_dir} data-testid='skill-paths-display'>
                {skillPaths.user_skills_dir}
              </span>
            </div>
          )}
        </div>

        {/* Usage tip */}
        <div className='flex items-start gap-10px rounded-16px border border-solid border-[var(--border-base)] bg-base px-14px py-12px text-t-secondary shadow-sm md:px-16px'>
          <Info size={18} className='text-primary-6 mt-2px shrink-0' />
          <div className='flex flex-col gap-4px'>
            <span className='font-bold text-t-primary text-14px'>
              {t('settings.skillsHub.tipTitle', { defaultValue: 'Usage Tip:' })}
            </span>
            <span className='text-13px leading-relaxed'>{t('settings.skillsHub.tipContent')}</span>
          </div>
        </div>
      </div>

      <SkillDetailDrawer
        visible={detailSkill !== null}
        skill={detailSkill}
        localeKey={localeKey}
        isAutoInjected={
          detailSkill !== null &&
          autoInjectedNames.has(detailSkill.name)
        }
        onClose={() => setDetailSkill(null)}
      />

      <AgentSkillImportDrawer
        visible={agentImportVisible}
        onClose={() => setAgentImportVisible(false)}
        existingSkillNames={availableSkills.map((skill) => skill.name)}
        onImported={handleAgentSkillsImported}
        loadSources={loadAgentSkillSources}
      />
    </div>
  );

  return mainContent;
};

export default SkillsHubSettings;
