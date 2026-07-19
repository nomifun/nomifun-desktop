/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import * as fs from 'fs';
import * as os from 'os';
import * as path from 'path';
import { deflateSync, inflateSync } from 'zlib';
import { ClientFactory } from '@/common/api/ClientFactory';
import type { UnifiedChatCompletionResponse } from '@/common/api/RotatingApiClient';
import type { TProviderWithModel } from '@/common/config/storage';
import {
  executeImageGeneration,
  MAX_GENERATED_IMAGE_BYTES,
  resolveWorkspaceGeneratedImagePath,
  saveGeneratedImage,
  saveGeneratedImages,
} from './imageGenCore';

const PNG_BASE64 =
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=';
const PNG_BYTES = Buffer.from(PNG_BASE64, 'base64');
const PNG_DATA_URL = `data:image/png;base64,${PNG_BASE64}`;
const TEST_PROVIDER = { use_model: 'test-image-model' } as TProviderWithModel;

function corruptPngWithValidContainerShape(): Buffer {
  const bytes = Buffer.from(PNG_BYTES);
  const idatTypeOffset = bytes.indexOf(Buffer.from('IDAT'));
  if (idatTypeOffset < 4) throw new Error('test PNG is missing IDAT');
  const dataLength = bytes.readUInt32BE(idatTypeOffset - 4);
  bytes.fill(0xff, idatTypeOffset + 4, idatTypeOffset + 4 + dataLength);
  return bytes;
}

function pngCrc32(bytes: Buffer): number {
  let crc = 0xffffffff;
  for (const value of bytes) {
    crc ^= value;
    for (let bit = 0; bit < 8; bit += 1) {
      crc = (crc & 1) !== 0 ? 0xedb88320 ^ (crc >>> 1) : crc >>> 1;
    }
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function pngWithInvalidFilterByte(): Buffer {
  const typeOffset = PNG_BYTES.indexOf(Buffer.from('IDAT'));
  if (typeOffset < 4) throw new Error('test PNG is missing IDAT');
  const lengthOffset = typeOffset - 4;
  const dataLength = PNG_BYTES.readUInt32BE(lengthOffset);
  const dataStart = typeOffset + 4;
  const dataEnd = dataStart + dataLength;
  const scanlines = inflateSync(PNG_BYTES.subarray(dataStart, dataEnd));
  scanlines[0] = 5; // PNG permits only filter algorithms 0..4.
  const compressed = deflateSync(scanlines);
  const length = Buffer.alloc(4);
  length.writeUInt32BE(compressed.length);
  const type = Buffer.from('IDAT');
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(pngCrc32(Buffer.concat([type, compressed])));
  return Buffer.concat([
    PNG_BYTES.subarray(0, lengthOffset),
    length,
    type,
    compressed,
    crc,
    PNG_BYTES.subarray(dataEnd + 4),
  ]);
}

async function withTempWorkspace<T>(run: (workspace: string) => Promise<T>): Promise<T> {
  const workspace = await fs.promises.mkdtemp(path.join(os.tmpdir(), 'nomifun-image-gen-test-'));
  try {
    return await run(workspace);
  } finally {
    await fs.promises.rm(workspace, { recursive: true, force: true });
  }
}

async function expectRejected(promise: Promise<unknown>, expectedMessage: string): Promise<void> {
  let caught: unknown;
  try {
    await promise;
  } catch (error) {
    caught = error;
  }
  expect(caught instanceof Error).toBe(true);
  expect(caught instanceof Error && caught.message.includes(expectedMessage)).toBe(true);
}

function completion(content: string, imageSources?: string[]): UnifiedChatCompletionResponse {
  return {
    id: 'test-completion',
    object: 'chat.completion',
    created: 0,
    model: 'test-image-model',
    choices: [
      {
        index: 0,
        message: {
          role: 'assistant',
          content,
          images: imageSources?.map((url) => ({ type: 'image_url', image_url: { url } })),
        },
        finish_reason: 'stop',
      },
    ],
  };
}

async function withMockedCompletion<T>(
  value: UnifiedChatCompletionResponse,
  run: () => Promise<T>
): Promise<T> {
  const originalCreateRotatingClient = ClientFactory.createRotatingClient;
  ClientFactory.createRotatingClient = (async () => ({
    createChatCompletion: async () => value,
  })) as unknown as typeof ClientFactory.createRotatingClient;
  try {
    return await run();
  } finally {
    ClientFactory.createRotatingClient = originalCreateRotatingClient;
  }
}

describe('generated image persistence', () => {
  test('saves every data URL and raw Base64 image with unique verified paths', async () => {
    await withTempWorkspace(async (workspace) => {
      const savedPaths = await saveGeneratedImages([PNG_DATA_URL, PNG_BASE64], workspace);

      expect(savedPaths).toHaveLength(2);
      expect(new Set(savedPaths).size).toBe(2);
      for (const savedPath of savedPaths) {
        expect(path.dirname(savedPath)).toBe(await fs.promises.realpath(workspace));
        expect((await fs.promises.readFile(savedPath)).equals(PNG_BYTES)).toBe(true);
      }
      const entries = await fs.promises.readdir(workspace);
      expect(entries).toHaveLength(2);
      expect(entries.every((entry) => entry.endsWith('.png') && !entry.endsWith('.tmp'))).toBe(true);
    });
  });

  test('downloads HTTP(S) image sources instead of decoding the URL as Base64', async () => {
    await withTempWorkspace(async (workspace) => {
      const originalFetch = globalThis.fetch;
      globalThis.fetch = (async () =>
        new Response(new Uint8Array(PNG_BYTES), {
          status: 200,
          headers: { 'content-type': 'image/png', 'content-length': String(PNG_BYTES.length) },
        })) as typeof fetch;
      try {
        const savedPath = await saveGeneratedImage('https://example.invalid/generated-image', workspace);
        expect((await fs.promises.readFile(savedPath)).equals(PNG_BYTES)).toBe(true);
      } finally {
        globalThis.fetch = originalFetch;
      }
    });
  });

  test('never overwrites an existing destination when publication collides', async () => {
    await withTempWorkspace(async (workspace) => {
      const originalLink = fs.promises.link;
      const existingBytes = Buffer.from('pre-existing artifact must survive');
      let destinationPath: string | undefined;
      fs.promises.link = (async (existingPath, newPath) => {
        destinationPath = path.resolve(newPath.toString());
        await fs.promises.writeFile(destinationPath, existingBytes, { flag: 'wx' });
        return originalLink(existingPath, newPath);
      }) as typeof fs.promises.link;

      try {
        await expectRejected(saveGeneratedImage(PNG_DATA_URL, workspace), 'EEXIST');
        expect(destinationPath).toBeDefined();
        expect((await fs.promises.readFile(destinationPath!)).equals(existingBytes)).toBe(true);
        expect((await fs.promises.readdir(workspace)).filter((entry) => entry.endsWith('.tmp'))).toHaveLength(0);
      } finally {
        fs.promises.link = originalLink;
      }
    });
  });

  test('falls back to exclusive durable publication when the filesystem does not support hard links', async () => {
    await withTempWorkspace(async (workspace) => {
      const originalLink = fs.promises.link;
      fs.promises.link = (async () => {
        const error = new Error('hard links are unsupported') as NodeJS.ErrnoException;
        error.code = 'ENOTSUP';
        throw error;
      }) as typeof fs.promises.link;

      try {
        const savedPath = await saveGeneratedImage(PNG_DATA_URL, workspace);
        expect((await fs.promises.readFile(savedPath)).equals(PNG_BYTES)).toBe(true);
        expect((await fs.promises.readdir(workspace)).filter((entry) => entry.endsWith('.tmp'))).toHaveLength(0);
      } finally {
        fs.promises.link = originalLink;
      }
    });
  });

  test('requires an intact decodable pixel stream instead of accepting a plausible image container', async () => {
    await withTempWorkspace(async (workspace) => {
      const corruptPng = corruptPngWithValidContainerShape();
      const emptyGifContainer = Buffer.from([
        ...Buffer.from('GIF89a'),
        0x01, 0x00, 0x01, 0x00,
        0x00, 0x00, 0x00,
        0x3b,
      ]);

      await expectRejected(
        saveGeneratedImage(`data:image/png;base64,${corruptPng.toString('base64')}`, workspace),
        'supported image signature'
      );
      await expectRejected(
        saveGeneratedImage(`data:image/gif;base64,${emptyGifContainer.toString('base64')}`, workspace),
        'supported image signature'
      );
      expect(await fs.promises.readdir(workspace)).toHaveLength(0);
    });
  });

  test('rejects a CRC-valid PNG whose decompressed scanline uses an invalid filter', async () => {
    await withTempWorkspace(async (workspace) => {
      const invalidFilterPng = pngWithInvalidFilterByte();
      await expectRejected(
        saveGeneratedImage(`data:image/png;base64,${invalidFilterPng.toString('base64')}`, workspace),
        'supported image signature'
      );
      expect(await fs.promises.readdir(workspace)).toHaveLength(0);
    });
  });

  test('cancellation during publication retracts every file instead of returning success', async () => {
    await withTempWorkspace(async (workspace) => {
      const controller = new AbortController();
      const originalLink = fs.promises.link;
      fs.promises.link = (async (existingPath, newPath) => {
        await originalLink(existingPath, newPath);
        controller.abort('test cancellation');
      }) as typeof fs.promises.link;
      try {
        await expectRejected(saveGeneratedImage(PNG_DATA_URL, workspace, controller.signal), 'cancelled');
        expect(await fs.promises.readdir(workspace)).toHaveLength(0);
      } finally {
        fs.promises.link = originalLink;
      }
    });
  });

  test('rejects empty, malformed, and MIME-mismatched payloads without leaving files', async () => {
    await withTempWorkspace(async (workspace) => {
      await expectRejected(saveGeneratedImage('', workspace), 'source is empty');
      await expectRejected(saveGeneratedImage('not-valid-base64!', workspace), 'invalid Base64');
      await expectRejected(saveGeneratedImage(`data:image/jpeg;base64,${PNG_BASE64}`, workspace), 'MIME mismatch');
      await expectRejected(
        saveGeneratedImage(
          `data:image/svg+xml;base64,${Buffer.from('<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>').toString('base64')}`,
          workspace
        ),
        'supported image signature'
      );
      await expectRejected(
        saveGeneratedImage(`data:image/png;base64,${Buffer.from('\x89PNG\r\n\x1a\n', 'binary').toString('base64')}`, workspace),
        'supported image signature'
      );
      expect(await fs.promises.readdir(workspace)).toHaveLength(0);
    });
  });

  test('rejects oversized HTTP responses before reading or writing a payload', async () => {
    await withTempWorkspace(async (workspace) => {
      const originalFetch = globalThis.fetch;
      globalThis.fetch = (async () =>
        new Response(new Uint8Array(PNG_BYTES), {
          status: 200,
          headers: {
            'content-type': 'image/png',
            'content-length': String(MAX_GENERATED_IMAGE_BYTES + 1),
          },
        })) as typeof fetch;
      try {
        await expectRejected(
          saveGeneratedImage('https://example.invalid/oversized-image', workspace),
          'exceeds the'
        );
        expect(await fs.promises.readdir(workspace)).toHaveLength(0);
      } finally {
        globalThis.fetch = originalFetch;
      }
    });
  });
});

describe('legacy image generation completion semantics', () => {
  test('returns failure and never fabricates a success message when the model emits no image', async () => {
    await withTempWorkspace(async (workspace) => {
      const result = await withMockedCompletion(completion(''), () =>
        executeImageGeneration({ prompt: 'draw a cat' }, TEST_PROVIDER, workspace)
      );

      expect(result.success).toBe(false);
      expect(Boolean(result.error)).toBe(true);
      expect(result.text.includes('Image generated successfully.')).toBe(false);
      expect(await fs.promises.readdir(workspace)).toHaveLength(0);
    });
  });

  test('persists and reports every generated image', async () => {
    await withTempWorkspace(async (workspace) => {
      const result = await withMockedCompletion(completion('Two generated images', [PNG_DATA_URL, PNG_BASE64]), () =>
        executeImageGeneration({ prompt: 'draw two cats' }, TEST_PROVIDER, workspace)
      );

      expect(result.success).toBe(true);
      expect(result.imagePaths).toHaveLength(2);
      expect(result.relativeImagePaths).toHaveLength(2);
      expect(result.imagePath).toBe(result.imagePaths?.[0]);
      expect(result.relativeImagePath).toBe(result.relativeImagePaths?.[0]);
      expect(await fs.promises.readdir(workspace)).toHaveLength(2);
    });
  });

  test('does not report success or leave a partial result when any generated image is invalid', async () => {
    await withTempWorkspace(async (workspace) => {
      const result = await withMockedCompletion(completion('Mixed result', [PNG_DATA_URL, 'invalid!']), () =>
        executeImageGeneration({ prompt: 'draw two cats' }, TEST_PROVIDER, workspace)
      );

      expect(result.success).toBe(false);
      expect(Boolean(result.error)).toBe(true);
      expect(await fs.promises.readdir(workspace)).toHaveLength(0);
    });
  });

  test('never adopts an older workspace file merely because model markdown names its path', async () => {
    await withTempWorkspace(async (workspace) => {
      const oldImage = path.join(workspace, 'old.png');
      await fs.promises.writeFile(oldImage, PNG_BYTES);
      const result = await withMockedCompletion(completion('![claimed result](old.png)'), () =>
        executeImageGeneration({ prompt: 'draw a new cat' }, TEST_PROVIDER, workspace)
      );

      expect(result.success).toBe(false);
      expect(result.imagePaths).toBeUndefined();
      expect(await fs.promises.readdir(workspace)).toEqual(['old.png']);
    });
  });
});

describe('model-authored generated image paths', () => {
  test('accepts only canonical files contained by the active workspace', async () => {
    await withTempWorkspace(async (workspace) => {
      const nested = path.join(workspace, 'nested');
      await fs.promises.mkdir(nested);
      const image = path.join(nested, 'image.png');
      await fs.promises.writeFile(image, PNG_BYTES);

      expect(await resolveWorkspaceGeneratedImagePath('nested/../nested/image.png', workspace)).toBe(
        await fs.promises.realpath(image)
      );
      expect(await resolveWorkspaceGeneratedImagePath(image, workspace)).toBe(await fs.promises.realpath(image));
    });
  });

  test('rejects absolute and parent paths that escape the workspace', async () => {
    const parent = await fs.promises.mkdtemp(path.join(os.tmpdir(), 'nomifun-image-gen-boundary-'));
    const workspace = path.join(parent, 'workspace');
    const outside = path.join(parent, 'old.png');
    await fs.promises.mkdir(workspace);
    await fs.promises.writeFile(outside, PNG_BYTES);
    try {
      await expectRejected(
        resolveWorkspaceGeneratedImagePath(outside, workspace),
        'outside the active workspace'
      );
      await expectRejected(
        resolveWorkspaceGeneratedImagePath('../old.png', workspace),
        'outside the active workspace'
      );
    } finally {
      await fs.promises.rm(parent, { recursive: true, force: true });
    }
  });

  test('rejects a workspace symlink or Windows junction that resolves outside', async () => {
    const parent = await fs.promises.mkdtemp(path.join(os.tmpdir(), 'nomifun-image-gen-symlink-'));
    const workspace = path.join(parent, 'workspace');
    const outside = path.join(parent, 'outside');
    const outsideImage = path.join(outside, 'old.png');
    const link = path.join(workspace, 'generated');
    await fs.promises.mkdir(workspace);
    await fs.promises.mkdir(outside);
    await fs.promises.writeFile(outsideImage, PNG_BYTES);
    await fs.promises.symlink(outside, link, process.platform === 'win32' ? 'junction' : 'dir');
    try {
      await expectRejected(
        resolveWorkspaceGeneratedImagePath(path.join('generated', 'old.png'), workspace),
        'outside the active workspace'
      );
    } finally {
      await fs.promises.unlink(link).catch(() => undefined);
      await fs.promises.rm(parent, { recursive: true, force: true });
    }
  });
});
