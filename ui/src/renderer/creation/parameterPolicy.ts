import { imageGenerationSizePolicyForModel, type ImageGenerationAspectRatioOption, type ImageGenerationModelOption, type ImageGenerationSizePolicy } from './parameters/image';
import type { CreationMode, CreationParameters, CreationInput } from './types';

type Model = Pick<ImageGenerationModelOption, 'model' | 'protocol' | 'platform'> | null | undefined;
export function creationVideoInputRoles(model: Model): CreationInput['role'][] {
  if (model?.protocol === 'ark.video_jobs' || (model?.protocol === 'xai.video_jobs' && model.model === 'grok-imagine-video-1.5')) return ['reference', 'first_frame', 'last_frame'];
  return ['reference', 'first_frame'];
}
const sizes = (values: string[]) => values.map(value => {
  const [width, height] = value.split('x').map(Number);
  let divisor = width, remainder = height;
  while (remainder) [divisor, remainder] = [remainder, divisor % remainder];
  return { value, requestSize: value, label: value, width, height, aspectRatio: `${width / divisor}:${height / divisor}`, resolution: String(Math.max(width, height)) };
});

/** Small protocol policy over the existing image size rules. Unknown options stay automatic. */
export function creationImageSizePolicy(model: Model): ImageGenerationSizePolicy {
  const base = imageGenerationSizePolicyForModel(model);
  if (model?.protocol !== 'openai.images') return base;
  if (model.model === 'dall-e-3') return { options: sizes(['1024x1024', '1792x1024', '1024x1792']), maxCount: 1, allowCustomDimensions: false };
  if (model.model === 'dall-e-2') return { options: sizes(['256x256', '512x512', '1024x1024']), maxCount: 10, allowCustomDimensions: false };
  if (model.model.startsWith('gpt-image-')) return { options: [...sizes(['1024x1024', '1536x1024', '1024x1536']), { value: 'auto', label: '自动', width: null, height: null }], maxCount: 10, allowCustomDimensions: false };
  return base;
}
export function creationParameterPolicy(model: Model) {
  const protocol = model?.protocol;
  const qualities = protocol === 'openai.images' && model?.model.startsWith('gpt-image-') ? ['auto', 'high', 'medium', 'low'] : protocol === 'openai.images' && model?.model === 'dall-e-3' ? ['standard', 'hd'] : [];
  const video = protocol === 'openai.videos'
    ? { seconds: [4, 8, 12], sizes: ['1280x720', '720x1280', ...(model?.model === 'sora-2-pro' ? ['1792x1024', '1024x1792'] : [])] }
    : protocol === 'ark.video_jobs' ? { seconds: [5, 10], sizes: ['1280x720', '720x1280', '1920x1080', '1080x1920'] }
    : protocol === 'zhipu.video_jobs' ? { seconds: [5, 10], sizes: ['1280x720', '1920x1080'] }
    : protocol === 'xai.video_jobs' ? { seconds: [5, 10], sizes: ['480p', '720p', '1080p'] }
    : protocol === 'siliconflow.video_jobs' ? { seconds: [], sizes: ['1280x720', '720x1280'] }
    : protocol === 'agnes.video_jobs' ? { seconds: [], sizes: ['1280x720', '720x1280', '720x720', '1920x1080', '1080x1920', '1080x1080'] }
    : { seconds: [], sizes: [] };
  return { qualities, video };
}

/** Present ratio and resolution separately while submitting the native pixel size. */
export function creationVideoSizeOptions(model: Model): ImageGenerationAspectRatioOption[] {
  const pixels = creationParameterPolicy(model).video.sizes.filter(value => /^\d+x\d+$/.test(value));
  if (!pixels.length) return [];
  return [
    { value: 'auto', label: '自动', width: null, height: null },
    ...sizes(pixels).map(option => ({ ...option, resolution: `${Math.min(option.width, option.height)}p` })),
  ];
}

export function creationMaxCount(mode: CreationMode, model: Model): number {
  return mode === 'image' ? Math.min(4, creationImageSizePolicy(model).maxCount) : 4;
}

export function creationCount(mode: CreationMode, value: unknown, model: Model): number {
  return Math.min(creationMaxCount(mode, model), Math.max(1, Math.floor(Number(value) || 1)));
}

/** Only composer-owned user parameters can be copied from history to a new task. */
export function creationUserParameters(mode: CreationMode, value: Record<string, unknown>): CreationParameters {
  const keys = mode === 'image' ? ['size', 'width', 'height', 'aspect', 'quality', 'count'] : mode === 'video' ? ['size', 'aspect', 'resolution', 'seconds', 'count'] : ['lyrics', 'instrumental', 'format'];
  return Object.fromEntries(keys.flatMap(key => typeof value[key] === 'string' || typeof value[key] === 'number' || typeof value[key] === 'boolean' || value[key] === null ? [[key, value[key]]] : [])) as CreationParameters;
}
export function normalizeCreationParameters(mode: CreationMode, value: CreationParameters, model: Model): CreationParameters {
  const params = creationUserParameters(mode, value);
  const policy = creationParameterPolicy(model);
  if (mode === 'image') {
    if (!policy.qualities.includes(String(params.quality))) delete params.quality;
    const image = creationImageSizePolicy(model);
    if (!image.options.some(size => (params.size !== undefined && size.requestSize === params.size) || (params.aspect !== undefined && size.value === params.aspect))) { delete params.size; delete params.width; delete params.height; delete params.aspect; }
    params.count = creationCount(mode, params.count, model);
  } else if (mode === 'video') {
    if (!policy.video.seconds.includes(Number(params.seconds))) delete params.seconds;
    if (!params.size) {
      const legacySizes: Record<string, string> = { '720p/16:9': '1280x720', '720p/9:16': '720x1280', '1080p/16:9': '1920x1080', '1080p/9:16': '1080x1920' };
      const legacySize = legacySizes[`${params.resolution}/${params.aspect}`];
      if (legacySize && policy.video.sizes.includes(legacySize)) params.size = legacySize;
    }
    if (!policy.video.sizes.includes(String(params.size))) delete params.size;
    delete params.aspect; delete params.resolution;
    params.count = creationCount(mode, params.count, model);
  }
  return params;
}
