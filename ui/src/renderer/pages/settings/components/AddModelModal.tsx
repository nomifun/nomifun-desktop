import { ipcBridge } from '@/common';
import type { IProvider, ModelTask, ModelTrait } from '@/common/config/storage';
import ModalHOC from '@/renderer/utils/ui/ModalHOC';
import NomiModal from '@/renderer/components/base/NomiModal';
import { useArcoMessage } from '@/renderer/utils/ui/useArcoMessage';
import { Checkbox, Select } from '@arco-design/web-react';
import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { mutate as mutateSWR } from 'swr';
import useModeModeList from '@renderer/hooks/agent/useModeModeList';
import { MODEL_PROFILES_SWR_KEY } from '@renderer/hooks/agent/useModelProfiles';
import { buildModelProfileUpsertRequest, MODEL_TASK_ORDER } from '@renderer/hooks/agent/modelProfileEditing';
import {
  isNewApiPlatform,
  NEW_API_PROTOCOL_OPTIONS,
  detectNewApiProtocol,
  getSupportedTasksForPlatform,
} from '@/renderer/utils/model/modelPlatforms';
import { ContextLimitSelect } from './ContextLimitSelect';

const AddModelModal = ModalHOC<{ data?: IProvider; onSubmit: (model: IProvider) => void }>(
  ({ modalProps, data, onSubmit, modalCtrl }) => {
    const { t } = useTranslation();
    const [message, messageHolder] = useArcoMessage();
    const [model, setModel] = useState('');
    const [modelProtocol, setModelProtocol] = useState<string>('openai');
    const [contextLimit, setContextLimit] = useState<number | undefined>();
    const [selectedTask, setSelectedTask] = useState<ModelTask | undefined>();
    const [discoveredTasks, setDiscoveredTasks] = useState<ModelTask[]>([]);
    const [discoveredTraits, setDiscoveredTraits] = useState<ModelTrait[]>([]);
    const [visionInput, setVisionInput] = useState(false);
    const isNewApi = isNewApiPlatform(data?.platform ?? '');
    const supportedTasks = useMemo(() => getSupportedTasksForPlatform(data ?? { platform: '' }), [data]);
    const taskOptions = useMemo(
      () =>
        MODEL_TASK_ORDER.filter((v) => supportedTasks.includes(v)).map((v) => ({
          label: t(`settings.modelTask.${v}`),
          value: v,
        })),
      [supportedTasks, t]
    );
    const { data: modelList, isLoading } = useModeModeList(data?.platform ?? '', data?.base_url, data?.api_key);
    const existingModels = data?.models || [];
    const optionsList = useMemo(() => {
      // 处理新的数据格式，可能包含 fix_base_url
      const models = Array.isArray(modelList) ? modelList : modelList?.models || [];
      if (!selectedTask) return [];
      return models
        .filter(
          (item) =>
            item.tasks.includes(selectedTask) ||
            (item.tasks.length === 0 && (selectedTask === 'chat' || isNewApi || data?.platform === 'custom'))
        )
        .map((item) => ({ ...item, disabled: data?.models?.includes(item.value) ?? false }));
    }, [modelList, data?.models, data?.platform, isNewApi, selectedTask]);

    useEffect(() => {
      if (modalProps.visible) {
        setModel('');
        setModelProtocol('openai');
        setContextLimit(undefined);
        setSelectedTask(undefined);
        setDiscoveredTasks([]);
        setDiscoveredTraits([]);
        setVisionInput(false);
      }
    }, [modalProps.visible]);

    const handleConfirm = useCallback(async () => {
      if (!model || !selectedTask || !data) return;
      const nextContextLimits = { ...data.model_context_limits };
      if (contextLimit && contextLimit > 0) {
        nextContextLimits[model] = contextLimit;
      } else {
        delete nextContextLimits[model];
      }

      const updatedData: IProvider = {
        ...data,
        models: [...existingModels, model],
        model_context_limits: Object.keys(nextContextLimits).length > 0 ? nextContextLimits : undefined,
      };

      // new-api 平台：添加模型协议配置 / new-api platform: add model protocol config
      if (isNewApi) {
        updatedData.model_protocols = { ...data?.model_protocols, [model]: modelProtocol };
      }

      onSubmit(updatedData);

      // Persist the authoritative capability profile for the new model so probing
      // and dispatch pick the correct endpoint (source=user = authoritative).
      try {
        const selectedTraits = new Set<ModelTrait>(discoveredTraits);
        if (selectedTask === 'chat' && visionInput) {
          selectedTraits.add('vision_input');
        } else {
          selectedTraits.delete('vision_input');
        }
        const verifiedTasks = discoveredTasks.filter((task) => supportedTasks.includes(task));
        const profileTasks = verifiedTasks.includes(selectedTask) ? verifiedTasks : [selectedTask];
        await ipcBridge.modelProfile.upsert.invoke({
          ...buildModelProfileUpsertRequest(data.id, model, profileTasks, [...selectedTraits]),
        });
        void mutateSWR(MODEL_PROFILES_SWR_KEY);
      } catch (e) {
        console.error('model profile upsert failed', e);
        message.warning(t('settings.saveModelConfigFailed', { defaultValue: '模型能力保存失败' }));
      }
      modalCtrl.close();
    }, [
      contextLimit,
      data,
      existingModels,
      model,
      selectedTask,
      discoveredTasks,
      discoveredTraits,
      modelProtocol,
      isNewApi,
      visionInput,
      onSubmit,
      modalCtrl,
      message,
      t,
      supportedTasks,
    ]);

    return (
      <>
        {messageHolder}
        <NomiModal
          visible={modalProps.visible}
          onCancel={modalCtrl.close}
          header={{ title: t('settings.addModel'), showClose: true }}
          style={{ maxHeight: '90vh' }}
          contentStyle={{
            background: 'var(--dialog-fill-0)',
            borderRadius: 16,
            padding: '20px 24px',
            overflow: 'auto',
          }}
          onOk={handleConfirm}
          okText={t('common.confirm')}
          cancelText={t('common.cancel')}
          okButtonProps={{ disabled: !selectedTask || !model }}
        >
        <div className='flex flex-col gap-16px pt-20px'>
          <div className='space-y-8px'>
            <div className='text-13px font-500 text-t-secondary'>{t('settings.modelModality')}</div>
            <Select
              value={selectedTask}
              onChange={(value: ModelTask) => {
                setSelectedTask(value);
                setModel('');
                setDiscoveredTasks([]);
                setDiscoveredTraits([]);
                setVisionInput(false);
              }}
              options={taskOptions}
              placeholder={t('settings.modelModality')}
              triggerProps={{ getPopupContainer: (node) => node.parentElement || document.body }}
            />
            <div className='text-11px text-t-secondary leading-4'>{t('settings.modelModalityTip')}</div>
          </div>

          <div className='space-y-8px'>
            <div className='text-13px font-500 text-t-secondary'>{t('settings.addModelPlaceholder')}</div>
            <Select
              disabled={!selectedTask}
              showSearch
              options={optionsList}
              loading={isLoading}
              onChange={(value: string) => {
                setModel(value);
                const profile = optionsList.find((item) => item.value === value);
                setDiscoveredTasks(profile?.tasks ?? []);
                const traits = profile?.traits ?? [];
                setDiscoveredTraits(traits);
                setVisionInput(traits.includes('vision_input'));
                if (isNewApi) setModelProtocol(detectNewApiProtocol(value));
              }}
              value={model}
              allowCreate
              placeholder={t('settings.addModelPlaceholder')}
            ></Select>
          </div>

          <div className='space-y-8px'>
            <div className='text-13px font-500 text-t-secondary'>
              {t('settings.contextLimit', { defaultValue: '上下文窗口 (tokens)' })}
            </div>
            <ContextLimitSelect value={contextLimit} onChange={setContextLimit} />
          </div>

          {/* Chat input traits are initialized from model discovery and remain user-adjustable. */}
          {selectedTask === 'chat' && (
            <Checkbox checked={visionInput} onChange={setVisionInput} className='!pl-0'>
              <span className='text-12px text-t-secondary'>{t('settings.modelVisionInput')}</span>
            </Checkbox>
          )}

          {/* New API 协议选择 / New API Protocol Selection */}
          {isNewApi && (
            <div className='space-y-8px'>
              <div className='text-13px font-500 text-t-secondary'>{t('settings.modelProtocol')}</div>
              <Select
                value={modelProtocol}
                onChange={setModelProtocol}
                options={NEW_API_PROTOCOL_OPTIONS}
                triggerProps={{ getPopupContainer: (node) => node.parentElement || document.body }}
              />
              <div className='text-11px text-t-secondary leading-4'>{t('settings.modelProtocolTip')}</div>
            </div>
          )}

          <div className='space-y-8px'>
            {/* <div className='text-13px font-500 text-t-secondary'>{t('settings.current_modelsLabel')}</div>
          {existingModels.length === 0 ? (
            <div className='text-13px text-t-secondary bg-fill-1 rd-8px px-12px py-14px border border-dashed border-arco-2'>{t('settings.addModelNoExisting')}</div>
          ) : (
            <div className='flex flex-wrap gap-8px bg-1 rd-8px px-12px py-10px border border-solid border-arco-2'>
              {previewModels.map((item) => (
                <Tag key={item} bordered={false} className='text-12px !bg-primary-1 !text-primary-6'>
                  {item}
                </Tag>
              ))}
              {remainingCount > 0 && <Tag bordered>{t('settings.addModelMoreCount', { count: remainingCount })}</Tag>}
            </div>
          )} */}
          </div>

          {/* <div className='text-12px tet-t-tertiary leading-5 bg-fill-1 rd-8px px-12px py-10px border border-dashed border-arco-2'>{t('settings.addModelTips')}</div> */}
        </div>
        {/* <div className='text-12px text-t-secondary leading-5 my-4'>{model ? t('settings.addModelSelectedHint', { model }) : t('settings.addModelHint')}</div> */}
        </NomiModal>
      </>
    );
  }
);

export default AddModelModal;
