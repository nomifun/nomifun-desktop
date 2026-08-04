/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Input, Message } from '@arco-design/web-react';
import NomiSelect from '@/renderer/components/base/NomiSelect';
import { NomiSettingList, NomiSettingRow } from '@/renderer/components/base/NomiSettingLayout';
import type { ICompanionMemoryKind } from '@/common/adapter/ipcBridge';
import { MEMORY_KINDS } from './constants';

interface MemoryComposePaneProps {
  onSubmit: (kind: ICompanionMemoryKind, content: string) => Promise<void>;
  onDone: () => void;
}

/**
 * Add one memory. There is no scope choice: a memory written here belongs to the
 * companion the workspace is showing, full stop.
 */
const MemoryComposePane: React.FC<MemoryComposePaneProps> = ({ onSubmit, onDone }) => {
  const { t } = useTranslation();
  const [kind, setKind] = useState<ICompanionMemoryKind>('knowledge');
  const [content, setContent] = useState('');
  const [saving, setSaving] = useState(false);

  const ready = content.trim().length > 0 && !saving;

  const submit = async () => {
    if (!ready) return;
    setSaving(true);
    try {
      await onSubmit(kind, content.trim());
      Message.success(t('nomi.memories.added', { defaultValue: '记忆已添加' }));
      onDone();
    } catch (e) {
      Message.error(String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className='flex flex-col gap-16px'>
      <NomiSettingList>
        <NomiSettingRow
          title={t('nomi.memory.metaKind', { defaultValue: '分类' })}
          description={t('nomi.memory.composeKindHint', { defaultValue: '分类决定这条记忆在检索时的权重与衰减方式。' })}
          controls={
            <NomiSelect
              contentFit
              contentMaxWidth={150}
              value={kind}
              onChange={(value: ICompanionMemoryKind) => setKind(value)}
            >
              {MEMORY_KINDS.map((item) => (
                <NomiSelect.Option key={item} value={item}>
                  {t(`nomi.kinds.${item}`)}
                </NomiSelect.Option>
              ))}
            </NomiSelect>
          }
        />
      </NomiSettingList>

      <Input.TextArea
        value={content}
        onChange={setContent}
        autoSize={{ minRows: 6, maxRows: 14 }}
        placeholder={t('nomi.memories.addPlaceholder', { defaultValue: '告诉 nomi 一件它应该记住的事…' })}
        className='!rd-8px text-13px leading-20px'
      />

      <div
        role='button'
        tabIndex={ready ? 0 : -1}
        aria-disabled={!ready}
        onClick={() => void submit()}
        onKeyDown={(event) => {
          if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault();
            void submit();
          }
        }}
        className={[
          'inline-flex select-none items-center justify-center rd-full px-18px py-9px text-13px font-700 leading-none transition-colors',
          ready
            ? 'cursor-pointer bg-[rgba(var(--primary-6),0.12)] text-[var(--color-text-1)] shadow-[0_6px_18px_rgba(var(--primary-6),0.14)] hover:bg-[rgba(var(--primary-6),0.18)]'
            : 'cursor-not-allowed bg-fill-2 text-t-tertiary',
        ].join(' ')}
      >
        {t('nomi.memories.add', { defaultValue: '添加记忆' })}
      </div>
    </div>
  );
};

export default MemoryComposePane;
