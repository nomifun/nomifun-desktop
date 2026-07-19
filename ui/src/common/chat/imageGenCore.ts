/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

/**
 * Shared image generation logic used by both:
 * - The built-in MCP server (imageGenServer.ts)
 * - The legacy Gemini-specific tool (img-gen.ts)
 */

import * as fs from 'fs';
import * as path from 'path';
import { randomUUID } from 'crypto';
import { inflateSync } from 'zlib';
import { loadImage } from '@napi-rs/canvas';
import { jsonrepair } from 'jsonrepair';
import type OpenAI from 'openai';
import { ClientFactory, type RotatingClient } from '@/common/api/ClientFactory';
import type { TProviderWithModel } from '@/common/config/storage';
import type { UnifiedChatCompletionResponse } from '@/common/api/RotatingApiClient';
import { IMAGE_EXTENSIONS, MIME_TYPE_MAP, MIME_TO_EXT_MAP, DEFAULT_IMAGE_EXTENSION } from '@/common/config/constants';

const API_TIMEOUT_MS = 120000; // 2 minutes for image generation API calls
const IMAGE_DOWNLOAD_TIMEOUT_MS = 30000;
export const MAX_GENERATED_IMAGE_BYTES = 50 * 1024 * 1024;
export const MAX_GENERATED_IMAGE_COUNT = 16;
export const MAX_TOTAL_GENERATED_IMAGE_BYTES = 100 * 1024 * 1024;
export const MAX_GENERATED_IMAGE_DIMENSION = 16_384;
export const MAX_GENERATED_IMAGE_PIXELS = 64 * 1024 * 1024;

type ImageExtension = (typeof IMAGE_EXTENSIONS)[number];

// ===== Utility Functions =====

export function safeJsonParse<T = unknown>(jsonString: string, fallbackValue: T): T {
  if (!jsonString || typeof jsonString !== 'string') {
    return fallbackValue;
  }

  try {
    return JSON.parse(jsonString) as T;
  } catch (_error) {
    try {
      const repairedJson = jsonrepair(jsonString);
      return JSON.parse(repairedJson) as T;
    } catch (_repairError) {
      console.warn('[ImageGen] JSON parse failed:', jsonString.substring(0, 50));
      return fallbackValue;
    }
  }
}

export function isImageFile(file_path: string): boolean {
  const ext = path.extname(file_path).toLowerCase();
  return IMAGE_EXTENSIONS.includes(ext as ImageExtension);
}

export function isHttpUrl(str: string): boolean {
  try {
    const protocol = new URL(str.trim()).protocol.toLowerCase();
    return protocol === 'http:' || protocol === 'https:';
  } catch (_error) {
    return false;
  }
}

export async function fileToBase64(file_path: string): Promise<string> {
  try {
    const metadata = await fs.promises.stat(file_path);
    if (!metadata.isFile()) {
      throw new Error(`Image path is not a regular file: ${file_path}`);
    }
    if (metadata.size === 0) {
      throw new Error(`Image file is empty: ${file_path}`);
    }
    if (metadata.size > MAX_GENERATED_IMAGE_BYTES) {
      throw new Error(`Image file exceeds the ${MAX_GENERATED_IMAGE_BYTES}-byte size limit`);
    }
    const fileBuffer = await fs.promises.readFile(file_path);
    if (fileBuffer.length !== metadata.size) {
      throw new Error(`Image file changed while it was being read: ${file_path}`);
    }
    return fileBuffer.toString('base64');
  } catch (error) {
    const errorMessage = error instanceof Error ? error.message : String(error);
    if (errorMessage.includes('ENOENT') || errorMessage.includes('no such file')) {
      throw new Error(`Image file not found: ${file_path}`, { cause: error });
    }
    throw new Error(`Failed to read image file: ${errorMessage}`, { cause: error });
  }
}

/**
 * Resolve a model-authored generated-image path without allowing it to adopt
 * an older/arbitrary file outside the active workspace. `realpath` closes both
 * lexical `..` escapes and workspace symlink escapes on macOS, Linux and
 * Windows; `path.relative` avoids case/separator assumptions.
 */
export async function resolveWorkspaceGeneratedImagePath(
  referencedPath: string,
  workspaceDir: string
): Promise<string> {
  const workspace = await fs.promises.realpath(workspaceDir);
  const candidate = path.isAbsolute(referencedPath)
    ? referencedPath
    : path.resolve(workspace, referencedPath);
  const canonicalCandidate = await fs.promises.realpath(candidate);
  const relative = path.relative(workspace, canonicalCandidate);
  if (relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) {
    throw new Error(`Generated image path is outside the active workspace: ${referencedPath}`);
  }

  const metadata = await fs.promises.stat(canonicalCandidate);
  if (!metadata.isFile()) {
    throw new Error(`Generated image path is not a regular file: ${referencedPath}`);
  }
  return canonicalCandidate;
}

export function getImageMimeType(file_path: string): string {
  const ext = path.extname(file_path).toLowerCase();
  return MIME_TYPE_MAP[ext] || MIME_TYPE_MAP[DEFAULT_IMAGE_EXTENSION];
}

export function getFileExtensionFromDataUrl(dataUrl: string): string {
  const mimeTypeMatch = dataUrl.match(/^data:image\/([^;]+);base64,/);
  if (mimeTypeMatch && mimeTypeMatch[1]) {
    const mimeType = mimeTypeMatch[1].toLowerCase();
    return MIME_TO_EXT_MAP[mimeType] || DEFAULT_IMAGE_EXTENSION;
  }
  return DEFAULT_IMAGE_EXTENSION;
}

interface ValidatedImage {
  bytes: Buffer;
  mimeType: string;
  extension: ImageExtension;
}

interface ImageDimensions {
  width: number;
  height: number;
}

const MIME_ALIASES: Record<string, string> = {
  'image/jpg': 'image/jpeg',
  'image/pjpeg': 'image/jpeg',
  'image/x-png': 'image/png',
  'image/x-ms-bmp': 'image/bmp',
  'image/x-tiff': 'image/tiff',
};

const PNG_CRC32_TABLE = Uint32Array.from({ length: 256 }, (_, value) => {
  let crc = value;
  for (let bit = 0; bit < 8; bit += 1) {
    crc = (crc & 1) !== 0 ? 0xedb88320 ^ (crc >>> 1) : crc >>> 1;
  }
  return crc >>> 0;
});

function pngCrc32(bytes: Buffer): number {
  let crc = 0xffffffff;
  for (const value of bytes) {
    crc = PNG_CRC32_TABLE[(crc ^ value) & 0xff] ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function normalizeMimeType(mimeType: string | null | undefined): string | undefined {
  const normalized = mimeType?.split(';', 1)[0]?.trim().toLowerCase();
  if (!normalized || normalized === 'application/octet-stream') return undefined;
  return MIME_ALIASES[normalized] || normalized;
}

function startsWithBytes(bytes: Buffer, signature: readonly number[]): boolean {
  return signature.every((value, index) => bytes[index] === value);
}

function throwIfGenerationAborted(signal?: AbortSignal): void {
  if (signal?.aborted) {
    throw new Error('Image generation was cancelled', { cause: signal.reason });
  }
}

function validPng(bytes: Buffer): boolean {
  if (!startsWithBytes(bytes, [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])) return false;
  let offset = 8;
  let chunkIndex = 0;
  let width = 0;
  let height = 0;
  let bitsPerPixel = 0;
  let interlaceMethod = 0;
  let sawImageData = false;
  let imageDataEnded = false;
  const imageDataChunks: Buffer[] = [];
  while (offset + 12 <= bytes.length) {
    const length = bytes.readUInt32BE(offset);
    const end = offset + 12 + length;
    if (!Number.isSafeInteger(end) || end > bytes.length) return false;
    const type = bytes.subarray(offset + 4, offset + 8).toString('ascii');
    const chunkBytes = bytes.subarray(offset + 4, offset + 8 + length);
    if (pngCrc32(chunkBytes) !== bytes.readUInt32BE(offset + 8 + length)) return false;
    if (chunkIndex === 0) {
      if (type !== 'IHDR' || length !== 13) return false;
      width = bytes.readUInt32BE(offset + 8);
      height = bytes.readUInt32BE(offset + 12);
      const bitDepth = bytes[offset + 16];
      const colorType = bytes[offset + 17];
      interlaceMethod = bytes[offset + 20];
      const channels = new Map<number, number>([[0, 1], [2, 3], [3, 1], [4, 2], [6, 4]]).get(colorType);
      const allowedBitDepths = new Map<number, readonly number[]>([
        [0, [1, 2, 4, 8, 16]],
        [2, [8, 16]],
        [3, [1, 2, 4, 8]],
        [4, [8, 16]],
        [6, [8, 16]],
      ]).get(colorType);
      if (
        width === 0 ||
        height === 0 ||
        width > MAX_GENERATED_IMAGE_DIMENSION ||
        height > MAX_GENERATED_IMAGE_DIMENSION ||
        width * height > MAX_GENERATED_IMAGE_PIXELS ||
        channels === undefined ||
        !allowedBitDepths?.includes(bitDepth) ||
        bytes[offset + 18] !== 0 ||
        bytes[offset + 19] !== 0 ||
        (interlaceMethod !== 0 && interlaceMethod !== 1)
      ) {
        return false;
      }
      bitsPerPixel = channels * bitDepth;
    } else if (type === 'IHDR') {
      return false;
    }
    if (type === 'IDAT') {
      if (length === 0 || imageDataEnded) return false;
      sawImageData = true;
      imageDataChunks.push(bytes.subarray(offset + 8, offset + 8 + length));
    } else if (sawImageData) {
      imageDataEnded = true;
    }
    if (type === 'IEND') {
      if (length !== 0 || !sawImageData || end !== bytes.length) return false;

      const rowBytes = (passWidth: number): number => Math.ceil((passWidth * bitsPerPixel) / 8);
      let expectedInflatedBytes = 0;
      const scanlineLayouts: Array<{ rows: number; rowBytes: number }> = [];
      if (interlaceMethod === 0) {
        const bytesPerRow = rowBytes(width);
        scanlineLayouts.push({ rows: height, rowBytes: bytesPerRow });
        expectedInflatedBytes = height * (1 + bytesPerRow);
      } else {
        const startsX = [0, 4, 0, 2, 0, 1, 0];
        const startsY = [0, 0, 4, 0, 2, 0, 1];
        const stepsX = [8, 8, 4, 4, 2, 2, 1];
        const stepsY = [8, 8, 8, 4, 4, 2, 2];
        for (let pass = 0; pass < 7; pass += 1) {
          const passWidth = width <= startsX[pass] ? 0 : Math.ceil((width - startsX[pass]) / stepsX[pass]);
          const passHeight = height <= startsY[pass] ? 0 : Math.ceil((height - startsY[pass]) / stepsY[pass]);
          if (passWidth > 0 && passHeight > 0) {
            const bytesPerRow = rowBytes(passWidth);
            scanlineLayouts.push({ rows: passHeight, rowBytes: bytesPerRow });
            expectedInflatedBytes += passHeight * (1 + bytesPerRow);
          }
        }
      }
      if (!Number.isSafeInteger(expectedInflatedBytes) || expectedInflatedBytes <= 0 || expectedInflatedBytes > 256 * 1024 * 1024) {
        return false;
      }
      try {
        const inflated = inflateSync(Buffer.concat(imageDataChunks), { maxOutputLength: expectedInflatedBytes });
        if (inflated.length !== expectedInflatedBytes) return false;
        let cursor = 0;
        for (const layout of scanlineLayouts) {
          for (let row = 0; row < layout.rows; row += 1) {
            // PNG defines only filter methods 0..4. Native decoders can be
            // deliberately lenient here, so validate this independently.
            if (inflated[cursor] > 4) return false;
            cursor += 1 + layout.rowBytes;
          }
        }
        return cursor === inflated.length;
      } catch {
        return false;
      }
    }
    offset = end;
    chunkIndex += 1;
  }
  return false;
}

function validJpeg(bytes: Buffer): boolean {
  if (!startsWithBytes(bytes, [0xff, 0xd8, 0xff]) || bytes.length < 16) return false;
  if (bytes[bytes.length - 2] !== 0xff || bytes[bytes.length - 1] !== 0xd9) return false;
  let offset = 2;
  let hasFrame = false;
  let hasScan = false;
  while (offset + 1 < bytes.length - 2) {
    if (bytes[offset] !== 0xff) return hasFrame && hasScan;
    while (bytes[offset] === 0xff) offset += 1;
    const marker = bytes[offset++];
    if (marker === 0xda) {
      hasScan = true;
      break;
    }
    if (marker === 0xd9) break;
    if (marker === 0x01 || (marker >= 0xd0 && marker <= 0xd7)) continue;
    if (offset + 2 > bytes.length) return false;
    const length = bytes.readUInt16BE(offset);
    if (length < 2 || offset + length > bytes.length) return false;
    hasFrame ||= (marker >= 0xc0 && marker <= 0xc3) || (marker >= 0xc5 && marker <= 0xc7) ||
      (marker >= 0xc9 && marker <= 0xcb) || (marker >= 0xcd && marker <= 0xcf);
    offset += length;
  }
  return hasFrame && hasScan;
}

function validWebp(bytes: Buffer): boolean {
  return bytes.length >= 20 && bytes.subarray(0, 4).toString('ascii') === 'RIFF' &&
    bytes.subarray(8, 12).toString('ascii') === 'WEBP' && bytes.readUInt32LE(4) + 8 === bytes.length &&
    ['VP8 ', 'VP8L', 'VP8X'].includes(bytes.subarray(12, 16).toString('ascii'));
}

function jpegDimensions(bytes: Buffer): ImageDimensions | null {
  let offset = 2;
  while (offset + 3 < bytes.length - 2) {
    if (bytes[offset] !== 0xff) return null;
    while (offset < bytes.length && bytes[offset] === 0xff) offset += 1;
    const marker = bytes[offset++];
    if (marker === 0xda || marker === 0xd9 || marker === undefined) return null;
    if (marker === 0x01 || (marker >= 0xd0 && marker <= 0xd7)) continue;
    if (offset + 2 > bytes.length) return null;
    const length = bytes.readUInt16BE(offset);
    if (length < 2 || offset + length > bytes.length) return null;
    const isStartOfFrame =
      (marker >= 0xc0 && marker <= 0xc3) ||
      (marker >= 0xc5 && marker <= 0xc7) ||
      (marker >= 0xc9 && marker <= 0xcb) ||
      (marker >= 0xcd && marker <= 0xcf);
    if (isStartOfFrame) {
      if (length < 7) return null;
      return {
        height: bytes.readUInt16BE(offset + 3),
        width: bytes.readUInt16BE(offset + 5),
      };
    }
    offset += length;
  }
  return null;
}

function webpDimensions(bytes: Buffer): ImageDimensions | null {
  const chunkType = bytes.subarray(12, 16).toString('ascii');
  if (chunkType === 'VP8X') {
    if (bytes.length < 30 || bytes.readUInt32LE(16) < 10) return null;
    return {
      width: bytes.readUIntLE(24, 3) + 1,
      height: bytes.readUIntLE(27, 3) + 1,
    };
  }
  if (chunkType === 'VP8L') {
    if (bytes.length < 25 || bytes[20] !== 0x2f) return null;
    return {
      width: 1 + bytes[21] + ((bytes[22] & 0x3f) << 8),
      height: 1 + ((bytes[22] & 0xc0) >> 6) + (bytes[23] << 2) + ((bytes[24] & 0x0f) << 10),
    };
  }
  if (chunkType === 'VP8 ') {
    if (
      bytes.length < 30 ||
      bytes[23] !== 0x9d ||
      bytes[24] !== 0x01 ||
      bytes[25] !== 0x2a
    ) {
      return null;
    }
    return {
      width: bytes.readUInt16LE(26) & 0x3fff,
      height: bytes.readUInt16LE(28) & 0x3fff,
    };
  }
  return null;
}

function encodedImageDimensions(bytes: Buffer, mimeType: string): ImageDimensions | null {
  switch (mimeType) {
    case 'image/png':
      return bytes.length >= 24
        ? { width: bytes.readUInt32BE(16), height: bytes.readUInt32BE(20) }
        : null;
    case 'image/jpeg':
      return jpegDimensions(bytes);
    case 'image/webp':
      return webpDimensions(bytes);
    default:
      return null;
  }
}

function detectImageFormat(bytes: Buffer): Omit<ValidatedImage, 'bytes'> | null {
  if (validPng(bytes)) {
    return { mimeType: 'image/png', extension: '.png' };
  }
  if (validJpeg(bytes)) {
    return { mimeType: 'image/jpeg', extension: '.jpg' };
  }
  if (validWebp(bytes)) {
    return { mimeType: 'image/webp', extension: '.webp' };
  }

  return null;
}

async function validateImageBytes(bytes: Buffer, declaredMimeType?: string): Promise<ValidatedImage> {
  if (bytes.length === 0) {
    throw new Error('Generated image is empty');
  }
  if (bytes.length > MAX_GENERATED_IMAGE_BYTES) {
    throw new Error(
      `Generated image exceeds the ${MAX_GENERATED_IMAGE_BYTES}-byte size limit (received ${bytes.length} bytes)`
    );
  }

  const detected = detectImageFormat(bytes);
  if (!detected) {
    throw new Error('Generated payload does not have a supported image signature');
  }

  const normalizedDeclaredMimeType = normalizeMimeType(declaredMimeType);
  if (normalizedDeclaredMimeType && normalizedDeclaredMimeType !== detected.mimeType) {
    throw new Error(
      `Generated image MIME mismatch: response declared ${normalizedDeclaredMimeType}, bytes are ${detected.mimeType}`
    );
  }

  const encodedDimensions = encodedImageDimensions(bytes, detected.mimeType);
  if (
    !encodedDimensions ||
    encodedDimensions.width <= 0 ||
    encodedDimensions.height <= 0 ||
    encodedDimensions.width > MAX_GENERATED_IMAGE_DIMENSION ||
    encodedDimensions.height > MAX_GENERATED_IMAGE_DIMENSION ||
    encodedDimensions.width * encodedDimensions.height > MAX_GENERATED_IMAGE_PIXELS
  ) {
    throw new Error('Generated image has invalid or unsafe dimensions');
  }

  // Container signatures alone are not proof that pixels can be rendered. A
  // truncated/corrupt PNG, JPEG or WebP can otherwise be persisted and
  // reported as successful even though every UI decoder rejects it. The native
  // decoder has binaries for the supported macOS/Linux/Windows architectures.
  let decodedImage;
  try {
    decodedImage = await loadImage(bytes);
  } catch (error) {
    throw new Error('Generated image pixel data cannot be decoded', { cause: error });
  }
  const exactDimensions =
    decodedImage.width === encodedDimensions.width && decodedImage.height === encodedDimensions.height;
  const exifOrientedJpegDimensions =
    detected.mimeType === 'image/jpeg' &&
    decodedImage.width === encodedDimensions.height &&
    decodedImage.height === encodedDimensions.width;
  if (!exactDimensions && !exifOrientedJpegDimensions) {
    throw new Error('Generated image dimensions changed during pixel decoding');
  }

  return { bytes, ...detected };
}

function decodeStrictBase64(base64Data: string): Buffer {
  const compact = base64Data.replace(/\s+/g, '');
  if (!compact) {
    throw new Error('Generated image Base64 payload is empty');
  }
  if (!/^[A-Za-z0-9+/]*={0,2}$/.test(compact)) {
    throw new Error('Generated image contains invalid Base64 characters or padding');
  }

  const withoutPadding = compact.replace(/=+$/, '');
  if (withoutPadding.length % 4 === 1 || (compact.includes('=') && compact.length % 4 !== 0)) {
    throw new Error('Generated image contains invalid Base64 padding');
  }
  if (Math.ceil((withoutPadding.length * 3) / 4) > MAX_GENERATED_IMAGE_BYTES) {
    throw new Error(`Generated image exceeds the ${MAX_GENERATED_IMAGE_BYTES}-byte size limit`);
  }

  const padded = withoutPadding.padEnd(Math.ceil(withoutPadding.length / 4) * 4, '=');
  const bytes = Buffer.from(padded, 'base64');
  if (bytes.toString('base64').replace(/=+$/, '') !== withoutPadding) {
    throw new Error('Generated image Base64 payload is malformed');
  }
  return bytes;
}

async function decodeDataUrl(dataUrl: string): Promise<ValidatedImage> {
  const match = /^data:([^;,]+)((?:;[^,]*)*?),(.*)$/is.exec(dataUrl);
  if (!match) {
    throw new Error('Generated image data URL is malformed');
  }
  if (!/(?:^|;)base64(?:;|$)/i.test(match[2])) {
    throw new Error('Generated image data URL must use Base64 encoding');
  }

  const declaredMimeType = normalizeMimeType(match[1]);
  if (!declaredMimeType?.startsWith('image/')) {
    throw new Error(`Generated data URL is not an image (${match[1]})`);
  }
  return validateImageBytes(decodeStrictBase64(match[3]), declaredMimeType);
}

async function readResponseBody(response: Response): Promise<Buffer> {
  const contentLengthValue = response.headers.get('content-length');
  if (contentLengthValue !== null) {
    const contentLength = Number(contentLengthValue);
    if (!Number.isSafeInteger(contentLength) || contentLength < 0) {
      throw new Error(`Generated image response has an invalid Content-Length: ${contentLengthValue}`);
    }
    if (contentLength === 0) {
      throw new Error('Generated image response is empty');
    }
    if (contentLength > MAX_GENERATED_IMAGE_BYTES) {
      throw new Error(`Generated image exceeds the ${MAX_GENERATED_IMAGE_BYTES}-byte size limit`);
    }
  }

  if (!response.body) {
    return Buffer.from(await response.arrayBuffer());
  }

  const reader = response.body.getReader();
  const chunks: Buffer[] = [];
  let totalBytes = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      if (!value || value.byteLength === 0) continue;
      totalBytes += value.byteLength;
      if (totalBytes > MAX_GENERATED_IMAGE_BYTES) {
        await reader.cancel('generated image exceeded size limit');
        throw new Error(`Generated image exceeds the ${MAX_GENERATED_IMAGE_BYTES}-byte size limit`);
      }
      chunks.push(Buffer.from(value));
    }
  } finally {
    reader.releaseLock();
  }
  return Buffer.concat(chunks, totalBytes);
}

async function downloadImage(imageUrl: string, signal?: AbortSignal): Promise<ValidatedImage> {
  throwIfGenerationAborted(signal);
  const parsedUrl = new URL(imageUrl);
  if (parsedUrl.protocol !== 'http:' && parsedUrl.protocol !== 'https:') {
    throw new Error(`Unsupported generated image URL protocol: ${parsedUrl.protocol}`);
  }

  const controller = new AbortController();
  const forwardAbort = () => controller.abort(signal?.reason);
  if (signal?.aborted) forwardAbort();
  signal?.addEventListener('abort', forwardAbort, { once: true });
  const timeout = setTimeout(
    () => controller.abort(new Error(`Generated image download timed out after ${IMAGE_DOWNLOAD_TIMEOUT_MS}ms`)),
    IMAGE_DOWNLOAD_TIMEOUT_MS
  );

  try {
    const response = await fetch(parsedUrl, { signal: controller.signal });
    if (!response.ok) {
      throw new Error(`Generated image download failed with HTTP ${response.status}`);
    }

    const declaredMimeType = normalizeMimeType(response.headers.get('content-type'));
    if (declaredMimeType && !declaredMimeType.startsWith('image/')) {
      throw new Error(`Generated image URL returned non-image Content-Type: ${declaredMimeType}`);
    }
    const bytes = await readResponseBody(response);
    throwIfGenerationAborted(signal);
    const validated = await validateImageBytes(bytes, declaredMimeType);
    throwIfGenerationAborted(signal);
    return validated;
  } finally {
    clearTimeout(timeout);
    signal?.removeEventListener('abort', forwardAbort);
  }
}

async function resolveGeneratedImage(imageSource: string, signal?: AbortSignal): Promise<ValidatedImage> {
  throwIfGenerationAborted(signal);
  const source = imageSource.trim();
  if (!source) {
    throw new Error('Generated image source is empty');
  }
  if (/^data:/i.test(source)) {
    const image = await decodeDataUrl(source);
    throwIfGenerationAborted(signal);
    return image;
  }
  if (isHttpUrl(source)) {
    return downloadImage(source, signal);
  }
  if (/^[a-z][a-z\d+.-]*:/i.test(source)) {
    throw new Error('Generated image source must be a data URL, Base64 payload, or HTTP(S) URL');
  }
  const image = await validateImageBytes(decodeStrictBase64(source));
  throwIfGenerationAborted(signal);
  return image;
}

async function persistValidatedImage(
  image: ValidatedImage,
  workspaceDir: string,
  signal?: AbortSignal
): Promise<string> {
  throwIfGenerationAborted(signal);
  if (!workspaceDir.trim()) {
    throw new Error('Workspace directory is required to save generated images');
  }

  const requestedWorkspace = path.resolve(workspaceDir);
  await fs.promises.mkdir(requestedWorkspace, { recursive: true });
  throwIfGenerationAborted(signal);
  const workspace = await fs.promises.realpath(requestedWorkspace);
  const workspaceStat = await fs.promises.stat(workspace);
  if (!workspaceStat.isDirectory()) {
    throw new Error(`Workspace path is not a directory: ${workspace}`);
  }

  const fileName = `img-${Date.now()}-${randomUUID()}${image.extension}`;
  const filePath = path.join(workspace, fileName);
  const tempPath = path.join(workspace, `.${fileName}.${randomUUID()}.tmp`);
  let destinationOwned = false;

  const syncWorkspaceDirectory = async (): Promise<void> => {
    // POSIX requires syncing the parent directory for link/unlink metadata to
    // be crash-durable. Windows does not expose a portable directory fsync in
    // Node; the production Rust artifact store performs its own durable write.
    if (process.platform === 'win32') return;

    const directoryHandle = await fs.promises.open(workspace, 'r');
    try {
      await directoryHandle.sync();
    } finally {
      await directoryHandle.close();
    }
  };

  try {
    throwIfGenerationAborted(signal);
    const handle = await fs.promises.open(tempPath, 'wx', 0o600);
    try {
      await handle.writeFile(image.bytes);
      await handle.sync();
    } finally {
      await handle.close();
    }
    throwIfGenerationAborted(signal);

    // A hard link gives atomic, no-replace publication. FAT/exFAT, some SMB/NFS
    // shares, and locked-down Windows volumes can reject hard links, so fall
    // back to an exclusive destination handle. The random destination is not
    // exposed until its bytes are synced and re-verified; `wx` preserves the
    // same never-overwrite contract. EEXIST must never enter the fallback.
    try {
      await fs.promises.link(tempPath, filePath);
      destinationOwned = true;
    } catch (error) {
      const code = (error as NodeJS.ErrnoException).code;
      const hardLinkUnsupported = new Set(['EPERM', 'ENOTSUP', 'EOPNOTSUPP', 'EXDEV', 'EINVAL']);
      if (!code || !hardLinkUnsupported.has(code)) throw error;

      const destinationHandle = await fs.promises.open(filePath, 'wx', 0o600);
      destinationOwned = true;
      try {
        await destinationHandle.writeFile(image.bytes);
        await destinationHandle.sync();
      } finally {
        await destinationHandle.close();
      }
    }
    throwIfGenerationAborted(signal);
    await fs.promises.unlink(tempPath);
    await syncWorkspaceDirectory();

    throwIfGenerationAborted(signal);
    const persistedBytes = await fs.promises.readFile(filePath);
    const persistedImage = await validateImageBytes(persistedBytes, image.mimeType);
    if (persistedImage.bytes.length !== image.bytes.length || !persistedImage.bytes.equals(image.bytes)) {
      throw new Error('Generated image failed post-write verification');
    }
    throwIfGenerationAborted(signal);
    return filePath;
  } catch (error) {
    await fs.promises.unlink(tempPath).catch(() => undefined);
    if (destinationOwned) {
      await fs.promises.unlink(filePath).catch(() => undefined);
    }
    await syncWorkspaceDirectory().catch(() => undefined);
    throw error;
  }
}

export async function saveGeneratedImages(
  imageSources: readonly string[],
  workspaceDir: string,
  signal?: AbortSignal
): Promise<string[]> {
  if (imageSources.length === 0) {
    throw new Error('No generated images were provided to save');
  }
  if (imageSources.length > MAX_GENERATED_IMAGE_COUNT) {
    throw new Error(`Too many generated images (maximum ${MAX_GENERATED_IMAGE_COUNT})`);
  }

  const validatedImages: ValidatedImage[] = [];
  let totalBytes = 0;
  for (const [index, imageSource] of imageSources.entries()) {
    try {
      throwIfGenerationAborted(signal);
      const image = await resolveGeneratedImage(imageSource, signal);
      totalBytes += image.bytes.length;
      if (totalBytes > MAX_TOTAL_GENERATED_IMAGE_BYTES) {
        throw new Error(`Generated images exceed the ${MAX_TOTAL_GENERATED_IMAGE_BYTES}-byte total size limit`);
      }
      validatedImages.push(image);
    } catch (error) {
      throw new Error(
        `Generated image ${index + 1} is invalid: ${error instanceof Error ? error.message : String(error)}`,
        { cause: error }
      );
    }
  }

  const savedPaths: string[] = [];
  try {
    for (const image of validatedImages) {
      throwIfGenerationAborted(signal);
      savedPaths.push(await persistValidatedImage(image, workspaceDir, signal));
    }
    throwIfGenerationAborted(signal);
    return savedPaths;
  } catch (error) {
    await Promise.allSettled(savedPaths.map((savedPath) => fs.promises.unlink(savedPath)));
    throw new Error(`Failed to save generated images: ${error instanceof Error ? error.message : String(error)}`, {
      cause: error,
    });
  }
}

export async function saveGeneratedImage(
  imageSource: string,
  workspaceDir: string,
  signal?: AbortSignal
): Promise<string> {
  const [savedPath] = await saveGeneratedImages([imageSource], workspaceDir, signal);
  if (!savedPath) {
    throw new Error('Generated image was not saved');
  }
  return savedPath;
}

// ===== Image Content Processing =====

interface ImageContent {
  type: 'image_url';
  image_url: {
    url: string;
    detail: 'auto' | 'low' | 'high';
  };
}

export async function processImageUri(imageUri: string, workspaceDir: string): Promise<ImageContent | null> {
  if (isHttpUrl(imageUri)) {
    return {
      type: 'image_url',
      image_url: { url: imageUri, detail: 'auto' },
    };
  }

  let processedUri = imageUri;
  if (imageUri.startsWith('@')) {
    processedUri = imageUri.substring(1);
  }

  let fullPath = processedUri;
  if (!path.isAbsolute(processedUri)) {
    fullPath = path.join(workspaceDir, processedUri);
  }

  try {
    await fs.promises.access(fullPath, fs.constants.F_OK);

    if (!isImageFile(fullPath)) {
      throw new Error(`File is not a supported image type: ${fullPath}`);
    }

    const base64Data = await fileToBase64(fullPath);
    const mimeType = getImageMimeType(fullPath);
    return {
      type: 'image_url',
      image_url: { url: `data:${mimeType};base64,${base64Data}`, detail: 'auto' },
    };
  } catch (error) {
    const possiblePaths = [imageUri, path.join(workspaceDir, imageUri)].filter((p, i, arr) => arr.indexOf(p) === i);
    const errorMessage = error instanceof Error ? error.message : String(error);

    if (errorMessage.includes('Image file not found') || errorMessage.includes('not a supported image type')) {
      throw error;
    }

    throw new Error(
      `Image file not found. Searched paths:\n${possiblePaths.map((p) => `- ${p}`).join('\n')}\n\nPlease ensure the image file exists and has a valid image extension (.jpg, .png, .gif, .webp, etc.)`,
      { cause: error }
    );
  }
}

// ===== Core Execution =====

export interface ImageGenParams {
  prompt: string;
  image_uris?: string[] | string;
}

export interface ImageGenResult {
  success: boolean;
  text: string;
  imagePath?: string;
  relativeImagePath?: string;
  imagePaths?: string[];
  relativeImagePaths?: string[];
  error?: string;
}

/**
 * Core image generation function shared between MCP server and Gemini tool.
 */
export async function executeImageGeneration(
  params: ImageGenParams,
  provider: TProviderWithModel,
  workspaceDir: string,
  proxy?: string,
  signal?: AbortSignal
): Promise<ImageGenResult> {
  if (signal?.aborted) {
    return { success: false, text: 'Image generation was cancelled.', error: 'cancelled' };
  }

  try {
    // Parse image URIs
    let imageUris: string[] = [];
    if (params.image_uris) {
      if (typeof params.image_uris === 'string') {
        const parsed = safeJsonParse<string[] | null>(params.image_uris, null);
        imageUris = Array.isArray(parsed) ? parsed : [params.image_uris];
      } else if (Array.isArray(params.image_uris)) {
        imageUris = params.image_uris;
      }
    }

    const hasImages = imageUris.length > 0;
    let enhancedPrompt: string;
    if (hasImages) {
      enhancedPrompt = `Analyze/Edit image: ${params.prompt}`;
    } else {
      enhancedPrompt = `Generate image: ${params.prompt}`;
    }

    const contentParts: OpenAI.Chat.Completions.ChatCompletionContentPart[] = [{ type: 'text', text: enhancedPrompt }];

    // Process image URIs
    if (hasImages) {
      const imageResults = await Promise.allSettled(imageUris.map((uri) => processImageUri(uri, workspaceDir)));

      const successful: ImageContent[] = [];
      const errors: string[] = [];

      imageResults.forEach((result, index) => {
        if (result.status === 'fulfilled' && result.value) {
          successful.push(result.value);
        } else {
          const error = result.status === 'rejected' ? result.reason : 'Unknown error';
          const errorMessage = error instanceof Error ? error.message : String(error);
          errors.push(`Image ${index + 1} (${imageUris[index]}): ${errorMessage}`);
        }
      });

      successful.forEach((imageContent) => contentParts.push(imageContent));

      if (successful.length === 0) {
        return {
          success: false,
          text: `Error: Failed to process any images. Errors:\n${errors.join('\n')}`,
          error: errors.join('\n'),
        };
      }
    }

    const messages: OpenAI.Chat.Completions.ChatCompletionMessageParam[] = [{ role: 'user', content: contentParts }];

    // Create client and call API
    const rotatingClient: RotatingClient = await ClientFactory.createRotatingClient(provider, {
      proxy,
      rotatingOptions: { maxRetries: 3, retryDelay: 1000 },
    });

    // `createChatCompletion` is typed as a union of the per-provider return
    // shapes; the OpenAI SDK's `ChatCompletion` differs only in that `content`
    // is `string | null` (handled by the runtime type check below) and `images`
    // is absent (handled by the `!images` guard below). The runtime object is
    // already consumed as the unified shape, so this assertion is type-only.
    const completion = (await rotatingClient.createChatCompletion(
      { model: provider.use_model, messages: messages as any },
      { signal, timeout: API_TIMEOUT_MS }
    )) as UnifiedChatCompletionResponse;

    const choice = completion.choices[0];
    if (!choice) {
      return { success: false, text: 'No response from image generation API', error: 'No response' };
    }

    const responseText = typeof choice.message.content === 'string' ? choice.message.content : '';
    let images = choice.message.images;

    // Extract images from markdown in content if not in images field
    if ((!images || images.length === 0) && responseText) {
      const extractedSources: string[] = [];
      const dataUrlRegex = /!\[[^\]]*\]\((data:image\/[^;]+;base64,[^)]+)\)/g;
      const dataUrlMatches = [...responseText.matchAll(dataUrlRegex)];
      extractedSources.push(...dataUrlMatches.map((match) => match[1]));

      const httpUrlRegex = /!\[[^\]]*\]\((https?:\/\/[^)\s]+)\)/gi;
      const httpUrlMatches = [...responseText.matchAll(httpUrlRegex)];
      extractedSources.push(...httpUrlMatches.map((match) => match[1]));

      if (extractedSources.length > 0) {
        images = [...new Set(extractedSources)].map((source) => ({
          type: 'image_url' as const,
          image_url: { url: source },
        }));
      }
    }

    if (!images || images.length === 0) {
      const modelResponse = responseText.trim() || '(empty response)';
      const errorMessage = `Image generation did not produce any images. Model response: ${modelResponse}`;
      return {
        success: false,
        text: `Error: ${errorMessage}\n\nTip: Make sure the selected model supports image generation. Current model: ${provider.use_model}`,
        error: errorMessage,
      };
    }

    const imageSources = images.map((image, index) => {
      if (image?.type !== 'image_url' || typeof image.image_url?.url !== 'string' || !image.image_url.url.trim()) {
        throw new Error(`Image ${index + 1} has an unsupported or missing image source`);
      }
      return image.image_url.url;
    });
    const imagePaths = await saveGeneratedImages(imageSources, workspaceDir, signal);
    if (imagePaths.length !== imageSources.length) {
      throw new Error(`Only ${imagePaths.length} of ${imageSources.length} generated images were saved`);
    }
    const relativeImagePaths = imagePaths.map((imagePath) => path.basename(imagePath));

    // Strip any inline base64 data URLs from the human-readable text before
    // returning. The images are already saved to disk and referenced by path,
    // so re-emitting the Base64 payload would force the parent process to ship
    // it through framed TCP again.
    const cleanText = responseText
      .replace(/!\[[^\]]*\]\(data:image\/[^;]+;base64,[^)]+\)/g, '[embedded image extracted]')
      .trim();
    const savedText =
      imagePaths.length === 1
        ? `Generated image saved to: ${imagePaths[0]}`
        : `Generated images saved to:\n${imagePaths.map((imagePath) => `- ${imagePath}`).join('\n')}`;

    return {
      success: true,
      text: cleanText ? `${cleanText}\n\n${savedText}` : savedText,
      imagePath: imagePaths[0],
      relativeImagePath: relativeImagePaths[0],
      imagePaths,
      relativeImagePaths,
    };
  } catch (error) {
    if (signal?.aborted) {
      return { success: false, text: 'Image generation was cancelled.', error: 'cancelled' };
    }
    const errorMessage = error instanceof Error ? error.message : String(error);
    console.error(`[ImageGen] API call failed:`, error);
    return { success: false, text: `Error generating image: ${errorMessage}`, error: errorMessage };
  }
}
