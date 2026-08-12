/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import { Button, Input, Popover, Switch, Tag } from '@arco-design/web-react';
import { Edit, LinkCloud } from '@icon-park/react';
import { ipcBridge } from '@/common';
import { configService } from '@/common/config/configService';
import type { ConfigKeyMap } from '@/common/config/configKeys';
import type { ProviderId } from '@/common/types/ids';
import { toProviderModelInput } from '@/common/utils/providerModels';
import NomiScrollArea from '@/renderer/components/base/NomiScrollArea';
import {
  NomiSettingList,
  NomiSettingRow,
} from '@/renderer/components/base/NomiSettingLayout';
import TaskModelSelect from '@/renderer/components/model/TaskModelSelect';
import { orderModelSelectorProviders } from '@/renderer/hooks/agent/modelSelectorProviderOrdering';
import { useProvidersQuery } from '@/renderer/hooks/agent/useModelProviderList';
import { useModelSelectorProviderLabel } from '@/renderer/hooks/agent/useModelSelectorProviderLabel';
import { useModelsForTask } from '@/renderer/hooks/agent/useModelsForTask';
import type { I18nKey } from '@/renderer/services/i18n/i18n-keys';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import {
  buildModalityGroups,
  MODALITY_SPECS,
  type ModalityKey,
  type ModalityModelRow,
  type ModalityProviderGroup,
} from './modalityModels';
import { SerializedLatestWriteQueue } from './serializedLatestWriteQueue';

type ImageGenerationDefaultModel = NonNullable<
  ConfigKeyMap['models.default.imageGeneration']
>;

interface ImageGenerationDefaultControlProps {
  preferenceKey: 'models.default.imageGeneration';
}

const ImageGenerationDefaultControl: React.FC<
  ImageGenerationDefaultControlProps
> = ({ preferenceKey }) => {
  const { t } = useTranslation();
  const [message, messageContext] = useArcoMessage({ maxCount: 1 });
  const { groups, isLoading } = useModelsForTask('image_generation');
  const [defaultModel, setDefaultModel] =
    useState<ImageGenerationDefaultModel | null>(
      () => configService.get(preferenceKey) ?? null,
    );
  const [isSavingDefault, setIsSavingDefault] = useState(false);
  const writeQueueRef = useRef(new SerializedLatestWriteQueue());

  useEffect(() => {
    let active = true;
    const sync = () => {
      if (active && !writeQueueRef.current.hasPending) {
        setDefaultModel(configService.get(preferenceKey) ?? null);
      }
    };
    const unsubscribe = configService.subscribe(preferenceKey, (value) => {
      if (active && !writeQueueRef.current.hasPending) {
        setDefaultModel(
          (value as ImageGenerationDefaultModel | undefined) ?? null,
        );
      }
    });
    void configService.whenReady().then(sync);
    return () => {
      active = false;
      unsubscribe();
    };
  }, [preferenceKey]);

  const persistDefault = useCallback(
    async (next: ImageGenerationDefaultModel | null) => {
      setDefaultModel(next);
      setIsSavingDefault(true);

      const queue = writeQueueRef.current;
      const { done } = queue.enqueue(
        () =>
          next
            ? configService.set(preferenceKey, next)
            : configService.remove(preferenceKey),
        {
          onLatestError: async (error, generation) => {
            await configService.reload();
            if (!queue.isLatest(generation)) return;
            setDefaultModel(configService.get(preferenceKey) ?? null);
            console.error(
              '[ModalityModels] Failed to save the default image model:',
              error,
            );
            message.error(t('settings.modelHub.creation.defaultSaveFailed'));
          },
          onLatestSettled: (generation) => {
            if (queue.isLatest(generation)) setIsSavingDefault(false);
          },
        },
      );
      await done;
    },
    [message, preferenceKey, t],
  );

  const hasCandidates = groups.some((group) => group.models.length > 0);
  const noCandidates = !isLoading && !hasCandidates;
  const description = isLoading
    ? t('settings.modelHub.creation.defaultLoading')
    : noCandidates
      ? t('settings.modelHub.creation.defaultNoModels')
      : defaultModel
        ? t('settings.modelHub.creation.defaultHint')
        : t('settings.modelHub.creation.defaultUnset');

  return (
    <div className='mt-14px'>
      {messageContext}
      <NomiSettingList>
        <NomiSettingRow
          title={t('settings.modelHub.creation.defaultTitle')}
          description={description}
          controls={
            <div className='flex min-w-0 flex-wrap items-center justify-end gap-8px'>
              <TaskModelSelect
                task='image_generation'
                size='small'
                disabled={noCandidates || isSavingDefault}
                value={defaultModel}
                emptyHint={t('settings.modelHub.creation.defaultNoModels')}
                onChange={({ provider_id, model }) =>
                  void persistDefault({ provider_id, model })
                }
              />
              {defaultModel && (
                <Button
                  size='mini'
                  disabled={isSavingDefault}
                  onClick={() => void persistDefault(null)}
                >
                  {t('settings.modelHub.creation.defaultClear')}
                </Button>
              )}
            </div>
          }
        />
      </NomiSettingList>
    </div>
  );
};

export interface ModalityModelsPanelProps {
  modality: ModalityKey;
  icon: React.ReactNode;
  titleKey: I18nKey;
  subtitleKey: I18nKey;
  /** Chat alone owns the install-wide default conversation model. */
  showDefaultModel?: boolean;
  /** Optional install-wide default owned by this model task. */
  defaultModelPreferenceKey?: 'models.default.imageGeneration';
}

/**
 * One management section over the nested provider response. Disabled rows stay
 * visible; every mutation sends one full model definition to the atomic save
 * route, so no capability can be lost by a partial row update.
 */
const ModalityModelsPanel: React.FC<ModalityModelsPanelProps> = ({
  modality,
  icon,
  titleKey,
  subtitleKey,
  showDefaultModel = false,
  defaultModelPreferenceKey,
}) => {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const [message, messageContext] = useArcoMessage({ maxCount: 2 });
  const { data: providerData, mutate } = useProvidersQuery();
  const providerLabel = useModelSelectorProviderLabel();
  const providers = useMemo(
    () => orderModelSelectorProviders(providerData ?? []),
    [providerData],
  );
  const [defaultModel, setDefaultModel] = useState(
    () => configService.get('nomi.defaultModel') ?? null,
  );
  const [draftDescription, setDraftDescription] = useState('');

  const groups = useMemo(
    () =>
      buildModalityGroups(providers, MODALITY_SPECS[modality], providerLabel),
    [providers, modality, providerLabel],
  );

  const saveRow = useCallback(
    async (
      row: ModalityModelRow,
      patch: { enabled?: boolean; description?: string },
    ) => {
      const definition = {
        ...row.definition,
        ...patch,
      };
      await ipcBridge.providerModel.save.invoke({
        provider_id: row.providerId,
        model: toProviderModelInput(definition),
      });
      await mutate();
    },
    [mutate],
  );

  const toggleRow = useCallback(
    async (row: ModalityModelRow, enabled: boolean) => {
      try {
        await saveRow(row, { enabled });
      } catch (error) {
        console.error('[ModalityModels] Failed to toggle a model:', error);
        message.error(t('settings.modelHub.modality.toggleFailed'));
      }
    },
    [message, saveRow, t],
  );

  const saveDescription = useCallback(
    async (row: ModalityModelRow, description: string) => {
      try {
        await saveRow(row, { description: description.trim() || undefined });
      } catch (error) {
        console.error(
          '[ModalityModels] Failed to save a model description:',
          error,
        );
        message.error(t('settings.modelHub.modality.descriptionFailed'));
      }
    },
    [message, saveRow, t],
  );

  const persistDefault = useCallback(
    (provider_id: ProviderId, model: string) => {
      const next = { provider_id, model };
      setDefaultModel(next);
      void configService
        .set('nomi.defaultModel', next)
        .catch(async (error: unknown) => {
          await configService.reload();
          setDefaultModel(configService.get('nomi.defaultModel') ?? null);
          console.error(
            '[ModalityModels] Failed to save the default chat model:',
            error,
          );
        });
    },
    [],
  );

  const renderGroup = (group: ModalityProviderGroup) => (
    <div key={group.providerId} className='flex flex-col gap-6px'>
      <div className='flex min-w-0 items-center gap-8px flex-wrap'>
        <span className='text-14px font-600 text-t-primary truncate'>
          {group.providerName}
        </span>
        <span className='text-11px text-t-tertiary shrink-0'>
          {group.platform}
        </span>
        {!group.enabled && (
          <Tag size='small' color='gray'>
            {t('settings.modelHub.modality.modelDisabled')}
          </Tag>
        )}
        <span className='text-11px text-t-tertiary shrink-0'>
          ·{' '}
          {t('settings.modelHub.modality.modelCount', {
            count: group.models.length,
          })}
        </span>
      </div>
      <NomiSettingList>
        {group.models.map((row) => (
          <NomiSettingRow
            key={row.model}
            title={
              <div className='flex min-w-0 items-center gap-6px flex-wrap'>
                <span className='truncate'>{row.model}</span>
                <Tag size='small' color='arcoblue'>
                  {t(`settings.modelTask.${row.capability.task}` as I18nKey)}
                </Tag>
                {row.traits.includes('vision_input') && (
                  <Tag size='small' color='purple'>
                    {t('settings.modelHub.modality.traitVision')}
                  </Tag>
                )}
                <Tag size='small' color='gray'>
                  {row.protocol}
                </Tag>
                {!row.enabled && (
                  <Tag size='small' color='gray'>
                    {t('settings.modelHub.modality.modelDisabled')}
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
                        placeholder={t(
                          'settings.modelHub.modality.descriptionPlaceholder',
                        )}
                        onChange={setDraftDescription}
                      />
                      <Button
                        size='mini'
                        type='primary'
                        onClick={() =>
                          void saveDescription(row, draftDescription)
                        }
                      >
                        {t('settings.modelHub.modality.descriptionSave')}
                      </Button>
                    </div>
                  }
                >
                  <Button
                    size='mini'
                    icon={<Edit theme='outline' size='12' strokeWidth={3} />}
                  />
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
          <h2 className='m-0 text-20px font-650 leading-28px text-t-primary'>
            {t(titleKey)}
          </h2>
          <p className='m-0 mt-2px text-12px leading-18px text-t-secondary'>
            {t(subtitleKey)}
          </p>
        </div>
      </header>

      {showDefaultModel && (
        <div className='mt-14px'>
          <NomiSettingList>
            <NomiSettingRow
              title={t('settings.modelHub.modality.defaultRow')}
              description={t('settings.modelHub.modality.chatDefaultHint')}
              controls={
                <TaskModelSelect
                  task='chat'
                  size='small'
                  value={defaultModel}
                  onChange={({ provider_id, model }) =>
                    persistDefault(provider_id, model)
                  }
                />
              }
            />
          </NomiSettingList>
        </div>
      )}

      {defaultModelPreferenceKey && (
        <ImageGenerationDefaultControl
          preferenceKey={defaultModelPreferenceKey}
        />
      )}

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
          <div className='flex flex-col gap-14px'>
            {groups.map(renderGroup)}
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
