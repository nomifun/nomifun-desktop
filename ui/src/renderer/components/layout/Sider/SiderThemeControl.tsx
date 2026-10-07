/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Message, Modal, Popover, Tooltip } from '@arco-design/web-react';
import { Check, EditTwo, Plus, Theme } from '@icon-park/react';
import classNames from 'classnames';
import { ThemeSwitcher } from '@renderer/components/settings/ThemeSwitcher';
import FontSizeControl from '@renderer/components/settings/FontSizeControl';
import CssThemeModal from '@renderer/pages/settings/DisplaySettings/CssThemeModal';
import { getCssThemeDisplayName } from '@renderer/pages/settings/DisplaySettings/presets';
import { useCssTheme } from '@renderer/hooks/ui/useCssTheme';
import type { ICssTheme } from '@/common/config/storage';
import type { SiderTooltipProps } from '@renderer/utils/ui/siderTooltip';
import './SiderThemeControl.css';

interface SiderThemeControlProps {
  collapsed: boolean;
  siderTooltipProps: SiderTooltipProps;
}

/** Pull a representative accent color out of a preset's CSS for the swatch dot. */
const pickAccent = (css: string): string | null => {
  const match = css.match(/--(?:color-primary|primary-6)\s*:\s*([^;!}]+)/i);
  if (!match) return null;
  const value = match[1].trim().replace(/\s*!important\s*/i, '');
  if (!value || /var\(/i.test(value)) return null;
  if (/^\d{1,3}\s*,\s*\d{1,3}\s*,\s*\d{1,3}$/.test(value)) return `rgb(${value})`;
  return value;
};

const footerButtonClass = (collapsed: boolean, active: boolean) =>
  classNames(
    'sider-theme-trigger h-28px shrink-0 flex items-center justify-center cursor-pointer rd-0.5rem transition-colors',
    collapsed ? 'w-full' : 'w-36px',
    active ? '!bg-primary-1 !text-primary-6' : 'text-t-secondary hover:bg-fill-2 hover:text-t-primary active:bg-fill-3'
  );

/** Desktop appearance popover: compact mode/zoom controls and a two-column theme grid. */
const SiderThemeControl: React.FC<SiderThemeControlProps> = ({ collapsed, siderTooltipProps }) => {
  const { t } = useTranslation();
  const { themes, activeThemeId, selectTheme, saveUserTheme, deleteUserTheme } = useCssTheme();
  const [popupVisible, setPopupVisible] = useState(false);
  const [modalVisible, setModalVisible] = useState(false);
  const [editingTheme, setEditingTheme] = useState<ICssTheme | null>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const returnThemeId = useRef<string | null>(null);
  const restoreEditorFocus = useRef(false);

  useEffect(() => {
    if (!popupVisible) return;
    const frame = requestAnimationFrame(() => {
      const panel = panelRef.current;
      if (!panel) return;
      const returning = restoreEditorFocus.current;
      const target = returning
        ? Array.from(panel.querySelectorAll<HTMLButtonElement>('[data-theme-choice]')).find(
            (button) => button.dataset.themeChoice === returnThemeId.current
          ) ?? panel.querySelector<HTMLButtonElement>('.sider-theme-add')
        : panel.querySelector<HTMLButtonElement>('[role="radio"][aria-checked="true"]');
      target?.focus({ preventScroll: true });
      if (returning) target?.scrollIntoView({ block: 'nearest' });
      restoreEditorFocus.current = false;
    });
    return () => cancelAnimationFrame(frame);
  }, [popupVisible]);

  // Opening the editor always closes the popover first so the modal isn't
  // anchored inside a popup that vanishes when focus moves.
  const openModal = (theme: ICssTheme | null) => {
    returnThemeId.current = theme?.id ?? null;
    setPopupVisible(false);
    setEditingTheme(theme);
    setModalVisible(true);
  };

  const closeModal = () => {
    setModalVisible(false);
    setEditingTheme(null);
    restoreEditorFocus.current = true;
    setPopupVisible(true);
  };

  const handleSave = async (data: Omit<ICssTheme, 'id' | 'created_at' | 'updated_at' | 'is_preset'>) => {
    await saveUserTheme(data, editingTheme);
    closeModal();
    Message.success(t('common.saveSuccess'));
  };

  // Delete is only offered for a real (non-preset) user theme.
  const canDelete = !!editingTheme && !editingTheme.is_preset;
  const handleDelete = () => {
    if (!editingTheme || editingTheme.is_preset) return;
    const target = editingTheme;
    Modal.confirm({
      title: t('common.confirmDelete'),
      content: t('settings.cssTheme.deleteConfirm'),
      okButtonProps: { status: 'danger' },
      onOk: async () => {
        await deleteUserTheme(target.id);
        closeModal();
        Message.success(t('common.deleteSuccess'));
      },
    });
  };

  const activeTheme = themes.find((theme) => theme.id === activeThemeId);
  const activeThemeLabel = activeTheme
    ? t('settings.cssTheme.currentTheme', { name: getCssThemeDisplayName(activeTheme, t) })
    : '';

  const popoverContent = (
    <div
      ref={panelRef}
      className='sider-theme-panel'
      role='dialog'
      aria-label={t('settings.appearance')}
      onKeyDown={(event) => {
        if (event.key !== 'Escape' || event.defaultPrevented) return;
        event.preventDefault();
        event.stopPropagation();
        setPopupVisible(false);
        triggerRef.current?.focus({ preventScroll: true });
      }}
    >
      <div className='sider-theme-header'>
        <span className='sider-theme-title'>
          <Theme theme='outline' size='15' fill='currentColor' aria-hidden='true' />
          {t('settings.appearance')}
        </span>
        <ThemeSwitcher />
      </div>

      <FontSizeControl />

      <div className='sider-theme-presets'>
        <div className='sider-theme-section-label'>{t('settings.cssTheme.styleLabel')}</div>
        <div
          className='sider-theme-grid'
          aria-label={t('settings.cssTheme.selectOrCustomize')}
          onKeyDown={(event) => {
            if (!['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return;
            const buttons = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('[data-theme-choice]'));
            const index = buttons.indexOf(event.target as HTMLButtonElement);
            if (index < 0) return;
            event.preventDefault();
            const offset = event.key === 'ArrowLeft' ? -1 : event.key === 'ArrowRight' ? 1 : event.key === 'ArrowUp' ? -2 : 2;
            const nextIndex = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : index + offset;
            buttons[Math.min(buttons.length - 1, Math.max(0, nextIndex))]?.focus();
          }}
        >
          {themes.map((theme) => {
            const active = activeThemeId === theme.id;
            const accent = pickAccent(theme.css || '');
            const displayName = getCssThemeDisplayName(theme, t);
            return (
              <div key={theme.id} className={classNames('sider-theme-entry', { 'is-active': active })}>
                <button
                  type='button'
                  data-theme-choice={theme.id}
                  aria-pressed={active}
                  title={displayName}
                  onClick={() => void selectTheme(theme)}
                  className='sider-theme-choice'
                >
                  <span
                    className='sider-theme-swatch'
                    aria-hidden='true'
                    style={{ background: accent ?? 'var(--color-fill-3)' }}
                  />
                  <span className='sider-theme-name'>{displayName}</span>
                </button>
                {active && (
                  <span className='sider-theme-check' aria-hidden='true'>
                    <Check theme='outline' size='13' fill='currentColor' />
                  </span>
                )}
                <button
                  type='button'
                  onClick={() => openModal(theme)}
                  aria-label={t('settings.cssTheme.editTheme') + ': ' + displayName}
                  title={t('settings.cssTheme.editTheme')}
                  className='sider-theme-edit'
                >
                  <EditTwo theme='outline' size='12' fill='currentColor' />
                </button>
              </div>
            );
          })}
          <button
            type='button'
            onClick={() => openModal(null)}
            className='sider-theme-add'
            aria-label={t('settings.cssTheme.addToPreset')}
            title={t('settings.cssTheme.addToPreset')}
          >
            <Plus theme='outline' size='13' fill='currentColor' aria-hidden='true' />
            <span>{t('settings.cssTheme.addCustom')}</span>
          </button>
        </div>
      </div>

      <div className='sider-theme-status'>
        <span className='sider-theme-applied' title={activeThemeLabel}>
          {activeTheme && <Check theme='outline' size='11' fill='currentColor' aria-hidden='true' />}
          <span>{activeThemeLabel}</span>
        </span>
        <span className='sider-theme-escape'><kbd>Esc</kbd>{t('common.close')}</span>
      </div>
    </div>
  );

  return (
    <>
      <Popover
        className='sider-soft-popover sider-theme-popover'
        trigger='click'
        position={collapsed ? 'rt' : 'top'}
        popupVisible={popupVisible}
        onVisibleChange={setPopupVisible}
        getPopupContainer={() => document.body}
        content={popoverContent}
        unmountOnExit
      >
        <Tooltip {...siderTooltipProps} content={t('settings.theme')} position='right'>
          <button
            ref={triggerRef}
            type='button'
            className={footerButtonClass(collapsed, popupVisible)}
            aria-label={t('settings.theme')}
            aria-haspopup='dialog'
            aria-expanded={popupVisible}
          >
            <Theme theme='outline' size='18' fill='currentColor' className='block leading-none' style={{ lineHeight: 0 }} />
          </button>
        </Tooltip>
      </Popover>

      <CssThemeModal
        visible={modalVisible}
        theme={editingTheme}
        onClose={closeModal}
        onSave={(data) => void handleSave(data)}
        onDelete={canDelete ? handleDelete : undefined}
      />
    </>
  );
};

export default SiderThemeControl;
