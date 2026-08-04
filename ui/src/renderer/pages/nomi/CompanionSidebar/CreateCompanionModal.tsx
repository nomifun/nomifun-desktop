/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Input, Message, Modal } from '@arco-design/web-react';
import { ipcBridge } from '@/common';
import { CUSTOM_CHARACTER_ID, DEFAULT_CHARACTER_ID } from '@renderer/pages/companion/characters';
import CharacterPicker from '../CharacterPicker';
import { figureToCustomPatch } from '../useFigures';
import type { ICompanionProfile, IFigureMeta } from '@/common/adapter/ipcBridge';

interface Props {
  visible: boolean;
  onCancel: () => void;
  onCreated: (profile: ICompanionProfile) => void | Promise<void>;
}

/**
 * 新建伙伴 — name plus appearance, the only two things needed before a companion
 * exists. Everything else is configured afterwards in 总览.
 *
 * Extracted from the former CompanionSessionRail so the sidebar stays a pure
 * roster view and creation is owned by the page shell.
 */
const CreateCompanionModal: React.FC<Props> = ({ visible, onCancel, onCreated }) => {
  const { t } = useTranslation();
  const [name, setName] = useState('');
  const [character, setCharacter] = useState<string>(DEFAULT_CHARACTER_ID);
  /** A library figure chosen for the new companion (overrides `character`). */
  const [figure, setFigure] = useState<IFigureMeta | null>(null);
  const [creating, setCreating] = useState(false);

  // Reset on each open so a cancelled attempt never leaks into the next one.
  React.useEffect(() => {
    if (!visible) return;
    setName('');
    setCharacter(DEFAULT_CHARACTER_ID);
    setFigure(null);
  }, [visible]);

  const submit = async () => {
    const trimmed = name.trim();
    if (!trimmed || creating) return;
    setCreating(true);
    try {
      const profile = await ipcBridge.companion.createCompanion.invoke({
        name: trimmed,
        character: figure ? CUSTOM_CHARACTER_ID : character,
      });
      // createCompanion only accepts name + character; a library figure is linked
      // by a follow-up patch before the roster refresh in onCreated.
      if (figure) {
        await ipcBridge.companion.patchCompanion.invoke({
          companion_id: profile.companion_id,
          patch: { appearance: { custom_figure: figureToCustomPatch(figure) } },
        });
      }
      onCancel();
      try {
        await onCreated(profile);
      } catch (refreshError) {
        Message.warning(`${t('nomi.companions.created', { companionName: profile.name })}: ${String(refreshError)}`);
        return;
      }
      Message.success(t('nomi.companions.created', { companionName: profile.name }));
    } catch (error) {
      Message.error(String(error));
    } finally {
      setCreating(false);
    }
  };

  return (
    <Modal
      title={t('nomi.companions.createTitle')}
      visible={visible}
      onOk={() => void submit()}
      onCancel={onCancel}
      okButtonProps={{ loading: creating, disabled: !name.trim() }}
      style={{ width: 560 }}
    >
      <div className='flex flex-col gap-14px'>
        <div className='flex flex-col gap-6px'>
          <span className='text-13px text-t-secondary'>{t('nomi.companions.nameLabel')}</span>
          <Input
            value={name}
            onChange={setName}
            placeholder={t('nomi.companions.namePlaceholder')}
            maxLength={30}
            onPressEnter={() => void submit()}
          />
        </div>
        <div className='flex flex-col gap-6px'>
          <span className='text-13px text-t-secondary'>{t('nomi.companions.characterLabel')}</span>
          <CharacterPicker
            value={figure ? CUSTOM_CHARACTER_ID : character}
            figureId={figure?.figure_id}
            onSelectCharacter={(id) => {
              setCharacter(id);
              setFigure(null);
            }}
            onSelectFigure={setFigure}
          />
        </div>
      </div>
    </Modal>
  );
};

export default CreateCompanionModal;
