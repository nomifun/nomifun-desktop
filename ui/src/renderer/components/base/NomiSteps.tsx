/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Steps } from '@arco-design/web-react';
import type { StepsProps } from '@arco-design/web-react/es/Steps';
import classNames from 'classnames';
import React from 'react';

/**
 * 步骤条组件属性 / Steps component props
 */
export interface NomiStepsProps extends StepsProps {
  /** 额外的类名 / Additional class name */
  className?: string;
}

/**
 * 步骤条组件 / Steps component
 *
 * 基于 Arco Design Steps 的封装，提供统一的样式主题
 * Wrapper around Arco Design Steps with unified theme styling
 *
 * @features
 * - 自定义品牌色主题 / Custom brand color theme
 * - 完成态的特殊样式处理 / Special styling for finished state
 * - 完整的 Arco Steps API 支持 / Full Arco Steps API support
 *
 * @example
 * ```tsx
 * // 基本用法 / Basic usage
 * <NomiSteps current={1}>
 *   <NomiSteps.Step title="步骤1" description="这是描述" />
 *   <NomiSteps.Step title="步骤2" description="这是描述" />
 *   <NomiSteps.Step title="步骤3" description="这是描述" />
 * </NomiSteps>
 *
 * // 垂直步骤条 / Vertical steps
 * <NomiSteps current={1} direction="vertical">
 *   <NomiSteps.Step title="步骤1" description="描述" />
 *   <NomiSteps.Step title="步骤2" description="描述" />
 * </NomiSteps>
 *
 * // 带图标的步骤条 / Steps with icons
 * <NomiSteps current={1}>
 *   <NomiSteps.Step title="完成" icon={<IconCheck />} />
 *   <NomiSteps.Step title="进行中" icon={<IconLoading />} />
 *   <NomiSteps.Step title="待处理" icon={<IconClock />} />
 * </NomiSteps>
 *
 * // 迷你版步骤条 / Mini steps
 * <NomiSteps current={1} size="small" type="dot">
 *   <NomiSteps.Step title="步骤1" />
 *   <NomiSteps.Step title="步骤2" />
 *   <NomiSteps.Step title="步骤3" />
 * </NomiSteps>
 * ```
 *
 * @see arco-override.css for custom styles (.nomifun-steps)
 */
const NomiSteps: React.FC<NomiStepsProps> & { Step: typeof Steps.Step } = ({ className, ...props }) => {
  return <Steps {...props} className={classNames('nomifun-steps', className)} />;
};

NomiSteps.displayName = 'NomiSteps';

// 导出子组件 / Export sub-component
NomiSteps.Step = Steps.Step;

export default NomiSteps;
