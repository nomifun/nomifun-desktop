/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { resolveBackendAssetUrl } from '@/renderer/utils/platform';

const buildLogoAssetUrl = (path: string): string =>
  resolveBackendAssetUrl(`/api/assets/logos/${path}`) ?? `/api/assets/logos/${path}`;

/**
 * Stable runtime provider family. The preset `value` is intentionally separate:
 * several commercial plans share one runtime family but have different manifests.
 */
export type PlatformType =
  | 'gemini'
  | 'gemini-vertex-ai'
  | 'anthropic'
  | 'openai'
  | 'custom'
  | 'new-api'
  | 'bedrock'
  | 'deepseek'
  | 'deepgram'
  | 'mimo'
  | 'mimo-token-plan-cn'
  | 'mimo-token-plan-sgp'
  | 'mimo-token-plan-ams'
  | 'minimax'
  | 'minimax-code'
  | 'minimax-coding-plan'
  | 'dashscope'
  | 'dashscope-coding'
  | 'siliconflow'
  | 'zhipu'
  | 'glm-coding-plan'
  | 'moonshot-cn'
  | 'moonshot-global'
  | 'ark'
  | 'ark-coding-plan'
  | 'ark-agent-plan'
  | 'qianfan'
  | 'qianfan-coding-plan'
  | 'hunyuan'
  | 'hunyuan-global'
  | 'lingyi'
  | 'stepfun'
  | 'stepfun-plan'
  | 'novita'
  | 'openrouter'
  | 'xai'
  | 'poe'
  | 'ppio'
  | 'modelscope'
  | 'infiniai'
  | 'ctyun'
  | (string & {});

/** UI-only metadata. Operational defaults come exclusively from the protocol manifest. */
export interface PlatformConfig {
  name: string;
  /** Unique manifest preset identifier. */
  value: string;
  logo: string | null;
  platform: PlatformType;
  i18nKey?: string;
}

export const MODEL_PLATFORMS: PlatformConfig[] = [
  { name: 'Custom', value: 'custom', logo: null, platform: 'custom', i18nKey: 'settings.platformCustom' },
  {
    name: 'New API',
    value: 'new-api',
    logo: buildLogoAssetUrl('ai-cloud/newapi.svg'),
    platform: 'new-api',
    i18nKey: 'settings.platformNewApi',
  },
  { name: 'Gemini', value: 'gemini', logo: buildLogoAssetUrl('ai-major/gemini.svg'), platform: 'gemini' },
  { name: 'OpenAI', value: 'OpenAI', logo: buildLogoAssetUrl('ai-major/openai.svg'), platform: 'openai' },
  {
    name: 'Anthropic',
    value: 'Anthropic',
    logo: buildLogoAssetUrl('ai-major/anthropic.svg'),
    platform: 'anthropic',
  },
  {
    name: 'AWS Bedrock (Anthropic Claude)',
    value: 'AWS-Bedrock',
    logo: buildLogoAssetUrl('ai-cloud/bedrock.svg'),
    platform: 'bedrock',
  },
  {
    name: 'DeepSeek',
    value: 'DeepSeek',
    logo: buildLogoAssetUrl('ai-major/deepseek.svg'),
    platform: 'deepseek',
  },
  { name: 'Deepgram', value: 'Deepgram', logo: null, platform: 'deepgram' },
  { name: 'Xiaomi MiMo', value: 'MiMo', logo: buildLogoAssetUrl('ai-china/mimo.svg'), platform: 'mimo' },
  {
    name: 'Xiaomi MiMo Token Plan (CN)',
    value: 'MiMo-Token-Plan-CN',
    logo: buildLogoAssetUrl('ai-china/mimo.svg'),
    platform: 'mimo-token-plan-cn',
  },
  {
    name: 'Xiaomi MiMo Token Plan (SGP)',
    value: 'MiMo-Token-Plan-SGP',
    logo: buildLogoAssetUrl('ai-china/mimo.svg'),
    platform: 'mimo-token-plan-sgp',
  },
  {
    name: 'Xiaomi MiMo Token Plan (AMS)',
    value: 'MiMo-Token-Plan-AMS',
    logo: buildLogoAssetUrl('ai-china/mimo.svg'),
    platform: 'mimo-token-plan-ams',
  },
  { name: 'MiniMax', value: 'MiniMax', logo: buildLogoAssetUrl('ai-china/minimax.png'), platform: 'minimax' },
  {
    name: 'MiniMax (International)',
    value: 'MiniMax-Code',
    logo: buildLogoAssetUrl('ai-china/minimax.png'),
    platform: 'minimax-code',
  },
  {
    name: 'MiniMax Token Plan (China)',
    value: 'MiniMax-Coding-Plan',
    logo: buildLogoAssetUrl('ai-china/minimax.png'),
    platform: 'minimax-coding-plan',
  },
  { name: 'Novita', value: 'Novita', logo: buildLogoAssetUrl('ai-cloud/novita.svg'), platform: 'novita' },
  {
    name: 'OpenRouter',
    value: 'OpenRouter',
    logo: buildLogoAssetUrl('ai-cloud/openrouter.svg'),
    platform: 'openrouter',
  },
  {
    name: 'Dashscope',
    value: 'Dashscope',
    logo: buildLogoAssetUrl('ai-china/qwen.svg'),
    platform: 'dashscope',
  },
  {
    name: 'Dashscope Coding Plan',
    value: 'Dashscope-Coding',
    logo: buildLogoAssetUrl('ai-china/qwen.svg'),
    platform: 'dashscope-coding',
  },
  {
    name: 'SiliconFlow-CN',
    value: 'SiliconFlow-CN',
    logo: buildLogoAssetUrl('ai-cloud/siliconflow.png'),
    platform: 'siliconflow',
  },
  {
    name: 'SiliconFlow',
    value: 'SiliconFlow',
    logo: buildLogoAssetUrl('ai-cloud/siliconflow.png'),
    platform: 'siliconflow',
  },
  { name: 'Zhipu', value: 'Zhipu', logo: buildLogoAssetUrl('ai-china/zhipu.svg'), platform: 'zhipu' },
  {
    name: 'GLM Coding Plan',
    value: 'GLM-Coding-Plan',
    logo: buildLogoAssetUrl('ai-china/zhipu.svg'),
    platform: 'glm-coding-plan',
  },
  {
    name: 'Moonshot (China)',
    value: 'Moonshot',
    logo: buildLogoAssetUrl('ai-china/kimi.svg'),
    platform: 'moonshot-cn',
  },
  {
    name: 'Moonshot (Global)',
    value: 'Moonshot-Global',
    logo: buildLogoAssetUrl('ai-china/kimi.svg'),
    platform: 'moonshot-global',
  },
  { name: 'xAI', value: 'xAI', logo: buildLogoAssetUrl('ai-major/xai.svg'), platform: 'xai' },
  {
    name: 'Doubao / Ark',
    value: 'Ark',
    logo: buildLogoAssetUrl('ai-china/volcengine.svg'),
    platform: 'ark',
  },
  {
    name: 'Doubao / Ark Coding Plan',
    value: 'Ark-Coding-Plan',
    logo: buildLogoAssetUrl('ai-china/volcengine.svg'),
    platform: 'ark-coding-plan',
  },
  {
    name: 'Doubao / Ark Agent Plan',
    value: 'Ark-Agent-Plan',
    logo: buildLogoAssetUrl('ai-china/volcengine.svg'),
    platform: 'ark-agent-plan',
  },
  { name: 'Qianfan', value: 'Qianfan', logo: buildLogoAssetUrl('ai-china/baidu.svg'), platform: 'qianfan' },
  {
    name: 'Qianfan Coding Plan',
    value: 'Qianfan-Coding-Plan',
    logo: buildLogoAssetUrl('ai-china/baidu.svg'),
    platform: 'qianfan-coding-plan',
  },
  {
    name: 'Tencent TokenHub (China)',
    value: 'Hunyuan',
    logo: buildLogoAssetUrl('ai-china/tencent.svg'),
    platform: 'hunyuan',
  },
  {
    name: 'Tencent TokenHub (Global)',
    value: 'Hunyuan-Global',
    logo: buildLogoAssetUrl('ai-china/tencent.svg'),
    platform: 'hunyuan-global',
  },
  {
    name: 'Lingyi',
    value: 'Lingyi',
    logo: buildLogoAssetUrl('ai-china/lingyiwanwu.svg'),
    platform: 'lingyi',
  },
  { name: 'Poe', value: 'Poe', logo: buildLogoAssetUrl('ai-cloud/poe.svg'), platform: 'poe' },
  { name: 'PPIO', value: 'PPIO', logo: buildLogoAssetUrl('ai-cloud/ppio.svg'), platform: 'ppio' },
  {
    name: 'ModelScope',
    value: 'ModelScope',
    logo: buildLogoAssetUrl('ai-cloud/modelscope.svg'),
    platform: 'modelscope',
  },
  {
    name: 'InfiniAI',
    value: 'InfiniAI',
    logo: buildLogoAssetUrl('ai-cloud/infiniai.svg'),
    platform: 'infiniai',
  },
  { name: 'Ctyun', value: 'Ctyun', logo: buildLogoAssetUrl('ai-cloud/ctyun.svg'), platform: 'ctyun' },
  {
    name: 'StepFun',
    value: 'StepFun',
    logo: buildLogoAssetUrl('ai-china/stepfun.svg'),
    platform: 'stepfun',
  },
  {
    name: 'StepFun Step Plan',
    value: 'StepFun-Plan',
    logo: buildLogoAssetUrl('ai-china/stepfun.svg'),
    platform: 'stepfun-plan',
  },
];

export const getPlatformByValue = (value: string): PlatformConfig | undefined =>
  MODEL_PLATFORMS.find((platform) => platform.value === value);

export const getProviderLogo = ({
  name,
  platform,
}: {
  name?: string;
  platform?: string;
}): string | null => {
  if (!name && !platform) return null;
  if (platform) {
    const byPlatform = MODEL_PLATFORMS.find((item) => item.platform === platform && item.logo);
    if (byPlatform?.logo) return byPlatform.logo;
  }
  if (!name) return null;
  const normalizedName = name.trim().toLowerCase();
  return MODEL_PLATFORMS.find((item) => item.name.toLowerCase() === normalizedName && item.logo)?.logo ?? null;
};

export const isGeminiPlatform = (platform: PlatformType): boolean =>
  platform === 'gemini' || platform === 'gemini-vertex-ai';

export const isCustomOption = (value: string): boolean => value === 'custom';

export { isNewApiPlatform } from '@/common/utils/platformConstants';
