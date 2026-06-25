/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { ipcBridge } from '@/common';
import { isBackendHttpError } from '@/common/adapter/httpBridge';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import { Button, Checkbox, Drawer, Tag } from '@arco-design/web-react';
import { CheckSmall, FolderOpen, ImportAndExport, Info, Refresh } from '@icon-park/react';
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import {
  buildAgentSkillRows,
  defaultSelectedAgentSkillKeys,
  summarizeAgentSkillImport,
  type AgentSkillImportRow,
  type ExternalAgentSkillSource,
} from './agentSkillImportUtils';

export type ImportedAgentSkill = {
  name: string;
  description: string;
  path: string;
  source: string;
  sourceName: string;
  alreadyImported: boolean;
};

type AgentSkillImportDrawerProps = {
  visible: boolean;
  onClose: () => void;
  existingSkillNames?: string[];
  onImported?: (skills: ImportedAgentSkill[]) => Promise<void> | void;
  mode?: 'library' | 'assistant';
  loadSources?: () => Promise<ExternalAgentSkillSource[]>;
  importSkills?: (rows: AgentSkillImportRow[]) => Promise<ImportedAgentSkill[]>;
};

const sourceToneClass = (source: string) => {
  if (source === 'claude') return 'bg-[rgba(245,132,38,0.08)] text-[rgb(210,105,30)]';
  if (source === 'gemini') return 'bg-[rgba(var(--primary-6),0.08)] text-primary-6';
  if (source === 'agents') return 'bg-[rgba(var(--success-6),0.1)] text-[rgb(var(--success-6))]';
  return 'bg-fill-2 text-t-secondary';
};

const toImportedSkill = (row: AgentSkillImportRow, name = row.name): ImportedAgentSkill => ({
  name,
  description: row.description,
  path: row.path,
  source: row.source,
  sourceName: row.sourceName,
  alreadyImported: row.alreadyImported,
});

const AgentSkillImportDrawer: React.FC<AgentSkillImportDrawerProps> = ({
  visible,
  onClose,
  existingSkillNames = [],
  onImported,
  mode = 'library',
  loadSources,
  importSkills,
}) => {
  const { t } = useTranslation();
  const [message, messageContext] = useArcoMessage({ maxCount: 5 });
  const [sources, setSources] = useState<ExternalAgentSkillSource[]>([]);
  const [loading, setLoading] = useState(false);
  const [importing, setImporting] = useState(false);
  const [selectedKeys, setSelectedKeys] = useState<string[]>([]);
  const existingNames = useMemo(() => new Set(existingSkillNames), [existingSkillNames]);
  const rows = useMemo(() => buildAgentSkillRows(sources, existingNames), [sources, existingNames]);
  const selectedKeySet = useMemo(() => new Set(selectedKeys), [selectedKeys]);
  const selectedRows = useMemo(() => rows.filter((row) => selectedKeySet.has(row.key)), [rows, selectedKeySet]);
  const summary = useMemo(() => summarizeAgentSkillImport(rows, selectedRows), [rows, selectedRows]);
  const rowSignature = useMemo(
    () => rows.map((row) => `${row.key}:${row.alreadyImported ? '1' : '0'}`).join('|'),
    [rows]
  );

  const fetchSources = useCallback(async () => {
    setLoading(true);
    try {
      const detected = loadSources ? await loadSources() : await ipcBridge.fs.detectAndCountExternalSkills.invoke();
      setSources(detected as ExternalAgentSkillSource[]);
    } catch (error) {
      console.error('Failed to detect external agent skills:', error);
      const detail = isBackendHttpError(error) ? error.backendMessage : '';
      message.error(
        detail
          ? t('settings.agentSkillImport.detectErrorDetailed', {
              detail,
              defaultValue: `Unable to scan agent skills: ${detail}`,
            })
          : t('settings.agentSkillImport.detectError', { defaultValue: 'Unable to scan agent skills' })
      );
    } finally {
      setLoading(false);
    }
  }, [loadSources, message, t]);

  useEffect(() => {
    if (visible) {
      void fetchSources();
    }
  }, [fetchSources, visible]);

  useEffect(() => {
    if (!visible) return;
    setSelectedKeys(defaultSelectedAgentSkillKeys(rows));
  }, [rowSignature, visible]);

  const toggleRow = (row: AgentSkillImportRow) => {
    setSelectedKeys((prev) => (prev.includes(row.key) ? prev.filter((key) => key !== row.key) : [...prev, row.key]));
  };

  const handleSelectAll = (checked: boolean) => {
    setSelectedKeys(checked ? rows.map((row) => row.key) : []);
  };

  const handleImport = async () => {
    if (selectedRows.length === 0) return;
    setImporting(true);
    try {
      const imported: ImportedAgentSkill[] = importSkills ? await importSkills(selectedRows) : [];
      if (!importSkills) {
        for (const row of selectedRows) {
          if (row.alreadyImported) {
            imported.push(toImportedSkill(row));
            continue;
          }
          const result = await ipcBridge.fs.importSkillWithSymlink.invoke({ skill_path: row.path });
          const names = result.skill_names?.length ? result.skill_names : result.skill_name ? [result.skill_name] : [row.name];
          for (const name of names) {
            imported.push(toImportedSkill(row, name));
          }
        }
      }
      await onImported?.(imported);
      message.success(
        mode === 'assistant'
          ? t('settings.agentSkillImport.assistantSuccess', {
              count: imported.length,
              defaultValue: `Added ${imported.length} skills to this assistant`,
            })
          : t('settings.agentSkillImport.librarySuccess', {
              count: imported.length,
              defaultValue: `Imported ${imported.length} agent skills`,
            })
      );
      onClose();
    } catch (error) {
      console.error('Failed to import agent skills:', error);
      const detail = isBackendHttpError(error) ? error.backendMessage : '';
      message.error(
        detail
          ? t('settings.agentSkillImport.importErrorDetailed', {
              detail,
              defaultValue: `Unable to import agent skills: ${detail}`,
            })
          : t('settings.agentSkillImport.importError', { defaultValue: 'Unable to import agent skills' })
      );
    } finally {
      setImporting(false);
    }
  };

  const allSelected = rows.length > 0 && selectedRows.length === rows.length;

  return (
    <Drawer
      visible={visible}
      onCancel={onClose}
      width={680}
      zIndex={1300}
      placement='right'
      title={t('settings.agentSkillImport.title', { defaultValue: 'Import from Agent' })}
      className='agent-skill-import-drawer'
      data-testid='agent-skill-import-drawer'
      footer={
        <div className='flex items-center justify-between w-full'>
          <div className='text-12px text-t-secondary'>
            {t('settings.agentSkillImport.selectionSummary', {
              selected: summary.selectedCount,
              importable: summary.importableCount,
              existing: summary.alreadyImportedCount,
              defaultValue: `${summary.selectedCount} selected · ${summary.importableCount} new · ${summary.alreadyImportedCount} already in library`,
            })}
          </div>
          <div className='flex items-center gap-8px'>
            <Button onClick={onClose} className='rounded-[100px]'>
              {t('common.cancel', { defaultValue: 'Cancel' })}
            </Button>
            <Button
              type='primary'
              loading={importing}
              disabled={selectedRows.length === 0}
              onClick={handleImport}
              className='rounded-[100px]'
              icon={<ImportAndExport size={14} fill='currentColor' />}
              data-testid='btn-confirm-agent-skill-import'
            >
              {mode === 'assistant'
                ? t('settings.agentSkillImport.addToAssistant', { defaultValue: 'Add to assistant' })
                : t('settings.agentSkillImport.importSelected', { defaultValue: 'Import selected' })}
            </Button>
          </div>
        </div>
      }
    >
      {messageContext}
      <div className='flex flex-col gap-16px' data-testid='agent-skill-import-content'>
        <div className='flex items-start gap-10px p-12px rd-10px bg-fill-2 shadow-[inset_0_0_0_1px_rgba(var(--primary-6),0.08)]'>
          <Info size={16} className='mt-2px text-primary-6 flex-shrink-0' />
          <div className='text-13px leading-20px text-t-secondary'>
            {t('settings.agentSkillImport.description', {
              defaultValue:
                'Bring reusable skills from Claude, Gemini, Codex-compatible Agent Skills, or custom external folders into Nomi.',
            })}
          </div>
        </div>

        <div className='flex items-center justify-between gap-10px'>
          <div className='text-14px font-600 text-t-primary'>
            {t('settings.agentSkillImport.sources', { defaultValue: 'Detected sources' })}
          </div>
          <Button
            size='small'
            type='text'
            onClick={fetchSources}
            loading={loading}
            icon={<Refresh size={14} fill='currentColor' />}
            className='!rounded-10px'
            data-testid='btn-refresh-agent-skills'
          >
            {t('common.refresh', { defaultValue: 'Refresh' })}
          </Button>
        </div>

        {sources.length > 0 && (
          <div className='grid gap-8px' style={{ gridTemplateColumns: 'repeat(auto-fill, minmax(180px, 1fr))' }}>
            {sources.map((source) => (
              <div key={`${source.source}:${source.path}`} className='p-10px rd-8px bg-fill-2 min-w-0'>
                <div className='flex items-center gap-8px min-w-0'>
                  <FolderOpen size={15} className='text-t-secondary flex-shrink-0' />
                  <span className='truncate text-13px font-600 text-t-primary' title={source.name}>
                    {source.name}
                  </span>
                </div>
                <div className='mt-6px flex items-center gap-6px'>
                  <Tag size='small' bordered={false} className={sourceToneClass(source.source)}>
                    {source.skill_count ?? source.skills.length}
                  </Tag>
                  <span className='truncate text-11px text-t-tertiary' title={source.path}>
                    {source.path}
                  </span>
                </div>
              </div>
            ))}
          </div>
        )}

        <div className='flex items-center justify-between'>
          <Checkbox checked={allSelected} indeterminate={selectedRows.length > 0 && !allSelected} onChange={handleSelectAll}>
            {t('settings.agentSkillImport.selectAll', { defaultValue: 'Select all' })}
          </Checkbox>
          <span className='text-12px text-t-secondary'>
            {t('settings.agentSkillImport.count', {
              count: rows.length,
              defaultValue: `${rows.length} skills`,
            })}
          </span>
        </div>

        <div className='rd-12px overflow-hidden bg-fill-1 shadow-[inset_0_0_0_1px_rgba(var(--primary-6),0.10)]'>
          {rows.length > 0 ? (
            <div className='max-h-[420px] overflow-auto divide-y divide-[rgba(var(--primary-6),0.10)]'>
              {rows.map((row) => (
                <div
                  key={row.key}
                  className='flex items-start gap-10px p-10px transition-colors hover:bg-[rgba(var(--primary-6),0.04)]'
                  data-testid={`agent-skill-import-row-${row.source}-${row.name}`}
                >
                  <Checkbox checked={selectedKeySet.has(row.key)} onChange={() => toggleRow(row)} className='mt-2px' />
                  <div className='min-w-0 flex-1'>
                    <div className='flex items-center gap-8px min-w-0'>
                      <span className='text-13px font-600 text-t-primary truncate' title={row.name}>
                        {row.name}
                      </span>
                      <Tag size='small' bordered={false} className={sourceToneClass(row.source)}>
                        {row.sourceName}
                      </Tag>
                      {row.alreadyImported && (
                        <Tag size='small' bordered={false} className='!bg-[rgba(var(--success-6),0.1)] !text-[rgb(var(--success-6))]'>
                          <span className='inline-flex items-center gap-3px'>
                            <CheckSmall size={12} fill='currentColor' />
                            {t('settings.agentSkillImport.alreadyImported', { defaultValue: 'In library' })}
                          </span>
                        </Tag>
                      )}
                    </div>
                    <div className='mt-3px text-12px text-t-secondary line-clamp-2'>
                      {row.description ||
                        t('settings.skillsHub.noDescription', { defaultValue: 'No description provided.' })}
                    </div>
                    <div className='mt-4px text-11px text-t-tertiary truncate font-mono' title={row.path}>
                      {row.path}
                    </div>
                  </div>
                </div>
              ))}
            </div>
          ) : (
            <div className='py-36px px-16px text-center text-t-secondary'>
              {loading
                ? t('common.loading', { defaultValue: 'Please wait...' })
                : t('settings.agentSkillImport.empty', {
                    defaultValue: 'No external agent skills found. Add a custom folder or create skills in an agent skills directory.',
                  })}
            </div>
          )}
        </div>
      </div>
    </Drawer>
  );
};

export default AgentSkillImportDrawer;
