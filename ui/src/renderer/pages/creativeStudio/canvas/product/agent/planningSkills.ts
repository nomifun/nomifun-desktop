/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

export const CREATIVE_STUDIO_PLANNING_SKILLS = [
  {
    id: 'creative-studio-canvas',
    label: '画布规划',
    description: '理解当前选择并提出安全的文本与结构操作。',
  },
  {
    id: 'creative-studio-organize',
    label: '整理布局',
    description: '调整现有节点的位置、尺寸与连接关系。',
  },
  {
    id: 'creative-studio-workflow',
    label: '工作流设计',
    description: '把创作目标整理成可人工确认的工作流草案。',
  },
] as const;

export type CreativeStudioPlanningSkillId =
  (typeof CREATIVE_STUDIO_PLANNING_SKILLS)[number]['id'];

export const DEFAULT_CREATIVE_STUDIO_PLANNING_SKILL_IDS = [
  'creative-studio-canvas',
] as const satisfies readonly CreativeStudioPlanningSkillId[];

export const isCreativeStudioPlanningSkillId = (
  value: string
): value is CreativeStudioPlanningSkillId =>
  CREATIVE_STUDIO_PLANNING_SKILLS.some((skill) => skill.id === value);
