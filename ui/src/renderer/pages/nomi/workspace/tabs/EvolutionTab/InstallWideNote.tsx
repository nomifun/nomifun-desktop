/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React from 'react';
import { useTranslation } from 'react-i18next';
import { Info } from '@icon-park/react';

/**
 * Honest disclosure for a section whose values still live in the install-wide
 * shared config (see `useEvolutionConfig` — the migration seam). One note per
 * section, rendered as the second line of the section description; it disappears
 * on its own once `installWide` turns false.
 *
 * `text` overrides the default sentence for a section that has more to disclose
 * than "install-wide" (e.g. 技能生成, whose output also lands elsewhere).
 */
const InstallWideNote: React.FC<{ text?: string }> = ({ text }) => {
  const { t } = useTranslation();
  return (
    <span className='mt-3px flex items-start gap-4px text-12px leading-18px text-t-tertiary'>
      <Info
        theme='outline'
        size='12'
        fill='currentColor'
        strokeWidth={3}
        className='line-height-0 mt-3px shrink-0'
      />
      <span className='min-w-0'>
        {text ??
          t('nomi.evolution.installWideNote', {
            defaultValue: '这组设置目前对所有伙伴共同生效，后续版本会改为按伙伴单独配置。',
          })}
      </span>
    </span>
  );
};

export default InstallWideNote;
