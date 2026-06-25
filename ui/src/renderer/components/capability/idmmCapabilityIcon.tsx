/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Brain } from '@icon-park/react';
import classNames from 'classnames';
import React from 'react';

export interface IdmmCapabilityIconOptions {
  size: number | string;
  className?: string;
  spinning?: boolean;
}

/**
 * Shared "智能决策" icon. Keep session-list capability markers and the
 * per-session IDMM control visually aligned by rendering the same glyph.
 */
export const renderIdmmCapabilityIcon = ({ size, className, spinning = false }: IdmmCapabilityIconOptions): React.ReactElement => (
  <Brain
    theme='outline'
    size={size}
    fill='currentColor'
    className={classNames('block', spinning && 'autowork-spin', className)}
  />
);
