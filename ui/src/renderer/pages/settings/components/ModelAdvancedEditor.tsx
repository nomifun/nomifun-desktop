/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, Input, Popover, Select, Tooltip } from '@arco-design/web-react';
import { SettingTwo } from '@icon-park/react';
import type { ProviderId } from '@/common/types/ids';
import {
  MODEL_PROTOCOL_OPTIONS,
  REQUEST_SHAPE_OPTIONS,
  mergeModelParams,
  splitModelParams,
} from './providerModelAdvanced';
import { useProviderConnections } from './useProviderConnections';

export interface ModelAdvancedPatch {
  protocol: string | null;
  connection_role: string | null;
  params: Record<string, unknown>;
}

/**
 * Per-model "高级" popover: invoke `protocol` override, `connection_role`
 * binding and `params` (quick fields endpoint/request_shape + raw JSON for
 * the rest, merged on save). Saves through `providerModel.update` via the
 * parent's onSave.
 */
const ModelAdvancedEditor: React.FC<{
  providerId: ProviderId;
  protocol?: string;
  connectionRole?: string;
  params: unknown;
  onSave: (patch: ModelAdvancedPatch) => Promise<void>;
}> = ({ providerId, protocol, connectionRole, params, onSave }) => {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [saving, setSaving] = useState(false);
  const [draftProtocol, setDraftProtocol] = useState('');
  const [draftRole, setDraftRole] = useState('');
  const [draftEndpoint, setDraftEndpoint] = useState('');
  const [draftShape, setDraftShape] = useState('');
  const [draftRestJson, setDraftRestJson] = useState('');
  const [jsonError, setJsonError] = useState(false);

  // Roles come from the provider's connection profiles; fetched only once the
  // popover opens (SWR dedupes with the connections section).
  const { connections } = useProviderConnections(providerId, open);

  const split = splitModelParams(params);
  const hasOverrides =
    Boolean(protocol) || Boolean(connectionRole) || Boolean(split.endpoint || split.requestShape || split.restJson);

  const handleVisibleChange = (visible: boolean) => {
    if (visible) {
      const next = splitModelParams(params);
      setDraftProtocol(protocol ?? '');
      setDraftRole(connectionRole ?? '');
      setDraftEndpoint(next.endpoint);
      setDraftShape(next.requestShape);
      setDraftRestJson(next.restJson);
      setJsonError(false);
    }
    setOpen(visible);
  };

  const protocolOptions = [
    { label: t('settings.modelAdvanced.protocolAuto', { defaultValue: '自动（按任务路由）' }), value: '' },
    ...MODEL_PROTOCOL_OPTIONS.map((value) => ({ label: value, value })),
  ];

  const roleOptions = [
    { label: t('settings.modelAdvanced.connectionDefault', { defaultValue: '默认连接' }), value: '' },
    ...connections.map((c) => ({ label: c.label ? `${c.role} · ${c.label}` : c.role, value: c.role })),
    // Keep a stored role visible even if its connection profile is gone.
    ...(connectionRole && !connections.some((c) => c.role === connectionRole)
      ? [{ label: connectionRole, value: connectionRole }]
      : []),
  ];

  const shapeOptions = [
    { label: t('settings.modelAdvanced.requestShapeAuto', { defaultValue: '自动' }), value: '' },
    ...REQUEST_SHAPE_OPTIONS.map((value) => ({ label: value, value })),
  ];

  const handleSave = async () => {
    const merged = mergeModelParams(draftRestJson, draftEndpoint, draftShape);
    if (!merged.ok) {
      setJsonError(true);
      return;
    }
    setJsonError(false);
    setSaving(true);
    try {
      await onSave({
        protocol: draftProtocol.trim() || null,
        connection_role: draftRole.trim() || null,
        params: merged.params,
      });
      setOpen(false);
    } catch {
      // Parent owns the toast; keep the editor open for retry.
    } finally {
      setSaving(false);
    }
  };

  return (
    <Popover
      trigger='click'
      position='bl'
      popupVisible={open}
      onVisibleChange={handleVisibleChange}
      content={
        <div className='flex flex-col gap-8px w-320px' onClick={(e) => e.stopPropagation()}>
          <div className='text-12px text-t-secondary'>
            {t('settings.modelAdvanced.title', { defaultValue: '高级设置' })}
          </div>

          <div className='text-11px text-t-tertiary'>
            {t('settings.modelAdvanced.protocol', { defaultValue: '调用协议（protocol）' })}
          </div>
          <Select
            value={draftProtocol}
            onChange={(v: string) => setDraftProtocol(v ?? '')}
            options={protocolOptions}
            allowCreate
            showSearch
            triggerProps={{ getPopupContainer: () => document.body }}
          />

          <div className='text-11px text-t-tertiary'>
            {t('settings.modelAdvanced.connectionRole', { defaultValue: '连接档案（connection_role）' })}
          </div>
          <Select
            value={draftRole}
            onChange={(v: string) => setDraftRole(v ?? '')}
            options={roleOptions}
            triggerProps={{ getPopupContainer: () => document.body }}
          />

          <div className='text-11px text-t-tertiary'>
            {t('settings.modelAdvanced.endpoint', { defaultValue: '端点路径（endpoint）' })}
          </div>
          <Input value={draftEndpoint} onChange={setDraftEndpoint} placeholder='/v1/images/generations' />

          <div className='text-11px text-t-tertiary'>
            {t('settings.modelAdvanced.requestShape', { defaultValue: '请求格式（request_shape）' })}
          </div>
          <Select
            value={draftShape}
            onChange={(v: string) => setDraftShape(v ?? '')}
            options={shapeOptions}
            triggerProps={{ getPopupContainer: () => document.body }}
          />

          <div className='text-11px text-t-tertiary'>
            {t('settings.modelAdvanced.paramsJson', { defaultValue: '其余参数（JSON）' })}
          </div>
          <Input.TextArea
            value={draftRestJson}
            onChange={setDraftRestJson}
            placeholder={t('settings.modelAdvanced.paramsJsonPlaceholder', {
              defaultValue: '{ "size": "1024x1024" }',
            })}
            autoSize={{ minRows: 2, maxRows: 8 }}
          />
          {jsonError && (
            <div className='text-11px leading-4 text-[rgb(var(--danger-6))]'>
              {t('settings.modelAdvanced.invalidParamsJson', {
                defaultValue: '参数 JSON 无效，需为 JSON 对象',
              })}
            </div>
          )}

          <div className='flex items-center justify-end gap-8px'>
            <Button size='mini' onClick={() => setOpen(false)} disabled={saving}>
              {t('common.cancel', { defaultValue: '取消' })}
            </Button>
            <Button size='mini' type='primary' loading={saving} onClick={handleSave}>
              {t('common.save', { defaultValue: '保存' })}
            </Button>
          </div>
        </div>
      }
    >
      <Tooltip content={t('settings.modelAdvanced.trigger', { defaultValue: '高级' })}>
        <Button
          size='mini'
          className={`model-provider-action-btn !w-24px !h-24px !min-w-24px shrink-0 ${hasOverrides ? 'text-[rgb(var(--primary-6))] hover:text-[rgb(var(--primary-5))]' : 'text-t-secondary hover:text-t-primary'}`}
          icon={<SettingTwo theme='outline' size='14' />}
          onClick={(e) => e.stopPropagation()}
        />
      </Tooltip>
    </Popover>
  );
};

export default ModelAdvancedEditor;
