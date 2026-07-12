/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useState } from 'react';
import { Down } from '@icon-park/react';
import classNames from 'classnames';
import { useTranslation } from 'react-i18next';

export interface LocalModelDetailsProps {
  forcedOpen?: boolean;
  className?: string;
  children: React.ReactNode;
}

const LocalModelDetails: React.FC<LocalModelDetailsProps> = ({ forcedOpen = false, className, children }) => {
  const { t } = useTranslation();
  const [manualOpen, setManualOpen] = useState(false);
  const open = manualOpen || forcedOpen;

  return (
    <div className={classNames('mt-10px border-t border-solid border-[var(--color-border-2)] pt-8px', className)}>
      <button
        type='button'
        aria-expanded={open}
        aria-disabled={forcedOpen}
        onClick={() => !forcedOpen && setManualOpen((current) => !current)}
        className={classNames(
          'h-28px w-full border-none bg-transparent px-1px flex items-center justify-between gap-8px text-11px font-500 text-t-secondary transition-colors',
          forcedOpen ? 'cursor-default' : 'cursor-pointer hover:text-t-primary'
        )}
      >
        <span>
          {t(
            open
              ? 'settings.modelHub.local.capabilityCenter.collapseDetails'
              : 'settings.modelHub.local.capabilityCenter.details'
          )}
        </span>
        <Down
          theme='outline'
          size='13'
          className={classNames('transition-transform duration-180', open && 'rotate-180')}
        />
      </button>
      {open && <div className='pt-7px'>{children}</div>}
    </div>
  );
};

export default LocalModelDetails;
