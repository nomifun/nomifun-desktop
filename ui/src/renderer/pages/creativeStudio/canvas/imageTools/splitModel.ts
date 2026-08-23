/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type {
  CreativeImageCropPixels,
  CreativeImageDimensions,
} from "./cropModel";
import { translateCreativeImageTool } from "./imageToolI18n";

export const CREATIVE_IMAGE_SPLIT_MAX_GRID = 12;
export const CREATIVE_IMAGE_SPLIT_MIN_GAP = 0.01;

export type CreativeImageSplitAxis = "horizontal" | "vertical";

export interface CreativeImageSplitParams {
  horizontalLines: readonly number[];
  verticalLines: readonly number[];
}

export interface CreativeImageSplitPiece {
  row: number;
  column: number;
  crop: CreativeImageCropPixels;
}

export const CREATIVE_IMAGE_DEFAULT_SPLIT: CreativeImageSplitParams = {
  horizontalLines: [0.5],
  verticalLines: [0.5],
};

const clamp = (value: number, minimum: number, maximum: number): number =>
  Math.min(maximum, Math.max(minimum, value));

export const creativeImageSplitRows = (
  params: CreativeImageSplitParams,
): number => params.horizontalLines.length + 1;

export const creativeImageSplitColumns = (
  params: CreativeImageSplitParams,
): number => params.verticalLines.length + 1;

export const creativeImageSplitTotal = (
  params: CreativeImageSplitParams,
): number => creativeImageSplitRows(params) * creativeImageSplitColumns(params);

export function buildCreativeImageSplitLines(count: number): number[] {
  const normalized = clamp(
    Math.round(Number.isFinite(count) ? count : 1),
    1,
    CREATIVE_IMAGE_SPLIT_MAX_GRID,
  );
  return Array.from(
    { length: normalized - 1 },
    (_, index) => (index + 1) / normalized,
  );
}

const linesForAxis = (
  params: CreativeImageSplitParams,
  axis: CreativeImageSplitAxis,
): readonly number[] =>
  axis === "horizontal" ? params.horizontalLines : params.verticalLines;

const withAxisLines = (
  params: CreativeImageSplitParams,
  axis: CreativeImageSplitAxis,
  lines: readonly number[],
): CreativeImageSplitParams =>
  axis === "horizontal"
    ? { ...params, horizontalLines: [...lines] }
    : { ...params, verticalLines: [...lines] };

export function setCreativeImageSplitCount(
  params: CreativeImageSplitParams,
  axis: CreativeImageSplitAxis,
  count: number,
): CreativeImageSplitParams {
  return withAxisLines(params, axis, buildCreativeImageSplitLines(count));
}

export function addCreativeImageSplitLine(
  params: CreativeImageSplitParams,
  axis: CreativeImageSplitAxis,
): CreativeImageSplitParams {
  const lines = [...linesForAxis(params, axis)].sort(
    (left, right) => left - right,
  );
  if (lines.length >= CREATIVE_IMAGE_SPLIT_MAX_GRID - 1) return params;
  const cuts = [0, ...lines, 1];
  let largestGap = 0;
  let position = 0.5;
  for (let index = 0; index < cuts.length - 1; index += 1) {
    const gap = cuts[index + 1] - cuts[index];
    if (gap > largestGap) {
      largestGap = gap;
      position = cuts[index] + gap / 2;
    }
  }
  return withAxisLines(
    params,
    axis,
    [...lines, position].sort((left, right) => left - right),
  );
}

export function removeCreativeImageSplitLine(
  params: CreativeImageSplitParams,
  axis: CreativeImageSplitAxis,
  index: number,
): CreativeImageSplitParams {
  const lines = linesForAxis(params, axis);
  if (!Number.isInteger(index) || index < 0 || index >= lines.length)
    return params;
  return withAxisLines(
    params,
    axis,
    lines.filter((_, lineIndex) => lineIndex !== index),
  );
}

export function moveCreativeImageSplitLine(
  params: CreativeImageSplitParams,
  axis: CreativeImageSplitAxis,
  index: number,
  value: number,
): CreativeImageSplitParams {
  const lines = [...linesForAxis(params, axis)];
  if (!Number.isInteger(index) || index < 0 || index >= lines.length)
    return params;
  const minimum = (lines[index - 1] ?? 0) + CREATIVE_IMAGE_SPLIT_MIN_GAP;
  const maximum = (lines[index + 1] ?? 1) - CREATIVE_IMAGE_SPLIT_MIN_GAP;
  lines[index] = clamp(
    Number.isFinite(value) ? value : lines[index],
    minimum,
    maximum,
  );
  return withAxisLines(params, axis, lines);
}

export function resetCreativeImageSplitLines(
  params: CreativeImageSplitParams,
): CreativeImageSplitParams {
  return {
    horizontalLines: buildCreativeImageSplitLines(
      creativeImageSplitRows(params),
    ),
    verticalLines: buildCreativeImageSplitLines(
      creativeImageSplitColumns(params),
    ),
  };
}

const pixelBoundaries = (
  size: number,
  lines: readonly number[],
  label: string,
): number[] => {
  if (!Number.isInteger(size) || size <= 0) {
    throw new Error(
      translateCreativeImageTool(
        "creativeStudio.canvas.imageTools.errors.positiveInteger",
        { label },
      ),
    );
  }
  const boundaries = [
    0,
    ...lines.map((line) => Math.round(clamp(line, 0, 1) * size)),
    size,
  ];
  for (let index = 1; index < boundaries.length; index += 1) {
    if (boundaries[index] <= boundaries[index - 1]) {
      throw new Error(
        translateCreativeImageTool(
          "creativeStudio.canvas.imageTools.errors.sliceDimensionTooSmall",
          { label },
        ),
      );
    }
  }
  return boundaries;
};

/** Project normalized grid lines to a gapless, row-major decoded-pixel partition. */
export function creativeImageSplitPieces(
  params: CreativeImageSplitParams,
  dimensions: CreativeImageDimensions,
): CreativeImageSplitPiece[] {
  const horizontal = pixelBoundaries(
    Math.round(dimensions.height),
    params.horizontalLines,
    translateCreativeImageTool(
      "creativeStudio.canvas.imageTools.dimensions.imageHeight",
    ),
  );
  const vertical = pixelBoundaries(
    Math.round(dimensions.width),
    params.verticalLines,
    translateCreativeImageTool(
      "creativeStudio.canvas.imageTools.dimensions.imageWidth",
    ),
  );
  const pieces: CreativeImageSplitPiece[] = [];
  for (let row = 0; row < horizontal.length - 1; row += 1) {
    for (let column = 0; column < vertical.length - 1; column += 1) {
      pieces.push({
        row,
        column,
        crop: {
          x: vertical[column],
          y: horizontal[row],
          width: vertical[column + 1] - vertical[column],
          height: horizontal[row + 1] - horizontal[row],
        },
      });
    }
  }
  return pieces;
}
