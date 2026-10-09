import { dirname, join } from 'node:path';
import { INTEL_ONNX_RUNTIME, prepareIntelOnnxRuntime } from './macos-onnx-runtime.mjs';

export function resolveMacosTarget(target, architecture = process.arch) {
  const resolved = target ?? ({ arm64: 'aarch64-apple-darwin', x64: 'x86_64-apple-darwin' })[architecture];
  if (!['aarch64-apple-darwin', 'x86_64-apple-darwin'].includes(resolved)) {
    throw new Error('macOS builds support aarch64-apple-darwin and x86_64-apple-darwin');
  }
  return resolved;
}

// ONNX is the only separately prepared native runtime. WebKit is supplied by
// macOS; the compiler and bundle do not discover or download a browser engine.
export async function prepareMacosBuildEnvironment({
  target = null, architecture = process.arch, environment = process.env,
} = {}, { prepareOnnx = prepareIntelOnnxRuntime } = {}) {
  const resolved = resolveMacosTarget(target ?? environment.CARGO_BUILD_TARGET, architecture);
  const compiled = { ...environment };
  delete compiled.ORT_LIB_PATH;
  delete compiled.ORT_LIB_LOCATION;
  delete compiled.ORT_PREFER_DYNAMIC_LINK;
  if (resolved !== 'x86_64-apple-darwin') {
    return { target: resolved, environment: compiled, bundleFiles: {}, bundleConfig: null };
  }
  const libraries = await prepareOnnx();
  compiled.ORT_LIB_PATH = libraries;
  compiled.ORT_PREFER_DYNAMIC_LINK = '1';
  const rpath = 'link-arg=-Wl,-rpath,@executable_path/../Frameworks';
  // Cargo gives encoded flags precedence. Preserve that contract when adding
  // the rpath needed by both a dev app and a bundled fast build.
  if (compiled.CARGO_ENCODED_RUSTFLAGS !== undefined) {
    const flags = compiled.CARGO_ENCODED_RUSTFLAGS.split('\x1f').filter(Boolean);
    if (!flags.includes(rpath)) flags.push('-C', rpath);
    compiled.CARGO_ENCODED_RUSTFLAGS = flags.join('\x1f');
  } else {
    const flags = compiled.RUSTFLAGS ?? '';
    compiled.RUSTFLAGS = flags.includes(rpath) ? flags : `${flags} -C ${rpath}`.trim();
  }
  const runtime = dirname(libraries);
  return {
    target: resolved,
    environment: compiled,
    bundleConfig: join(runtime, 'tauri.conf.json'),
    bundleFiles: {
      [`Frameworks/${INTEL_ONNX_RUNTIME.library}`]: join(libraries, INTEL_ONNX_RUNTIME.library),
      'Resources/onnxruntime-LICENSE': join(runtime, 'LICENSE'),
      'Resources/onnxruntime-ThirdPartyNotices.txt': join(runtime, 'ThirdPartyNotices.txt'),
    },
  };
}
