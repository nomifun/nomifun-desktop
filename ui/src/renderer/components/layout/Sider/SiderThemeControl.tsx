/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Message, Modal, Popover, Tooltip } from '@arco-design/web-react';
import { CheckOne, EditTwo, Plus, Theme } from '@icon-park/react';
import classNames from 'classnames';
import { ThemeSwitcher } from '@renderer/components/settings/ThemeSwitcher';
import FontSizeControl from '@renderer/components/settings/FontSizeControl';
import CssThemeModal from '@renderer/pages/settings/DisplaySettings/CssThemeModal';
import { useCssTheme } from '@renderer/hooks/ui/useCssTheme';
import type { ICssTheme } from '@/common/config/storage';
import type { SiderTooltipProps } from '@renderer/utils/ui/siderTooltip';

interface SiderThemeControlProps {
  isMobile: boolean;
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

const footerButtonClass = (collapsed: boolean, isMobile: boolean, active: boolean) =>
  classNames(
    'h-34px shrink-0 flex items-center justify-center cursor-pointer rd-0.5rem transition-colors',
    collapsed ? 'w-full' : 'w-36px',
    isMobile && 'sider-footer-btn-mobile',
    active ? '!bg-primary-1 !text-primary-6' : 'text-t-secondary hover:bg-fill-2 hover:text-t-primary active:bg-fill-3'
  );

/**
 * SiderThemeControl — the footer theme entry that lives right next to 设置.
 *
 * Now the complete home for everything the former Display settings page covered:
 * a single always-visible button opens a popover with the light/dark axis
 * (ThemeSwitcher), interface scaling (FontSizeControl), and the CSS preset/skin
 * list (via the shared `useCssTheme` hook). Each preset gets a hover edit
 * affordance and a trailing "add CSS" entry, both opening the self-contained
 * `CssThemeModal` — so the dedicated Display page can be dissolved entirely.
 */
const SiderThemeControl: React.FC<SiderThemeControlProps> = ({ isMobile, collapsed, siderTooltipProps }) => {
  const { t } = useTranslation();
  const { themes, activeThemeId, selectTheme, saveUserTheme, deleteUserTheme } = useCssTheme();
  const [popupVisible, setPopupVisible] = useState(false);
  const [modalVisible, setModalVisible] = useState(false);
  const [editingTheme, setEditingTheme] = useState<ICssTheme | null>(null);

  // Opening the editor always closes the popover first so the modal isn't
  // anchored inside a popup that vanishes when focus moves.
  const openModal = (theme: ICssTheme | null) => {
    setPopupVisible(false);
    setEditingTheme(theme);
    setModalVisible(true);
  };

  const closeModal = () => {
    setModalVisible(false);
    setEditingTheme(null);
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

  const popoverContent = (
    <div className='w-320px flex flex-col gap-12px py-4px'>
      {/* 明暗 / Light–dark */}
      <div className='flex flex-col gap-6px'>
        <div className='text-12px font-500 text-t-tertiary px-2px'>{t('settings.theme')}</div>
        <ThemeSwitcher />
      </div>

      {/* 界面缩放 / Interface scaling */}
      <div className='flex flex-col gap-6px'>
        <div className='text-12px font-500 text-t-tertiary px-2px'>{t('settings.fontSize')}</div>
        <FontSizeControl />
      </div>

      {/* CSS 预设主题 / CSS preset themes */}
      <div className='flex flex-col gap-6px'>
        <div className='text-12px font-500 text-t-tertiary px-2px'>{t('settings.cssTheme.selectOrCustomize')}</div>
        <div className='flex flex-col gap-2px max-h-300px overflow-y-auto -mx-4px px-4px'>
          {themes.map((theme) => {
            const active = activeThemeId === theme.id;
            const accent = pickAccent(theme.css || '');
            return (
              <div
                key={theme.id}
                className={classNames(
                  'group flex items-center gap-8px h-32px px-8px rd-8px text-left transition-colors',
                  active ? '!bg-primary-1' : 'hover:bg-fill-2'
                )}
              >
                <button
                  type='button'
                  onClick={() => void selectTheme(theme)}
                  className='flex-1 min-w-0 flex items-center gap-8px cursor-pointer border-none bg-transparent p-0 text-left'
                >
                  <span
                    className='size-14px rd-full shrink-0 border border-solid border-[var(--color-border-2)]'
                    style={accent ? { background: accent } : { background: 'var(--color-fill-3)' }}
                  />
                  <span
                    className={classNames(
                      'flex-1 min-w-0 truncate text-13px',
                      active ? 'text-primary-6 font-500' : 'text-t-primary'
                    )}
                  >
                    {theme.name}
                  </span>
                </button>
                {active && <CheckOne theme='filled' size='15' fill='rgb(var(--primary-6))' className='shrink-0' />}
                <button
                  type='button'
                  onClick={() => openModal(theme)}
                  aria-label={t('settings.cssTheme.editTheme')}
                  className='shrink-0 opacity-0 group-hover:opacity-100 size-22px flex items-center justify-center rd-6px text-t-tertiary hover:text-primary-6 hover:bg-fill-3 cursor-pointer border-none bg-transparent transition-opacity'
                >
                  <EditTwo theme='outline' size='13' fill='currentColor' />
                </button>
              </div>
            );
          })}

          {/* 手动添加 CSS 样式 / Manually add a CSS theme */}
          <button
            type='button'
            onClick={() => openModal(null)}
            className='flex items-center gap-8px h-32px px-8px rd-8px text-13px text-t-secondary hover:text-primary-6 hover:bg-fill-2 cursor-pointer border-none bg-transparent transition-colors'
          >
            <span className='size-14px shrink-0 flex items-center justify-center'>
              <Plus theme='outline' size='14' fill='currentColor' />
            </span>
            <span className='flex-1 min-w-0 truncate text-left'>{t('settings.cssTheme.addManually')}</span>
          </button>
        </div>
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
          <div className={footerButtonClass(collapsed, isMobile, popupVisible)} aria-label={t('settings.theme')}>
            <Theme theme='outline' size='18' fill='currentColor' className='block leading-none' style={{ lineHeight: 0 }} />
          </div>
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
