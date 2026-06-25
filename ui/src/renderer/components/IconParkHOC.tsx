/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

import React from 'react';
import { IconProvider, DEFAULT_ICON_CONFIGS } from '@icon-park/react/es/runtime';
import { theme } from '@/platform';
import { iconColors } from '@/renderer/styles/colors';

const IconParkHOC = <T extends Record<string, any>>(Component: React.FunctionComponent<T>): React.FC<T> => {
  return (props) => {
    return React.createElement(
      IconProvider,
      {
        value: {
          ...DEFAULT_ICON_CONFIGS,
          size: theme.Size.IconSize.normal,
        },
      },
      [
        React.createElement(Component, {
          key: 'c3',
          strokeWidth: 3,
          fill: iconColors.secondary,
          ...props,
          className: 'cursor-pointer  ' + ((props as any).className || ''),
        }),
      ]
    );
  };
};

export default IconParkHOC;
