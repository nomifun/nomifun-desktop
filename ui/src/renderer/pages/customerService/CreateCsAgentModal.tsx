/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Form, Input, InputNumber, Message, Modal, Select } from '@arco-design/web-react';
import { useModelProviderList } from '@renderer/hooks/agent/useModelProviderList';
import { useKnowledgeBaseOptions } from './useKnowledgeBaseOptions';
import type { ICsAgent, ICsAgentPatch } from '@/common/adapter/ipcBridge';
import type { KnowledgeBaseId, ProviderId } from '@/common/types/ids';

interface Props {
  visible: boolean;
  onClose: () => void;
  onCreated: (agent: ICsAgent) => void;
  create: (input: { name: string } & ICsAgentPatch) => Promise<ICsAgent>;
}

/**
 * 创建客服员工：名称 / 对话模型（复用模型目录）/ 知识库多选（复用知识库目录）/
 * 问候语 / 人设 / 服务策略 / 并发上限。
 */
const CreateCsAgentModal: React.FC<Props> = ({ visible, onClose, onCreated, create }) => {
  const { t } = useTranslation();
  const [form] = Form.useForm();
  const [submitting, setSubmitting] = useState(false);
  const { providers, getAvailableModels } = useModelProviderList();
  const { options: kbOptions } = useKnowledgeBaseOptions();
  const [providerId, setProviderId] = useState<ProviderId | undefined>(undefined);

  const modelOptions = useMemo(() => {
    const provider = providers.find((p) => p.id === providerId);
    return provider ? getAvailableModels(provider) : [];
  }, [providers, providerId, getAvailableModels]);

  const handleSubmit = async () => {
    const values = await form.validate();
    setSubmitting(true);
    try {
      const created = await create({
        name: (values.name as string).trim(),
        greeting: (values.greeting as string) ?? '',
        persona: (values.persona as string) ?? '',
        service_policy: (values.service_policy as string) ?? '',
        provider_id: (values.provider_id as ProviderId | undefined) ?? null,
        model: (values.model as string | undefined) ?? null,
        knowledge_base_ids: ((values.knowledge_base_ids as KnowledgeBaseId[] | undefined) ?? []),
        max_concurrent: (values.max_concurrent as number | undefined) ?? 8,
      });
      Message.success(t('customerService.create.done', { defaultValue: '客服已创建' }));
      form.resetFields();
      onClose();
      onCreated(created);
    } catch (error) {
      Message.error(error instanceof Error ? error.message : String(error));
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <Modal
      visible={visible}
      title={t('customerService.create.title', { defaultValue: '创建客服' })}
      onCancel={onClose}
      onOk={() => void handleSubmit()}
      confirmLoading={submitting}
      autoFocus={false}
      maskClosable={false}
      style={{ width: 520 }}
    >
      <Form form={form} layout='vertical'>
        <Form.Item
          label={t('customerService.fields.name', { defaultValue: '名称' })}
          field='name'
          rules={[{ required: true, message: t('customerService.fields.nameRequired', { defaultValue: '请输入客服名称' }) }]}
        >
          <Input placeholder={t('customerService.fields.namePlaceholder', { defaultValue: '例如：售后小助' })} />
        </Form.Item>
        <div className='grid grid-cols-2 gap-x-12px'>
          <Form.Item label={t('customerService.fields.provider', { defaultValue: '模型服务商' })} field='provider_id'>
            <Select
              placeholder={t('customerService.fields.providerPlaceholder', { defaultValue: '选择服务商' })}
              allowClear
              onChange={(value) => {
                setProviderId(value as ProviderId | undefined);
                form.setFieldValue('model', undefined);
              }}
            >
              {providers.map((p) => (
                <Select.Option key={p.id} value={p.id}>
                  {p.name}
                </Select.Option>
              ))}
            </Select>
          </Form.Item>
          <Form.Item label={t('customerService.fields.model', { defaultValue: '对话模型' })} field='model'>
            <Select placeholder={t('customerService.fields.modelPlaceholder', { defaultValue: '选择模型' })} allowClear>
              {modelOptions.map((m) => (
                <Select.Option key={m} value={m}>
                  {m}
                </Select.Option>
              ))}
            </Select>
          </Form.Item>
        </div>
        <Form.Item label={t('customerService.fields.knowledgeBases', { defaultValue: '知识库' })} field='knowledge_base_ids'>
          <Select
            mode='multiple'
            placeholder={t('customerService.fields.knowledgeBasesPlaceholder', { defaultValue: '选择可检索的知识库' })}
            allowClear
          >
            {kbOptions.map((kb) => (
              <Select.Option key={kb.value} value={kb.value}>
                {kb.label}
              </Select.Option>
            ))}
          </Select>
        </Form.Item>
        <Form.Item label={t('customerService.fields.greeting', { defaultValue: '问候语' })} field='greeting'>
          <Input.TextArea rows={2} placeholder={t('customerService.fields.greetingPlaceholder', { defaultValue: '访客打招呼时的开场白' })} />
        </Form.Item>
        <Form.Item label={t('customerService.fields.persona', { defaultValue: '人设话术' })} field='persona'>
          <Input.TextArea rows={2} placeholder={t('customerService.fields.personaPlaceholder', { defaultValue: '语气与人设，例如：耐心、简洁、以事实为准' })} />
        </Form.Item>
        <Form.Item label={t('customerService.fields.servicePolicy', { defaultValue: '服务策略' })} field='service_policy'>
          <Input.TextArea rows={2} placeholder={t('customerService.fields.servicePolicyPlaceholder', { defaultValue: '业务范围 / 禁答话题 / 合规话术' })} />
        </Form.Item>
        <Form.Item
          label={t('customerService.fields.maxConcurrent', { defaultValue: '并发上限' })}
          field='max_concurrent'
          initialValue={8}
        >
          <InputNumber min={1} max={64} />
        </Form.Item>
      </Form>
    </Modal>
  );
};

export default CreateCsAgentModal;
