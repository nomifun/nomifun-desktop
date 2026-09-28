/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { useEffect, useId, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import type { ICompanionProfile, ICompanionWithStatus } from '@/common/adapter/ipcBridge';
import type { CompanionId } from '@/common/types/ids';
import InstantHoverTooltip from '@/renderer/components/base/InstantHoverTooltip';
import CompanionAvatar from './CompanionAvatar';
import { customFigureMetaOf } from './characters/customMeta';

interface CompanionSwitcherProps {
  companionId: CompanionId | null;
  profile: ICompanionProfile | null;
  roster: ICompanionWithStatus[];
  switchingCompanionId: CompanionId | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSwitch: (id: CompanionId) => void;
  onShowAll: () => void;
}

export default function CompanionSwitcher({
  companionId, profile, roster, switchingCompanionId, open, onOpenChange, onSwitch, onShowAll,
}: CompanionSwitcherProps) {
  const { t } = useTranslation();
  const rootRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const railRef = useRef<HTMLDivElement>(null);
  const railId = useId();
  const alternatives = roster.filter((item) => item.companion_id !== companionId);
  const targets = alternatives.slice(0, 3);
  const overflow = alternatives.length - targets.length;

  useEffect(() => {
    if (!open) return;
    const dismiss = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) onOpenChange(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return;
      onOpenChange(false);
      triggerRef.current?.focus();
    };
    const onWindowBlur = () => onOpenChange(false);
    document.addEventListener('pointerdown', dismiss, true);
    document.addEventListener('keydown', onKeyDown);
    window.addEventListener('blur', onWindowBlur);
    return () => {
      document.removeEventListener('pointerdown', dismiss, true);
      document.removeEventListener('keydown', onKeyDown);
      window.removeEventListener('blur', onWindowBlur);
    };
  }, [open, onOpenChange]);

  if (targets.length === 0) return null;

  const focusChoice = (last = false) => {
    const buttons = railRef.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)');
    (last ? buttons?.[buttons.length - 1] : buttons?.[0])?.focus();
  };

  return (
    <div
      ref={rootRef}
      className='nomi-companion-switcher'
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) onOpenChange(false);
      }}
    >
      <InstantHoverTooltip content={profile?.name || 'Nomi'} position='left' className='nomi-companion-switcher__tooltip'>
        <button
          ref={triggerRef}
          type='button'
          className='nomi-companion-switcher__trigger'
          aria-label={t('nomi.companion.switchCompanion')}
          aria-expanded={open}
          aria-controls={railId}
          onClick={() => onOpenChange(!open)}
          onKeyDown={(event) => {
            if (event.key !== 'ArrowUp' && event.key !== 'ArrowDown') return;
            event.preventDefault();
            if (!open) {
              onOpenChange(true);
              // Wait for React to remove inert from the rail before focusing it.
              requestAnimationFrame(() => focusChoice(event.key === 'ArrowUp'));
            } else focusChoice(event.key === 'ArrowUp');
          }}
        >
          <CompanionAvatar character={profile?.character} companionId={companionId ?? undefined}
            customFigure={customFigureMetaOf(profile)} mood='content' activity='idle' size={24} />
        </button>
      </InstantHoverTooltip>
      <div
        ref={railRef}
        id={railId}
        className={`nomi-companion-switcher__rail${open ? ' is-open' : ''}`}
        role='group'
        aria-label={t('nomi.companion.switchCompanion')}
        aria-hidden={!open}
        inert={!open}
        data-companion-hit={open ? '' : undefined}
        onKeyDown={(event) => {
          if (event.key !== 'ArrowUp' && event.key !== 'ArrowDown') return;
          event.preventDefault();
          const buttons = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('button:not(:disabled)'));
          const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
          buttons[(current + (event.key === 'ArrowDown' ? 1 : buttons.length - 1)) % buttons.length]?.focus();
        }}
      >
        {targets.map((item) => {
          const switching = switchingCompanionId === item.companion_id;
          return (
            <InstantHoverTooltip key={item.companion_id} content={item.name} position='left' className='nomi-companion-switcher__tooltip' disabled={!open}>
              <button
                type='button'
                className={`nomi-companion-switcher__item${switching ? ' is-switching' : ''}`}
                aria-label={t('nomi.companion.switchTo', { name: item.name })}
                aria-busy={switching || undefined}
                disabled={switchingCompanionId !== null}
                onClick={() => {
                  onOpenChange(false);
                  triggerRef.current?.focus();
                  onSwitch(item.companion_id);
                }}
              >
                <CompanionAvatar character={item.character} companionId={item.companion_id}
                  customFigure={customFigureMetaOf(item)} mood='content' activity='idle' size={24} />
              </button>
            </InstantHoverTooltip>
          );
        })}
        {overflow > 0 && (
          <InstantHoverTooltip content={t('nomi.companion.showAll', { count: roster.length })} position='left' className='nomi-companion-switcher__tooltip' disabled={!open}>
            <button
              type='button'
              className='nomi-companion-switcher__more'
              aria-label={t('nomi.companion.showAll', { count: roster.length })}
              disabled={switchingCompanionId !== null}
              onClick={() => { onOpenChange(false); onShowAll(); }}
            >
              +{overflow}
            </button>
          </InstantHoverTooltip>
        )}
      </div>
    </div>
  );
}
