/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { translateCreativeImageTool } from "./imageToolI18n";

export interface CreativeImageDimensions {
  width: number;
  height: number;
}

/** Coordinates relative to the decoded source image, all in the [0, 1] range. */
export interface CreativeImageCropRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface CreativeImageCropPixels {
  x: number;
  y: number;
  width: number;
  height: number;
}

export type CreativeImageCropAspect = "free" | "1:1" | "4:3" | "16:9";

export type CreativeImageCropHandle =
  | "move"
  | "north-west"
  | "north"
  | "north-east"
  | "east"
  | "south-east"
  | "south"
  | "south-west"
  | "west";

export const CREATIVE_IMAGE_DEFAULT_CROP: CreativeImageCropRect = {
  x: 0.12,
  y: 0.12,
  width: 0.76,
  height: 0.76,
};

const MIN_CROP_SIZE = 0.04;

const clamp = (value: number, minimum: number, maximum: number): number =>
  Math.min(maximum, Math.max(minimum, value));

const finitePositive = (value: number, label: string): number => {
  if (!Number.isFinite(value) || value <= 0) {
    throw new Error(
      translateCreativeImageTool(
        "creativeStudio.canvas.imageTools.errors.finitePositive",
        { label },
      ),
    );
  }
  return value;
};

export function normalizeCreativeImageCrop(
  crop: CreativeImageCropRect,
): CreativeImageCropRect {
  const width = clamp(
    Number.isFinite(crop.width) ? crop.width : MIN_CROP_SIZE,
    MIN_CROP_SIZE,
    1,
  );
  const height = clamp(
    Number.isFinite(crop.height) ? crop.height : MIN_CROP_SIZE,
    MIN_CROP_SIZE,
    1,
  );
  return {
    x: clamp(Number.isFinite(crop.x) ? crop.x : 0, 0, 1 - width),
    y: clamp(Number.isFinite(crop.y) ? crop.y : 0, 0, 1 - height),
    width,
    height,
  };
}

export function creativeImageCropAspectRatio(
  aspect: CreativeImageCropAspect,
): number | null {
  switch (aspect) {
    case "free":
      return null;
    case "1:1":
      return 1;
    case "4:3":
      return 4 / 3;
    case "16:9":
      return 16 / 9;
  }
}

function normalizedAspectRatio(
  aspect: CreativeImageCropAspect,
  image: CreativeImageDimensions,
): number | null {
  const pixelAspect = creativeImageCropAspectRatio(aspect);
  if (pixelAspect === null) return null;
  return (
    pixelAspect *
    (finitePositive(
      image.height,
      translateCreativeImageTool(
        "creativeStudio.canvas.imageTools.dimensions.imageHeight",
      ),
    ) /
      finitePositive(
        image.width,
        translateCreativeImageTool(
          "creativeStudio.canvas.imageTools.dimensions.imageWidth",
        ),
      ))
  );
}

export function cropForCreativeImageAspect(
  image: CreativeImageDimensions,
  aspect: CreativeImageCropAspect,
  inset = 0.12,
): CreativeImageCropRect {
  const normalizedAspect = normalizedAspectRatio(aspect, image);
  if (normalizedAspect === null) {
    return normalizeCreativeImageCrop(CREATIVE_IMAGE_DEFAULT_CROP);
  }
  const available = clamp(1 - inset * 2, MIN_CROP_SIZE, 1);
  let width = available;
  let height = width / normalizedAspect;
  if (height > available) {
    height = available;
    width = height * normalizedAspect;
  }
  return normalizeCreativeImageCrop({
    x: (1 - width) / 2,
    y: (1 - height) / 2,
    width,
    height,
  });
}

export function moveCreativeImageCrop(
  crop: CreativeImageCropRect,
  delta: { x: number; y: number },
): CreativeImageCropRect {
  const normalized = normalizeCreativeImageCrop(crop);
  return {
    ...normalized,
    x: clamp(normalized.x + delta.x, 0, 1 - normalized.width),
    y: clamp(normalized.y + delta.y, 0, 1 - normalized.height),
  };
}

const handleDirections = (
  handle: Exclude<CreativeImageCropHandle, "move">,
): { horizontal: -1 | 0 | 1; vertical: -1 | 0 | 1 } => ({
  horizontal: handle.includes("west") ? -1 : handle.includes("east") ? 1 : 0,
  vertical: handle.includes("north") ? -1 : handle.includes("south") ? 1 : 0,
});

function resizeFreeCrop(
  crop: CreativeImageCropRect,
  handle: Exclude<CreativeImageCropHandle, "move">,
  delta: { x: number; y: number },
): CreativeImageCropRect {
  let left = crop.x;
  let right = crop.x + crop.width;
  let top = crop.y;
  let bottom = crop.y + crop.height;
  const direction = handleDirections(handle);
  if (direction.horizontal < 0) {
    left = clamp(left + delta.x, 0, right - MIN_CROP_SIZE);
  } else if (direction.horizontal > 0) {
    right = clamp(right + delta.x, left + MIN_CROP_SIZE, 1);
  }
  if (direction.vertical < 0) {
    top = clamp(top + delta.y, 0, bottom - MIN_CROP_SIZE);
  } else if (direction.vertical > 0) {
    bottom = clamp(bottom + delta.y, top + MIN_CROP_SIZE, 1);
  }
  return normalizeCreativeImageCrop({
    x: left,
    y: top,
    width: right - left,
    height: bottom - top,
  });
}

function cropFromCorner(
  crop: CreativeImageCropRect,
  direction: { horizontal: -1 | 1; vertical: -1 | 1 },
  delta: { x: number; y: number },
  normalizedAspect: number,
): CreativeImageCropRect {
  const anchorX = direction.horizontal > 0 ? crop.x : crop.x + crop.width;
  const anchorY = direction.vertical > 0 ? crop.y : crop.y + crop.height;
  const startX = direction.horizontal > 0 ? crop.x + crop.width : crop.x;
  const startY = direction.vertical > 0 ? crop.y + crop.height : crop.y;
  const targetX = clamp(startX + delta.x, 0, 1);
  const targetY = clamp(startY + delta.y, 0, 1);
  const maxWidth = direction.horizontal > 0 ? 1 - anchorX : anchorX;
  const maxHeight = direction.vertical > 0 ? 1 - anchorY : anchorY;
  const desiredWidth = Math.abs(targetX - anchorX);
  const desiredHeight = Math.abs(targetY - anchorY);

  const candidates = [
    {
      width: clamp(desiredWidth, MIN_CROP_SIZE, maxWidth),
      height: 0,
    },
    {
      width: 0,
      height: clamp(desiredHeight, MIN_CROP_SIZE, maxHeight),
    },
  ].map((candidate, index) => {
    const width =
      index === 0 ? candidate.width : candidate.height * normalizedAspect;
    const height =
      index === 0 ? candidate.width / normalizedAspect : candidate.height;
    return { width, height };
  });

  const valid = candidates.filter(
    (candidate) =>
      candidate.width >= MIN_CROP_SIZE &&
      candidate.height >= MIN_CROP_SIZE &&
      candidate.width <= maxWidth &&
      candidate.height <= maxHeight,
  );
  const fallbackHeight = Math.min(maxHeight, maxWidth / normalizedAspect);
  const fallback = {
    width: fallbackHeight * normalizedAspect,
    height: fallbackHeight,
  };
  const chosen = (valid.length > 0 ? valid : [fallback]).reduce(
    (best, candidate) => {
      const candidateX = anchorX + direction.horizontal * candidate.width;
      const candidateY = anchorY + direction.vertical * candidate.height;
      const bestX = anchorX + direction.horizontal * best.width;
      const bestY = anchorY + direction.vertical * best.height;
      const candidateDistance =
        (candidateX - targetX) ** 2 + (candidateY - targetY) ** 2;
      const bestDistance = (bestX - targetX) ** 2 + (bestY - targetY) ** 2;
      return candidateDistance < bestDistance ? candidate : best;
    },
  );

  return normalizeCreativeImageCrop({
    x: direction.horizontal > 0 ? anchorX : anchorX - chosen.width,
    y: direction.vertical > 0 ? anchorY : anchorY - chosen.height,
    width: chosen.width,
    height: chosen.height,
  });
}

function cropFromHorizontalEdge(
  crop: CreativeImageCropRect,
  direction: -1 | 1,
  deltaX: number,
  normalizedAspect: number,
): CreativeImageCropRect {
  const anchorX = direction > 0 ? crop.x : crop.x + crop.width;
  const startX = direction > 0 ? crop.x + crop.width : crop.x;
  const targetX = clamp(startX + deltaX, 0, 1);
  const centerY = crop.y + crop.height / 2;
  const maxWidthByX = direction > 0 ? 1 - anchorX : anchorX;
  const maxHeight = 2 * Math.min(centerY, 1 - centerY);
  const width = clamp(
    Math.min(Math.abs(targetX - anchorX), maxHeight * normalizedAspect),
    MIN_CROP_SIZE,
    maxWidthByX,
  );
  const height = width / normalizedAspect;
  return normalizeCreativeImageCrop({
    x: direction > 0 ? anchorX : anchorX - width,
    y: centerY - height / 2,
    width,
    height,
  });
}

function cropFromVerticalEdge(
  crop: CreativeImageCropRect,
  direction: -1 | 1,
  deltaY: number,
  normalizedAspect: number,
): CreativeImageCropRect {
  const anchorY = direction > 0 ? crop.y : crop.y + crop.height;
  const startY = direction > 0 ? crop.y + crop.height : crop.y;
  const targetY = clamp(startY + deltaY, 0, 1);
  const centerX = crop.x + crop.width / 2;
  const maxHeightByY = direction > 0 ? 1 - anchorY : anchorY;
  const maxWidth = 2 * Math.min(centerX, 1 - centerX);
  const height = clamp(
    Math.min(Math.abs(targetY - anchorY), maxWidth / normalizedAspect),
    MIN_CROP_SIZE,
    maxHeightByY,
  );
  const width = height * normalizedAspect;
  return normalizeCreativeImageCrop({
    x: centerX - width / 2,
    y: direction > 0 ? anchorY : anchorY - height,
    width,
    height,
  });
}

export function resizeCreativeImageCrop(
  crop: CreativeImageCropRect,
  handle: Exclude<CreativeImageCropHandle, "move">,
  delta: { x: number; y: number },
  image: CreativeImageDimensions,
  aspect: CreativeImageCropAspect,
): CreativeImageCropRect {
  const normalized = normalizeCreativeImageCrop(crop);
  const lockedAspect = normalizedAspectRatio(aspect, image);
  if (lockedAspect === null) return resizeFreeCrop(normalized, handle, delta);
  const direction = handleDirections(handle);
  if (direction.horizontal !== 0 && direction.vertical !== 0) {
    return cropFromCorner(
      normalized,
      {
        horizontal: direction.horizontal,
        vertical: direction.vertical,
      },
      delta,
      lockedAspect,
    );
  }
  if (direction.horizontal !== 0) {
    return cropFromHorizontalEdge(
      normalized,
      direction.horizontal,
      delta.x,
      lockedAspect,
    );
  }
  return cropFromVerticalEdge(
    normalized,
    direction.vertical || 1,
    delta.y,
    lockedAspect,
  );
}

export function creativeImageCropToPixels(
  crop: CreativeImageCropRect,
  image: CreativeImageDimensions,
): CreativeImageCropPixels {
  const width = Math.round(
    finitePositive(
      image.width,
      translateCreativeImageTool(
        "creativeStudio.canvas.imageTools.dimensions.imageWidth",
      ),
    ),
  );
  const height = Math.round(
    finitePositive(
      image.height,
      translateCreativeImageTool(
        "creativeStudio.canvas.imageTools.dimensions.imageHeight",
      ),
    ),
  );
  const normalized = normalizeCreativeImageCrop(crop);
  const x = clamp(Math.round(normalized.x * width), 0, width - 1);
  const y = clamp(Math.round(normalized.y * height), 0, height - 1);
  return {
    x,
    y,
    width: clamp(Math.round(normalized.width * width), 1, width - x),
    height: clamp(Math.round(normalized.height * height), 1, height - y),
  };
}
