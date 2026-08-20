/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from 'bun:test';
import { BoxGeometry, Mesh, MeshStandardMaterial, Skeleton, Texture } from 'three';

import {
  directorAssetResourcePath,
  disposeDirectorObject3D,
  resolveTrustedDirectorAssetUrl,
} from './resources';

describe('Director trusted asset URLs', () => {
  test('accepts backend-relative, HTTPS, and blob URLs', () => {
    expect(
      resolveTrustedDirectorAssetUrl('/api/workshop/files/asset-1', 'https://nomifun.test/workshop/')
    ).toBe('https://nomifun.test/api/workshop/files/asset-1');
    expect(resolveTrustedDirectorAssetUrl('https://assets.nomifun.test/model.glb')).toBe(
      'https://assets.nomifun.test/model.glb'
    );
    expect(resolveTrustedDirectorAssetUrl('blob:https://nomifun.test/asset-1')).toBe(
      'blob:https://nomifun.test/asset-1'
    );
  });

  test('rejects data, javascript, file, and empty URLs', () => {
    for (const value of [
      'data:model/gltf-binary;base64,AAAA',
      'javascript:alert(1)',
      'file:///tmp/model.glb',
      '   ',
    ]) {
      let rejected = false;
      try {
        resolveTrustedDirectorAssetUrl(value);
      } catch {
        rejected = true;
      }
      expect(rejected).toBe(true);
    }
  });

  test('derives the GLTF dependency base without query or fragment leakage', () => {
    expect(directorAssetResourcePath('https://assets.nomifun.test/models/scene.gltf?token=x#y')).toBe(
      'https://assets.nomifun.test/models/'
    );
    expect(directorAssetResourcePath('blob:https://nomifun.test/asset-1')).toBe('');
  });
});

describe('Director Three.js resource disposal', () => {
  test('disposes shared geometry, material, and textures once and detaches the root', () => {
    const geometry = new BoxGeometry();
    const texture = new Texture();
    let imageCloses = 0;
    texture.image = { close: () => imageCloses += 1 };
    const material = new MeshStandardMaterial({ map: texture });
    const parent = new Mesh();
    const root = new Mesh(geometry, material);
    root.add(new Mesh(geometry, material));
    parent.add(root);
    let geometryDisposals = 0;
    let materialDisposals = 0;
    let textureDisposals = 0;
    let skeletonDisposals = 0;
    const skeleton = new Skeleton();
    skeleton.dispose = () => skeletonDisposals += 1;
    Object.assign(root, { skeleton });
    geometry.addEventListener('dispose', () => geometryDisposals += 1);
    material.addEventListener('dispose', () => materialDisposals += 1);
    texture.addEventListener('dispose', () => textureDisposals += 1);

    disposeDirectorObject3D(root);

    expect(root.parent).toBeNull();
    expect(geometryDisposals).toBe(1);
    expect(materialDisposals).toBe(1);
    expect(textureDisposals).toBe(1);
    expect(imageCloses).toBe(1);
    expect(skeletonDisposals).toBe(1);
  });
});
