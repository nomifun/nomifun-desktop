/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

import { Steps } from '@arco-design/web-react';
import type { StepsProps } from '@arco-design/web-react/es/Steps';
import React from 'react';

interface StepsWrapperProps extends StepsProps {
  className?: string;
}

const StepsWrapper: React.FC<StepsWrapperProps> & { Step: typeof Steps.Step } = ({ className, ...props }) => {
  return <Steps {...props} className={`nomifun-steps ${className || ''}`} />;
};

StepsWrapper.Step = Steps.Step;

export default StepsWrapper;
