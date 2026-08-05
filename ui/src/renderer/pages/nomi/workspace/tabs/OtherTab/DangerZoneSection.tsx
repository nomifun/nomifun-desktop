/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Message, Modal } from '@arco-design/web-react';
import { Attention } from '@icon-park/react';
import { ipcBridge } from '@/common';
import { NomiSettingList, NomiSettingRow, NomiSettingSection } from '@/renderer/components/base/NomiSettingLayout';
import type { CompanionId } from '@/common/types/ids';
import { errText } from './bundleIo';

interface Props {
  companionId: CompanionId;
  companionName: string;
}

/**
 * 危险操作 — the one irreversible action on this page.
 *
 * What `DELETE /api/companion/companions/:id` really destroys (service.rs
 * `delete_companion` + store.rs `delete_companion_rows`): the profile, this
 * companion's own memories and skills, its runtime state (xp = 成长进度), its
 * session windows and — cascaded — its conversation, i.e. the chat history. The
 * copy below lists exactly that set and nothing more: it does NOT claim the
 * 迁移 bundle above can bring any of it back, because that bundle carries only
 * 设定 + 成长进度 + knowledge-base names (see export.rs
 * `export_companion_bundle`). It also deliberately drops the old copy's promise
 * that "shared memories are unaffected", which was a statement about a scope
 * this surface no longer exposes.
 *
 * Deletion is the shell's business once the endpoint returns: the roster is
 * reconciled by the `companion.deleted` WS event (see useNomi's useCompanions),
 * so this section neither navigates nor takes an onDeleted callback.
 */
const DangerZoneSection: React.FC<Props> = ({ companionId, companionName }) => {
  const { t } = useTranslation();

  const confirmDelete = useCallback(() => {
    Modal.confirm({
      title: t('nomi.other.deleteConfirmTitle', { companionName, defaultValue: '删除「{{companionName}}」？' }),
      content: t('nomi.other.deleteConfirmBody', {
        companionName,
        defaultValue:
          '「{{companionName}}」的设定、它在「记忆」里的全部记忆、它的技能、成长进度与聊天记录会一起永久删除，无法恢复。上面的迁移包只保存设定与成长进度，记忆和聊天记录不在包内，导出也留不下它们。',
      }),
      okText: t('nomi.other.deleteConfirmOk', { defaultValue: '永久删除' }),
      okButtonProps: { status: 'danger' },
      onOk: async () => {
        try {
          await ipcBridge.companion.deleteCompanion.invoke({ companion_id: companionId });
          Message.success(t('nomi.settings.deleted', { companionName, defaultValue: '{{companionName}} 已离开' }));
        } catch (e) {
          Message.error(errText(e));
        }
      },
    });
  }, [companionId, companionName, t]);

  return (
    <NomiSettingSection
      title={t('nomi.other.dangerSection', { defaultValue: '危险操作' })}
      description={t('nomi.other.dangerSectionDesc', { defaultValue: '这里的操作不可撤销，执行前请先确认。' })}
    >
      <NomiSettingList>
        <NomiSettingRow
          leading={
            <Attention
              theme='filled'
              size='14'
              fill='currentColor'
              strokeWidth={3}
              className='line-height-0 shrink-0 text-danger-6'
            />
          }
          title={t('nomi.settings.deleteCompanion', { defaultValue: '删除伙伴' })}
          description={t('nomi.other.deleteHint', {
            companionName,
            defaultValue: '永久删除「{{companionName}}」，连同它的记忆、技能、成长进度与聊天记录。',
          })}
          controls={
            <Button status='danger' onClick={confirmDelete}>
              {t('nomi.settings.deleteCompanion', { defaultValue: '删除伙伴' })}
            </Button>
          }
        />
      </NomiSettingList>
    </NomiSettingSection>
  );
};

export default DangerZoneSection;
