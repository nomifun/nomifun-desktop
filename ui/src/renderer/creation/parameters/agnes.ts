import { getI18n } from 'react-i18next';
import type { ImageGenerationAspectRatioOption, ImageGenerationModelOption, ImageGenerationSizePolicy } from './image';

type Model = Pick<ImageGenerationModelOption, 'model' | 'protocol'> | null | undefined;

// Provider-native dimensions from wiki.agnes-ai.com (2026-10-08). Displayed
// pixels are translated at the Agnes adapter boundary into tiers and ratios.
const imageRatios = [
  ['1:1', 1024, 1024], ['3:4', 864, 1152], ['4:3', 1152, 864],
  ['16:9', 1312, 736], ['9:16', 736, 1312], ['2:3', 832, 1248],
  ['3:2', 1248, 832], ['21:9', 1568, 672],
] as const;
const videoRatios = [
  ['21:9', 1470, 630, 2206, 946], ['16:9', 1280, 720, 1920, 1080],
  ['4:3', 1112, 834, 1664, 1248], ['1:1', 960, 960, 1440, 1440],
  ['3:4', 834, 1112, 1248, 1664], ['9:16', 720, 1280, 1080, 1920],
] as const;
const flashVideoRatios = [
  ['21:9', 1680, 720], ['16:9', 1280, 704], ['4:3', 960, 720],
  ['1:1', 720, 720], ['3:4', 720, 960], ['9:16', 720, 1280],
] as const;

function option(ratio: string, resolution: string, width: number, height: number): ImageGenerationAspectRatioOption {
  const value = `${width}x${height}`;
  return { value, requestSize: value, label: `${ratio} · ${resolution}`, aspectRatio: ratio, resolution, width, height };
}
const automatic: ImageGenerationAspectRatioOption = {
  value: 'auto',
  get label() {
    return getI18n()?.t('creativeStudio.image.options.auto', { defaultValue: '自动' }) ?? '自动';
  },
  width: null,
  height: null,
};

export function agnesImageSizePolicy(model: Model): ImageGenerationSizePolicy | null {
  if (model?.protocol !== 'agnes.images') return null;
  if (model.model === 'agnes-image-2.1-flash' || model.model === 'agnes-image-2.5-flash') {
    return {
      options: [automatic, ...[1, 2, 3, 4].flatMap(tier => imageRatios.map(([ratio, width, height]) => option(ratio, `${tier}K`, width * tier, height * tier)))],
      maxCount: 1, allowCustomDimensions: false,
    };
  }
  return {
    options: [automatic, option('1:1', '1K', 1024, 1024),
      ...(model.model === 'agnes-image-2.0-flash' ? [option('4:3', '1K', 1024, 768), option('3:4', '1K', 768, 1024)] : [])],
    maxCount: 1, allowCustomDimensions: false,
  };
}

export function isCurrentAgnesVideo(model: Model): boolean {
  return model?.protocol === 'agnes.video_jobs' && ['agnes-video-2.5', 'agnes-video-2.5-flash'].includes(model.model);
}

export function isAgnesVideoV20(model: Model): boolean {
  return model?.protocol === 'agnes.video_jobs' && model.model === 'agnes-video-v2.0';
}

export function isAgnesVideo(model: Model): boolean {
  return isAgnesVideoV20(model) || isCurrentAgnesVideo(model);
}

export function agnesVideoSizeOptions(model: Model): ImageGenerationAspectRatioOption[] {
  if (isAgnesVideoV20(model)) {
    // Restore the established pixel choices; v2.0 sends width/height, not tiers.
    return [automatic,
      option('16:9', '720P', 1280, 720), option('9:16', '720P', 720, 1280), option('1:1', '720P', 720, 720),
      option('16:9', '1080P', 1920, 1080), option('9:16', '1080P', 1080, 1920), option('1:1', '1080P', 1080, 1080),
    ];
  }
  if (!isCurrentAgnesVideo(model)) return [];
  const ratios = model?.model === 'agnes-video-2.5-flash' ? flashVideoRatios : videoRatios;
  const options = ratios.map(([ratio, width, height]) => option(ratio, '720P', width, height));
  if (model?.model === 'agnes-video-2.5') {
    options.push(...videoRatios.map(([ratio, , , width, height]) => option(ratio, '1080P', width, height)),
      option('1:1', '1K', 1024, 1024),
      ...videoRatios.map(([ratio, width, height]) => option(ratio, '2K', width * 2, height * 2)));
  }
  return [automatic, ...options];
}

export function agnesVideoPolicy(model: Model): { seconds: number[]; sizes: string[] } {
  return { seconds: isAgnesVideoV20(model) ? [5, 10, 15] : isCurrentAgnesVideo(model) ? [4, 5, 6, 7, 8, 9, 10, 11, 12] : [],
    sizes: agnesVideoSizeOptions(model).flatMap(value => value.requestSize ? [value.requestSize] : []) };
}

/** Preserve the ratio of previously saved Flash sizes without advertising the
 * standard model's dimensions as Flash output. Other providers are unchanged. */
export function normalizeAgnesVideoSize(model: Model, size: string): string {
  if (model?.protocol !== 'agnes.video_jobs' || model.model !== 'agnes-video-2.5-flash') return size;
  const previous = videoRatios.find(([, width, height]) => size === `${width}x${height}`);
  const current = previous && flashVideoRatios.find(([ratio]) => ratio === previous[0]);
  return current ? `${current[1]}x${current[2]}` : size;
}

/** The canvas saves native resolution/ratio controls instead of pixel sizes. */
export function agnesCanvasVideoParameters(model: Model, resolution?: string, aspectRatio?: string): Record<string, string> {
  if (!isCurrentAgnesVideo(model)) return {};
  return { size: resolution?.toUpperCase() || '720P', aspect_ratio: aspectRatio || '16:9' };
}
