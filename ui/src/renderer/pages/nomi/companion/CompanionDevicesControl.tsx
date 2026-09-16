/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Button, Message, Popover, Radio, Tag } from '@arco-design/web-react';
import { useState } from 'react';
import { Robot } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import useSWR from 'swr';
import { ipcBridge } from '@/common';
import type { ConversationId } from '@/common/types/ids';
import type { useCompanion } from '../useNomi';
import { useRobotStatuses } from '../workspace/tabs/RemoteTab/useRobotStatuses';

/** Physical endpoints of this Companion; never additional conversation links. */
export default function CompanionDevicesControl({ companion, conversationId }: {
  companion: ReturnType<typeof useCompanion>; conversationId: ConversationId;
}) {
  const companionId = companion.profile?.companion_id;
  const [speaking, setSpeaking] = useState(false);
  const { t } = useTranslation();
  const navigate = useNavigate();
  const statuses = useRobotStatuses();
  const { data: robots } = useSWR('robots.list', () => ipcBridge.robot.list.invoke(), { refreshInterval: 5000 });
  const devices = (robots ?? []).filter((robot) => robot.companion_id === companionId);
  if (devices.length === 0) return null;
  const online = devices.filter((robot) => statuses[robot.robot_id]?.phase && statuses[robot.robot_id].phase !== 'offline').length;
  const selectedId = companion.profile?.control_robot_id ?? (devices.length === 1 ? devices[0].robot_id : '');
  const selected = devices.find((device) => device.robot_id === selectedId);
  return <Popover trigger='click' position='br' content={
    <div className='flex flex-col gap-10px min-w-220px max-w-360px'>
      <span className='text-12px text-t-secondary'>{t('nomi.robot.controlTarget')}</span>
      <Radio.Group value={selectedId} direction='vertical' onChange={(robotId: string) => {
        void companion.patchCompanion({ control_robot_id: robotId }).catch(() => Message.error(t('nomi.robot.permissionSaveFailed')));
      }}>
      {devices.map((device) => {
        const phase = statuses[device.robot_id]?.phase ?? 'offline';
        return <div key={device.robot_id} className='flex items-center justify-between gap-12px'>
          <Radio value={device.robot_id}><span className='truncate text-13px'>{device.name}</span></Radio>
          <Tag size='small' color={phase === 'offline' ? 'gray' : 'green'}>{t(`nomi.robot.status.${phase}`)}</Tag>
        </div>;
      })}
      </Radio.Group>
      <Button size='small' loading={speaking}
        disabled={!selected || !selected.permissions.proactive_speech || !statuses[selected.robot_id] || statuses[selected.robot_id].phase !== 'idle'}
        onClick={async () => {
          if (!selected) return;
          setSpeaking(true);
          try { await ipcBridge.robot.speak.invoke({ robot_id: selected.robot_id, conversation_id: conversationId }); }
          catch (error) { Message.error(error instanceof Error ? error.message : t('nomi.robot.playbackFailed')); }
          finally { setSpeaking(false); }
        }}>
        {t('nomi.robot.playLastReply')}
      </Button>
      {selected && !selected.permissions.proactive_speech && <span className='text-12px text-t-secondary'>{t('nomi.robot.playbackPermissionHint')}</span>}
      <Button size='small' onClick={() => void navigate(`/nomi?companion=${companionId}&tab=remote`)}>
        {t('nomi.robot.manageDevices')}
      </Button>
    </div>
  }>
    <Button size='small' aria-label={t('nomi.robot.devices')} icon={<Robot theme='outline' size={14} />}>
      <span className='inline-flex max-w-200px items-center gap-6px'>
        <span className='min-w-0 truncate'>{selected?.name ?? t('nomi.robot.chooseDevice')}</span>
        <span className='shrink-0'>{online}/{devices.length}</span>
      </span>
    </Button>
  </Popover>;
}
