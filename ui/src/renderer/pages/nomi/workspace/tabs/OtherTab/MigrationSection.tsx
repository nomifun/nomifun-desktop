/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Message } from '@arco-design/web-react';
import { Attention, Computer } from '@icon-park/react';
import { ipcBridge } from '@/common';
import { isTauriRuntime } from '@/common/adapter/tauriRuntime';
import { NomiSettingList, NomiSettingRow, NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import type { CompanionId } from '@/common/types/ids';
import {
  collectKnowledgeNames,
  defaultBundleName,
  errText,
  pickSavePath,
  pickZipPath,
  rebuildKnowledgeBinding,
  type ImportOutcome,
} from './bundleIo';

interface Props {
  companionId: CompanionId;
  companionName: string;
}

/**
 * 迁移 — export this companion to a .zip bundle and restore one from a bundle.
 * Both halves need native file dialogs, so on a web/Docker host the section
 * degrades to a single explanatory row instead of disappearing.
 */
const MigrationSection: React.FC<Props> = ({ companionId, companionName }) => {
  const { t } = useTranslation();
  const [exporting, setExporting] = useState(false);
  const [importing, setImporting] = useState(false);
  /** Knowledge-base names from an imported bundle with no local match. */
  const [unmatched, setUnmatched] = useState<string[]>([]);

  const runExport = async () => {
    const dest = await pickSavePath(defaultBundleName(companionName));
    if (!dest) return;
    setExporting(true);
    try {
      const knowledgeNames = await collectKnowledgeNames(companionId);
      const res = await ipcBridge.companion.exportCompanion.invoke({
        companion_id: companionId,
        dest_path: dest,
        knowledge_names: knowledgeNames,
      });
      Message.success(t('nomi.migrate.exportCompanionOk', { path: res.dest_path }));
    } catch (e) {
      Message.error(errText(e));
    } finally {
      setExporting(false);
    }
  };

  const runImport = async () => {
    const src = await pickZipPath();
    if (!src) return;
    setImporting(true);
    setUnmatched([]);
    try {
      const outcome = (await ipcBridge.companion.importCompanionBundle.invoke({
        src_path: src,
      })) as ImportOutcome;
      if (outcome.kind === 'memory') {
        // Legacy memory package — the backend dispatches on manifest.kind, so
        // report what actually happened rather than pretending it was a companion.
        Message.success(
          t('nomi.migrate.importMemoryOk', {
            imported: outcome.imported,
            skipped: outcome.skipped_duplicates,
          })
        );
        return;
      }
      // The shell's companion.created WS subscription puts the new companion in the roster.
      Message.success(t('nomi.migrate.importCompanionOk', { name: outcome.name }));
      try {
        const { matched, unmatched: missing } = await rebuildKnowledgeBinding(outcome);
        if (matched) Message.success(t('nomi.migrate.bindingRebuilt', { count: matched }));
        setUnmatched(missing);
      } catch (e) {
        // Rebuild failed — surface every name so the user can bind manually.
        setUnmatched(outcome.knowledge_names);
        Message.error(errText(e));
      }
    } catch (e) {
      Message.error(errText(e));
    } finally {
      setImporting(false);
    }
  };

  if (!isTauriRuntime()) {
    return (
      <NomiSettingSection
        title={t('nomi.other.migrateSection', { defaultValue: '迁移' })}
        description={t('nomi.other.migrateSectionDesc', {
          defaultValue: '把这个伙伴打包带到另一台设备，或从迁移包里恢复一个伙伴。',
        })}
      >
        <NomiSettingList>
          <NomiSettingRow
            leading={
              <Computer
                theme='outline'
                size='14'
                fill='currentColor'
                strokeWidth={3}
                className='line-height-0 shrink-0 text-t-tertiary'
              />
            }
            title={t('nomi.other.desktopOnlyTitle', { defaultValue: '仅桌面版可用' })}
            description={t('nomi.migrate.desktopOnly', { defaultValue: '迁移功能仅桌面版支持' })}
          />
        </NomiSettingList>
      </NomiSettingSection>
    );
  }

  return (
    <NomiSettingSection
      title={t('nomi.other.migrateSection', { defaultValue: '迁移' })}
      description={t('nomi.other.migrateSectionDesc', {
        defaultValue: '把这个伙伴打包带到另一台设备，或从迁移包里恢复一个伙伴。',
      })}
    >
      <NomiSettingList>
        <NomiSettingRow
          title={t('nomi.other.exportTitle', { defaultValue: '导出伙伴' })}
          description={t('nomi.other.exportDesc', {
            companionName,
            defaultValue:
              '把「{{companionName}}」打包成 .zip 迁移包，在另一台设备导入即可恢复。包内只有设定、成长进度与已绑定的知识库名单：记忆、技能、聊天记录与自定义形象图片都不在其中。',
          })}
          controls={
            <Button type='primary' loading={exporting} onClick={() => void runExport()}>
              {t('nomi.migrate.exportCompanion', { defaultValue: '导出迁移包' })}
            </Button>
          }
        />
        <NomiSettingRow
          title={t('nomi.other.importTitle', { defaultValue: '导入伙伴' })}
          description={t('nomi.other.importDesc', {
            defaultValue: '选择伙伴迁移包（.zip），会在本机创建一个新伙伴，并按名称恢复它的知识库绑定。',
          })}
          controls={
            <Button loading={importing} onClick={() => void runImport()}>
              {t('nomi.other.importAction', { defaultValue: '选择迁移包' })}
            </Button>
          }
          footer={
            unmatched.length > 0 ? (
              <div className='flex items-start gap-8px text-12px leading-18px text-t-secondary'>
                <Attention
                  theme='outline'
                  size='14'
                  fill='currentColor'
                  strokeWidth={3}
                  className='line-height-0 mt-1px shrink-0 text-warning-6'
                />
                <div className='min-w-0'>
                  <div>
                    {t('nomi.migrate.unmatchedTitle', {
                      defaultValue: '以下知识库不在本机，请先到知识库页面导入对应知识库包，再手动绑定：',
                    })}
                  </div>
                  <div className='mt-4px font-500 text-t-primary'>{unmatched.join('、')}</div>
                </div>
              </div>
            ) : undefined
          }
        />
      </NomiSettingList>
    </NomiSettingSection>
  );
};

export default MigrationSection;
