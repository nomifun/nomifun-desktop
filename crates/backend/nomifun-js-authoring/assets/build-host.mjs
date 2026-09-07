import fs from "node:fs/promises";
import { stripTypeScriptTypes } from "node:module";
import { SourceTextModule } from "node:vm";

const [requestPath, responsePath] = process.argv.slice(2);

function assertExactKeys(value, keys, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label} must be an object`);
  }
  const actual = Object.keys(value).sort();
  const expected = [...keys].sort();
  if (actual.length !== expected.length ||
      actual.some((key, index) => key !== expected[index])) {
    throw new Error(`${label} contains unsupported fields`);
  }
}

async function writeResponse(response) {
  await fs.writeFile(responsePath, JSON.stringify(response), { flag: "wx" });
}

async function main() {
try {
  const major = Number.parseInt(process.versions.node.split(".", 1)[0], 10);
  if (major !== 24) {
    throw new Error(`Node 24 is required, observed ${process.versions.node}`);
  }
  const request = JSON.parse(await fs.readFile(requestPath, "utf8"));
  assertExactKeys(
    request,
    ["format_version", "language", "source_path", "output_path", "map_path", "source_url"],
    "build request",
  );
  if (request.format_version !== "1.0.0") {
    throw new Error("unsupported build request version");
  }
  if (request.language !== "javascript" && request.language !== "typescript") {
    throw new Error("unsupported source language");
  }

  const source = await fs.readFile(request.source_path, "utf8");
  let output = source;
  let sourceMap = null;
  if (request.language === "typescript") {
    output = stripTypeScriptTypes(source, {
      mode: "transform",
      sourceMap: true,
      sourceUrl: request.source_url,
    });
    const marker = "\n//# sourceMappingURL=data:application/json;base64,";
    const markerIndex = output.lastIndexOf(marker);
    if (markerIndex < 0) {
      throw new Error("Node TypeScript transform did not produce a source map");
    }
    sourceMap = Buffer.from(output.slice(markerIndex + marker.length), "base64");
    output = `${output.slice(0, markerIndex)}\n//# sourceMappingURL=main.mjs.map\n`;
  }

  const dynamicModulePattern =
    /\b(?:import|require)(?:\s|\/\*[\s\S]*?\*\/|\/\/[^\r\n]*(?:\r?\n|$))*\(/;
  if (dynamicModulePattern.test(output)) {
    await writeResponse({
      format_version: "1.0.0",
      status: "unsupported_module",
      specifier: "<dynamic module expression>",
    });
    process.exitCode = 42;
    return;
  }

  const module = new SourceTextModule(output, {
    identifier: request.source_url,
  });
  for (const moduleRequest of module.moduleRequests) {
    if (!moduleRequest.specifier.startsWith("node:") ||
        moduleRequest.specifier === "node:module") {
      await writeResponse({
        format_version: "1.0.0",
        status: "unsupported_module",
        specifier: moduleRequest.specifier,
      });
      process.exitCode = 42;
      return;
    }
  }

  await fs.writeFile(request.output_path, output, { flag: "wx" });
  if (sourceMap !== null) {
    await fs.writeFile(request.map_path, sourceMap, { flag: "wx" });
  }
  await writeResponse({
    format_version: "1.0.0",
    status: "ok",
    source_map_written: sourceMap !== null,
  });
} catch (error) {
  try {
    await writeResponse({
      format_version: "1.0.0",
      status: "failed",
      message: error instanceof Error ? error.message : String(error),
    });
  } catch {
    // Rust still receives the original failure through stderr and exit status.
  }
  console.error(error instanceof Error ? error.stack : String(error));
  process.exitCode = 1;
}
}

await main();
