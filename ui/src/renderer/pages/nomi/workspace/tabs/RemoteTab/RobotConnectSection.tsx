/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import dayjs from 'dayjs';
import { Button, Input, Message, Modal, Tag } from '@arco-design/web-react';
import { Robot } from '@icon-park/react';
import { ipcBridge } from '@/common';
import type { IApiRobot, IApiRobotPhase } from '@/common/adapter/ipcBridge';
import type { CompanionId } from '@/common/types/ids';
import {
  NomiSettingList,
  NomiSettingRow,
  NomiSettingSection,
} from '@/renderer/components/base/NomiSettingLayout';
import { ROBOT_STATUS_COLOR } from '@/renderer/components/capability/capabilityStatusColors';
import type { I18nKey } from '@/renderer/services/i18n/i18n-keys';
import AddRobotModal from './AddRobotModal';
import { useRobotStatuses } from './useRobotStatuses';

interface RobotConnectSectionProps {
  companionId: CompanionId;
  companionName: string;
  onAttentionChange?: (hasAttention: boolean) => void;
}

/** One key per phase the backend can publish; a new phase must fail to compile. */
const PHASE_LABEL_KEY: Record<IApiRobotPhase, I18nKey> = {
  offline: 'nomi.robot.status.offline',
  idle: 'nomi.robot.status.idle',
  listening: 'nomi.robot.status.listening',
  speaking: 'nomi.robot.status.speaking',
};

/**
 * 「机器人连接」节：绑到这只伙伴的实体机器人。
 *
 * 与同一 Tab 上方的「远程连接」节严格区分——那里的「机器人」指 IM bot（渠道插件），
 * 这里指真实硬件设备。Attention 只在**可行动**时点亮：本伙伴已绑机器人、但局域网访问
 * 关着，于是设备无论如何都连不上电脑。设备单纯离线（拔电、带走了）不是待办。
 */
const RobotConnectSection: React.FC<RobotConnectSectionProps> = ({
  companionId,
  companionName,
  onAttentionChange,
}) => {
  const { t } = useTranslation();
  const statuses = useRobotStatuses();
  const [robots, setRobots] = useState<IApiRobot[]>([]);
  const [loadFailed, setLoadFailed] = useState(false);
  const [lanEnabled, setLanEnabled] = useState<boolean | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const [busyRobotId, setBusyRobotId] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [rows, endpoints] = await Promise.all([
        ipcBridge.robot.list.invoke(),
        ipcBridge.robot.endpoints.invoke(),
      ]);
      setRobots(rows);
      setLanEnabled(endpoints.lan_enabled);
      setLoadFailed(false);
    } catch (error) {
      console.error('[RobotConnect] Failed to load robots:', error);
      setLoadFailed(true);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const mine = useMemo(
    () => robots.filter((row) => row.companion_id === companionId),
    [robots, companionId]
  );

  const attention = mine.length > 0 && lanEnabled === false;
  useEffect(() => {
    onAttentionChange?.(attention);
  }, [attention, onAttentionChange]);

  const rename = useCallback(
    (row: IApiRobot) => {
      let draft = row.name;
      Modal.confirm({
        title: t('nomi.robot.renameTitle'),
        content: (
          <Input
            defaultValue={row.name}
            placeholder={t('nomi.robot.renamePlaceholder')}
            onChange={(next: string) => {
              draft = next;
            }}
          />
        ),
        onOk: async () => {
          setBusyRobotId(row.robot_id);
          try {
            await ipcBridge.robot.update.invoke({
              robot_id: row.robot_id,
              updates: { name: draft.trim() || row.name },
            });
            await refresh();
          } catch (error) {
            console.error('[RobotConnect] Failed to rename a robot:', error);
            Message.error(t('nomi.robot.renameFailed'));
          } finally {
            setBusyRobotId(null);
          }
        },
      });
    },
    [refresh, t]
  );

  const unbind = useCallback(
    (row: IApiRobot) => {
      Modal.confirm({
        title: t('nomi.robot.unbind'),
        content: t('nomi.robot.unbindConfirm'),
        onOk: async () => {
          setBusyRobotId(row.robot_id);
          try {
            await ipcBridge.robot.update.invoke({
              robot_id: row.robot_id,
              updates: { companion_id: null },
            });
            await refresh();
          } catch (error) {
            console.error('[RobotConnect] Failed to unbind a robot:', error);
            Message.error(t('nomi.robot.renameFailed'));
          } finally {
            setBusyRobotId(null);
          }
        },
      });
    },
    [refresh, t]
  );

  const remove = useCallback(
    (row: IApiRobot) => {
      Modal.confirm({
        title: t('nomi.robot.remove'),
        content: t('nomi.robot.removeConfirm'),
        okButtonProps: { status: 'danger' },
        onOk: async () => {
          setBusyRobotId(row.robot_id);
          try {
            await ipcBridge.robot.remove.invoke({ robot_id: row.robot_id });
            await refresh();
          } catch (error) {
            console.error('[RobotConnect] Failed to delete a robot:', error);
            Message.error(t('nomi.robot.removeFailed'));
          } finally {
            setBusyRobotId(null);
          }
        },
      });
    },
    [refresh, t]
  );

  const phaseOf = (row: IApiRobot): IApiRobotPhase => statuses[row.robot_id]?.phase ?? 'offline';

  return (
    <>
      <NomiSettingSection
        title={t('nomi.robot.title')}
        description={t('nomi.robot.hint', { companionName })}
        action={
          <Button size='small' type='primary' onClick={() => setAddOpen(true)}>
            {t('nomi.robot.add')}
          </Button>
        }
      >
        <NomiSettingList>
          {loadFailed ? (
            <NomiSettingRow title={t('nomi.robot.loadFailed')} />
          ) : mine.length === 0 ? (
            <NomiSettingRow title={t('nomi.robot.empty')} />
          ) : (
            mine.map((row) => (
              <NomiSettingRow
                key={row.robot_id}
                leading={
                  <Robot
                    theme='outline'
                    size='16'
                    fill='currentColor'
                    strokeWidth={3}
                    className='shrink-0'
                    style={{ color: ROBOT_STATUS_COLOR[phaseOf(row)] }}
                  />
                }
                title={
                  <div className='flex min-w-0 flex-wrap items-center gap-6px'>
                    <span className='truncate'>{row.name}</span>
                    <Tag
                      size='small'
                      bordered={false}
                      style={{ color: ROBOT_STATUS_COLOR[phaseOf(row)] }}
                    >
                      {t(PHASE_LABEL_KEY[phaseOf(row)])}
                    </Tag>
                  </div>
                }
                description={[
                  t('nomi.robot.board', { board: row.board }),
                  t('nomi.robot.firmware', { version: row.firmware_version }),
                  row.last_seen
                    ? t('nomi.robot.lastSeen', {
                        time: dayjs(row.last_seen).format('YYYY-MM-DD HH:mm'),
                      })
                    : t('nomi.robot.lastSeenNever'),
                ].join(' · ')}
                controls={
                  <>
                    <Button
                      size='small'
                      loading={busyRobotId === row.robot_id}
                      onClick={() => rename(row)}
                    >
                      {t('nomi.robot.rename')}
                    </Button>
                    <Button size='small' onClick={() => unbind(row)}>
                      {t('nomi.robot.unbind')}
                    </Button>
                    <Button size='small' status='danger' onClick={() => remove(row)}>
                      {t('nomi.robot.remove')}
                    </Button>
                  </>
                }
              />
            ))
          )}
        </NomiSettingList>
      </NomiSettingSection>

      <AddRobotModal
        visible={addOpen}
        companionId={companionId}
        companionName={companionName}
        onCancel={() => setAddOpen(false)}
        onClaimed={() => {
          setAddOpen(false);
          void refresh();
        }}
      />
    </>
  );
};

export default RobotConnectSection;
