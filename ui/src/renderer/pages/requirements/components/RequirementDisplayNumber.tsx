/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Message, Tooltip } from '@arco-design/web-react';
import { Copy } from '@icon-park/react';
import React, { useCallback } from 'react';
import { useTranslation } from 'react-i18next';

import { copyText } from '@/renderer/utils/ui/clipboard';

interface RequirementDisplayNumberProps {
  displayNo: number;
  /** Supplying the canonical ID turns the badge into a full-ID copy action. */
  fullId?: string;
  className?: string;
}

const RequirementDisplayNumber: React.FC<RequirementDisplayNumberProps> = ({
  displayNo,
  fullId,
  className,
}) => {
  const { t } = useTranslation();
  const label = `#${displayNo}`;

  const copyFullId = useCallback(
    (event: React.SyntheticEvent) => {
      event.stopPropagation();
      if (!fullId) return;
      copyText(fullId)
        .then(() => Message.success(t('common.copySuccess')))
        .catch(() => Message.error(t('common.copyFailed')));
    },
    [fullId, t]
  );

  const badge = (
    <span
      role={fullId ? 'button' : undefined}
      tabIndex={fullId ? 0 : undefined}
      aria-label={fullId ? `${label}, ${t('common.copyFullId')}` : label}
      onClick={fullId ? copyFullId : undefined}
      onKeyDown={
        fullId
          ? (event) => {
              if (event.key === 'Enter' || event.key === ' ') {
                event.preventDefault();
                copyFullId(event);
              }
            }
          : undefined
      }
      className={[
        'inline-flex h-24px min-w-48px flex-shrink-0 items-center justify-center gap-4px rounded-6px border border-solid px-6px',
        'border-[var(--color-border-2)] bg-[var(--color-fill-1)] font-mono text-11px font-medium leading-none tabular-nums text-[var(--color-text-3)]',
        fullId
          ? 'cursor-copy transition-colors hover:border-[var(--color-primary-light-4)] hover:bg-[var(--color-primary-light-1)] hover:text-[rgb(var(--primary-6))] focus-visible:outline-2 focus-visible:outline-[rgb(var(--primary-6))]'
          : '',
        className,
      ]
        .filter(Boolean)
        .join(' ')}
    >
      <span>{label}</span>
      {fullId ? <Copy theme='outline' size={10} strokeWidth={3} aria-hidden /> : null}
    </span>
  );

  return fullId ? <Tooltip content={t('common.copyFullId')}>{badge}</Tooltip> : badge;
};

export default RequirementDisplayNumber;
