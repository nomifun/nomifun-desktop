/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import useSWR from 'swr';
import { Button, Input, Popover, Switch, Tag } from '@arco-design/web-react';
import { Edit, LinkCloud } from '@icon-park/react';
import { ipcBridge } from '@/common';
import { configService } from '@/common/config/configService';
import type { ProviderId } from '@/common/types/ids';
import NomiScrollArea from '@/renderer/components/base/NomiScrollArea';
import { NomiSettingList, NomiSettingRow } from '@/renderer/components/base/NomiSettingLayout';
import TaskModelSelect from '@/renderer/components/model/TaskModelSelect';
import { useModelProviderList } from '@/renderer/hooks/agent/useModelProviderList';
import { useModelSelectorProviderLabel } from '@/renderer/hooks/agent/useModelSelectorProviderLabel';
import type { I18nKey } from '@/renderer/services/i18n/i18n-keys';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import {
  buildModalityGroups,
  buildUntaggedGroups,
  MODALITY_SPECS,
  type ModalityKey,
  type ModalityModelRow,
  type ModalityProviderGroup,
} from './modalityModels';

export interface ModalityModelsPanelProps {
  modality: ModalityKey;
  icon: React.ReactNode;
  titleKey: I18nKey;
  subtitleKey: I18nKey;
  /** Render the modality's install-wide default model row. */
  showDefaultModel?: boolean;
  /** Append the "no task tag yet" bucket (the chat section owns it). */
  showUntagged?: boolean;
}

const CATALOG_ROWS_SWR_KEY = 'provider-models.all';

const TASK_LABEL_KEY: Record<string, I18nKey> = {
  chat: 'settings.modelHub.modality.taskChat',
  embedding: 'settings.modelHub.modality.taskEmbedding',
  rerank: 'settings.modelHub.modality.taskRerank',
};

/**
 * One modality section of the model hub: the catalog rows that belong to this
 * modality, grouped by provider, each switchable on/off and describable in place.
 *
 * Task TAGGING is not here on purpose — the tasks/traits editor on the 供应商与密钥
 * page is the single editor for them, and a second one would be a second write
 * path for the same row. This panel links there instead.
 */
const ModalityModelsPanel: React.FC<ModalityModelsPanelProps> = ({
  modality,
  icon,
  titleKey,
  subtitleKey,
  showDefaultModel = false,
  showUntagged = false,
}) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [message, messageContext] = useArcoMessage({ maxCount: 2 });
  // The ordered, enabled-only provider list — the ONE selector ordering
  // authority, which ranks the managed free platform last. The raw provider
  // query would hand back the backend order, and that order LEADS with the free
  // provider (it is auto-created before the user configures anything).
  const { providers } = useModelProviderList();
  const providerLabel = useModelSelectorProviderLabel();
  const { data: rows, mutate } = useSWR(CATALOG_ROWS_SWR_KEY, () =>
    ipcBridge.providerModel.list.invoke({})
  );
  const [defaultModel, setDefaultModel] = useState(
    () => configService.get('nomi.defaultModel') ?? null
  );
  const [draftDescription, setDraftDescription] = useState('');

  const groups = useMemo(
    () => buildModalityGroups(rows ?? [], providers, MODALITY_SPECS[modality], providerLabel),
    [rows, providers, modality, providerLabel]
  );

  const untagged = useMemo(
    () => (showUntagged ? buildUntaggedGroups(rows ?? [], providers, providerLabel) : []),
    [showUntagged, rows, providers, providerLabel]
  );

  const toggleRow = useCallback(
    async (row: ModalityModelRow, enabled: boolean) => {
      try {
        await ipcBridge.providerModel.update.invoke({
          provider_id: row.providerId,
          model: row.model,
          enabled,
        });
        await mutate();
      } catch (error) {
        console.error('[ModalityModels] Failed to toggle a catalog row:', error);
        message.error(t('settings.modelHub.modality.toggleFailed'));
      }
    },
    [message, mutate, t]
  );

  const saveDescription = useCallback(
    async (row: ModalityModelRow, description: string) => {
      try {
        await ipcBridge.providerModel.update.invoke({
          provider_id: row.providerId,
          model: row.model,
          description: description.trim() || null,
        });
        await mutate();
      } catch (error) {
        console.error('[ModalityModels] Failed to save a model description:', error);
        message.error(t('settings.modelHub.modality.descriptionFailed'));
      }
    },
    [message, mutate, t]
  );

  const persistDefault = useCallback((provider_id: ProviderId, model: string) => {
    const next = { provider_id, model };
    setDefaultModel(next);
    void configService.set('nomi.defaultModel', next).catch(async (error: unknown) => {
      await configService.reload();
      setDefaultModel(configService.get('nomi.defaultModel') ?? null);
      console.error('[ModalityModels] Failed to save the default chat model:', error);
    });
  }, []);

  const renderGroup = (group: ModalityProviderGroup) => (
    <div key={group.providerId} className='flex flex-col gap-6px'>
      <div className='flex min-w-0 items-center gap-8px flex-wrap'>
        <span className='text-14px font-600 text-t-primary truncate'>{group.providerName}</span>
        <span className='text-11px text-t-tertiary shrink-0'>{group.platform}</span>
        <span className='text-11px text-t-tertiary shrink-0'>
          · {t('settings.modelHub.modality.modelCount', { count: group.models.length })}
        </span>
      </div>
      <NomiSettingList>
        {group.models.map((row) => (
          <NomiSettingRow
            key={row.model}
            title={
              <div className='flex min-w-0 items-center gap-6px flex-wrap'>
                <span className='truncate'>{row.model}</span>
                {row.tasks
                  .filter((task) => TASK_LABEL_KEY[task])
                  .map((task) => (
                    <Tag key={task} size='small' color='arcoblue'>
                      {t(TASK_LABEL_KEY[task])}
                    </Tag>
                  ))}
                {row.traits.includes('vision_input') && (
                  <Tag size='small' color='purple'>
                    {t('settings.modelHub.modality.traitVision')}
                  </Tag>
                )}
              </div>
            }
            description={row.description ?? undefined}
            controls={
              <>
                <Switch
                  size='small'
                  className='compact-dark-switch shrink-0'
                  checked={row.enabled}
                  onChange={(enabled: boolean) => void toggleRow(row, enabled)}
                />
                <Popover
                  trigger='click'
                  onVisibleChange={(visible) => {
                    if (visible) setDraftDescription(row.description ?? '');
                  }}
                  content={
                    <div className='flex w-260px flex-col gap-8px'>
                      <Input.TextArea
                        autoSize={{ minRows: 2, maxRows: 5 }}
                        value={draftDescription}
                        placeholder={t('settings.modelHub.modality.descriptionPlaceholder')}
                        onChange={setDraftDescription}
                      />
                      <Button
                        size='mini'
                        type='primary'
                        onClick={() => void saveDescription(row, draftDescription)}
                      >
                        {t('settings.modelHub.modality.descriptionSave')}
                      </Button>
                    </div>
                  }
                >
                  <Button size='mini' icon={<Edit theme='outline' size='12' strokeWidth={3} />} />
                </Popover>
              </>
            }
          />
        ))}
      </NomiSettingList>
    </div>
  );

  return (
    <div className='flex min-h-0 flex-col rd-16px bg-2 px-24px py-16px'>
      {messageContext}
      <header className='flex items-center gap-9px border-b border-b-solid border-[var(--color-border-2)] pb-14px'>
        <span className='size-30px shrink-0 flex items-center justify-center rd-9px bg-primary-1 text-primary-6'>
          {icon}
        </span>
        <div className='min-w-0'>
          <h2 className='m-0 text-20px font-650 leading-28px text-t-primary'>{t(titleKey)}</h2>
          <p className='m-0 mt-2px text-12px leading-18px text-t-secondary'>{t(subtitleKey)}</p>
        </div>
      </header>

      <div className='mt-14px'>
        <NomiSettingList>
          <NomiSettingRow
            title={t('settings.modelHub.modality.defaultRow')}
            description={
              showDefaultModel
                ? t('settings.modelHub.modality.chatDefaultHint')
                : t('settings.modelHub.modality.noDefault')
            }
            controls={
              showDefaultModel ? (
                <TaskModelSelect
                  task='chat'
                  size='small'
                  value={defaultModel}
                  onChange={({ provider_id, model }) => persistDefault(provider_id, model)}
                />
              ) : undefined
            }
          />
        </NomiSettingList>
      </div>

      <NomiScrollArea className='mt-14px flex-1 min-h-0' disableOverflow>
        {groups.length === 0 ? (
          <div className='flex flex-col items-center justify-center py-42px text-center'>
            <h3 className='m-0 text-16px font-500 text-t-primary'>
              {t('settings.modelHub.modality.empty')}
            </h3>
            <p className='mt-6px max-w-420px text-13px leading-20px text-t-secondary'>
              {t('settings.modelHub.modality.emptyHint')}
            </p>
          </div>
        ) : (
          <div className='flex flex-col gap-14px'>{groups.map(renderGroup)}</div>
        )}

        {untagged.length > 0 && (
          <div className='mt-18px flex flex-col gap-8px'>
            <div className='text-14px font-600 text-t-primary'>
              {t('settings.modelHub.modality.untaggedTitle')}
            </div>
            <div className='text-12px leading-18px text-t-secondary'>
              {t('settings.modelHub.modality.untaggedHint')}
            </div>
            {untagged.map(renderGroup)}
          </div>
        )}
      </NomiScrollArea>

      <div className='mt-12px flex items-center gap-8px flex-wrap'>
        <Button
          type='text'
          size='small'
          icon={<LinkCloud theme='outline' size='14' />}
          onClick={() => navigate('/models?section=models')}
        >
          {t('settings.modelHub.modality.manageModels')}
        </Button>
      </div>
    </div>
  );
};

export default ModalityModelsPanel;
