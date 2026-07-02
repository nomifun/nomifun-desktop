/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Input, Modal } from '@arco-design/web-react';
import { Shield } from '@icon-park/react';
import type { IFigureMeta, ICompanionProfile } from '@/common/adapter/ipcBridge';
import { CUSTOM_CHARACTER_ID, DEFAULT_CHARACTER_ID } from '@renderer/pages/companion/characters';
import CharacterPicker from '@renderer/pages/nomi/CharacterPicker';
import { useArcoMessage } from '@renderer/utils/ui/useArcoMessage';

interface Props {
  visible: boolean;
  onClose: () => void;
  onHired: (profile: ICompanionProfile) => void;
  hireEmployee: (input: { name: string; character: string; figure?: IFigureMeta | null }) => Promise<ICompanionProfile>;
}

/**
 * 招聘外呼员工 —— 新建一个【专属】桌面伙伴并即刻设为「公开服务」。
 * (锁定的产品决策：外呼员工总是新建专属伙伴，绝不把已有私有伙伴转公开。)
 */
const HireEmployeeModal: React.FC<Props> = ({ visible, onClose, onHired, hireEmployee }) => {
  const { t } = useTranslation();
  const [message, holder] = useArcoMessage();
  const [name, setName] = useState('');
  const [character, setCharacter] = useState<string>(DEFAULT_CHARACTER_ID);
  const [figure, setFigure] = useState<IFigureMeta | null>(null);
  const [hiring, setHiring] = useState(false);

  // Reset the form each time the modal opens.
  const handleAfterOpen = () => {
    setName('');
    setCharacter(DEFAULT_CHARACTER_ID);
    setFigure(null);
  };

  const submit = async () => {
    const trimmed = name.trim();
    if (!trimmed || hiring) return;
    setHiring(true);
    try {
      const profile = await hireEmployee({ name: trimmed, character, figure });
      message.success(t('outbound.hire.success', { defaultValue: '已招聘外呼员工 {{name}}', name: profile.name }));
      onHired(profile);
      onClose();
    } catch (e) {
      message.error(
        t('outbound.hire.failed', { defaultValue: '招聘失败：{{err}}', err: e instanceof Error ? e.message : String(e) })
      );
    } finally {
      setHiring(false);
    }
  };

  return (
    <Modal
      title={
        <span className='flex items-center gap-8px'>
          <span
            className='flex items-center justify-center w-24px h-24px rd-7px text-white'
            style={{ background: 'linear-gradient(160deg, rgb(var(--success-5)), rgb(var(--success-6)))' }}
          >
            <Shield theme='filled' size='14' fill='currentColor' className='block' style={{ lineHeight: 0 }} />
          </span>
          {t('outbound.hire.title', { defaultValue: '招聘外呼员工' })}
        </span>
      }
      visible={visible}
      onOk={() => void submit()}
      onCancel={onClose}
      afterOpen={handleAfterOpen}
      okText={t('outbound.hire.confirm', { defaultValue: '招聘' })}
      cancelText={t('common.cancel', { defaultValue: '取消' })}
      okButtonProps={{ loading: hiring, disabled: !name.trim() }}
      style={{ width: 580 }}
    >
      {holder}
      <div className='flex flex-col gap-16px'>
        <div className='rd-10px bg-[rgba(var(--success-6),0.08)] border border-solid border-[rgba(var(--success-6),0.22)] px-12px py-10px text-12px leading-18px text-t-secondary'>
          {t('outbound.hire.hint', {
            defaultValue:
              '将新建一位专属桌面伙伴并设为「公开服务」：只能问答 + 检索知识库，高危能力全部关闭。稍后可在详情中配置公开知识库与社交渠道。',
          })}
        </div>
        <div className='flex flex-col gap-6px'>
          <span className='text-13px text-t-secondary'>{t('outbound.hire.nameLabel', { defaultValue: '员工名称' })}</span>
          <Input
            value={name}
            onChange={setName}
            placeholder={t('outbound.hire.namePlaceholder', { defaultValue: '例如：小助手、客服小美' })}
            maxLength={30}
            onPressEnter={() => void submit()}
          />
        </div>
        <div className='flex flex-col gap-6px'>
          <span className='text-13px text-t-secondary'>{t('outbound.hire.characterLabel', { defaultValue: '形象' })}</span>
          <CharacterPicker
            value={figure ? CUSTOM_CHARACTER_ID : character}
            figureId={figure?.id}
            onSelectCharacter={(id) => {
              setCharacter(id);
              setFigure(null);
            }}
            onSelectFigure={(fig) => setFigure(fig)}
          />
        </div>
      </div>
    </Modal>
  );
};

export default HireEmployeeModal;
