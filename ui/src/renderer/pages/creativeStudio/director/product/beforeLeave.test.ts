/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { afterEach, describe, expect, test } from "bun:test";

import {
  hasCreativeDirectorProductBeforeLeave,
  registerCreativeDirectorProductBeforeLeave,
  requestCreativeDirectorProductBeforeLeave,
} from "./beforeLeave";

let cleanup: (() => void) | null = null;

afterEach(() => {
  cleanup?.();
  cleanup = null;
});

describe("Creative Director product before-leave registry", () => {
  test("defaults to safe when the product is not mounted", async () => {
    expect(hasCreativeDirectorProductBeforeLeave()).toBe(false);
    expect(await requestCreativeDirectorProductBeforeLeave()).toBe(true);
  });

  test("uses only the latest mounted owner and fails closed", async () => {
    cleanup = registerCreativeDirectorProductBeforeLeave(async () => false);
    const firstCleanup = cleanup;
    cleanup = registerCreativeDirectorProductBeforeLeave(async () => true);
    firstCleanup();
    expect(await requestCreativeDirectorProductBeforeLeave()).toBe(true);

    cleanup();
    cleanup = registerCreativeDirectorProductBeforeLeave(async () => {
      throw new Error("flush failed");
    });
    expect(await requestCreativeDirectorProductBeforeLeave()).toBe(false);
  });
});
