/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  BufferGeometry,
  Material,
  Object3D,
  Texture,
  type Skeleton,
  type WebGLRenderTarget,
} from 'three';

const TRUSTED_ASSET_PROTOCOLS = new Set(['http:', 'https:', 'blob:']);

export function resolveTrustedDirectorAssetUrl(
  value: string,
  baseUrl = globalThis.location?.href ?? 'https://nomifun.invalid/'
): string {
  const trimmed = value.trim();
  if (!trimmed) throw new TypeError('Asset resolver returned an empty URL');
  const resolved = new URL(trimmed, baseUrl);
  if (!TRUSTED_ASSET_PROTOCOLS.has(resolved.protocol)) {
    throw new TypeError(`Asset resolver returned an unsupported URL protocol: ${resolved.protocol}`);
  }
  return resolved.href;
}

export function directorAssetResourcePath(url: string): string {
  const parsed = new URL(url);
  if (parsed.protocol === 'blob:') return '';
  parsed.hash = '';
  parsed.search = '';
  const slash = parsed.pathname.lastIndexOf('/');
  parsed.pathname = slash >= 0 ? parsed.pathname.slice(0, slash + 1) : '/';
  return parsed.href;
}

function materialTextures(material: Material): Texture[] {
  const textures: Texture[] = [];
  for (const value of Object.values(material)) {
    if (value instanceof Texture) textures.push(value);
  }
  return textures;
}

/**
 * Dispose a loaded GLTF subtree exactly once. Assets are never shared between
 * bindings, so disposing one entity cannot invalidate another entity's model.
 */
export function disposeDirectorObject3D(root: Object3D): void {
  const geometries = new Set<BufferGeometry>();
  const materials = new Set<Material>();
  const textures = new Set<Texture>();
  const skeletons = new Set<Skeleton>();
  const closableImages = new Set<{ close(): void }>();

  root.traverse((object) => {
    const candidate = object as Object3D & {
      geometry?: BufferGeometry;
      material?: Material | Material[];
      skeleton?: Skeleton;
    };
    if (candidate.geometry) geometries.add(candidate.geometry);
    if (candidate.skeleton) skeletons.add(candidate.skeleton);
    const currentMaterials = Array.isArray(candidate.material)
      ? candidate.material
      : candidate.material
        ? [candidate.material]
        : [];
    for (const material of currentMaterials) {
      materials.add(material);
      for (const texture of materialTextures(material)) textures.add(texture);
    }
  });

  root.removeFromParent();
  for (const texture of textures) {
    const image = texture.image as { close?: () => void } | null | undefined;
    if (image && typeof image.close === 'function') {
      closableImages.add(image as { close(): void });
    }
    texture.dispose();
  }
  for (const image of closableImages) image.close();
  for (const skeleton of skeletons) skeleton.dispose();
  for (const material of materials) material.dispose();
  for (const geometry of geometries) geometry.dispose();
}

export function disposeDirectorRenderTarget(target: WebGLRenderTarget | null): void {
  target?.dispose();
}
