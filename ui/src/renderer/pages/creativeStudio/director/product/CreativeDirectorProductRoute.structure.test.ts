/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";

const source = readFileSync(
  new URL("./CreativeDirectorProductRoute.tsx", import.meta.url),
  "utf8",
);

describe("CreativeDirectorProductRoute structure", () => {
  test("owns route identity, project CAS, real assets, runtime, and leave gate", () => {
    expect(source.includes("useParams<{ projectId: string }>()")).toBe(true);
    expect(source.includes("creativeProjectRepository.load(projectId)")).toBe(
      true,
    );
    expect(source.includes("persistDirectorProject({")).toBe(true);
    expect(source.includes("<DirectorRuntimeViewport")).toBe(true);
    expect(source.includes("creativeAssetClient.upload(file")).toBe(true);
    expect(source.includes("registerCreativeDirectorProductBeforeLeave")).toBe(
      true,
    );
    expect(source.includes("creativeStudioCanvasProjectPath(projectId)")).toBe(
      true,
    );
    expect(source.includes("返回画布")).toBe(true);
  });

  test("does not substitute bundled media or pretend unsupported exports exist", () => {
    expect(/\.glb['"`]/i.test(source)).toBe(false);
    expect(source.includes("MediaRecorder")).toBe(false);
    expect(source.includes("onTimelineExport=")).toBe(false);
    expect(source.includes("onCaptureSendToCanvas=")).toBe(false);
  });
});
