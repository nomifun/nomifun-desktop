import { fileURLToPath } from 'node:url';

export const MACOS_APPLICATION_ENTITLEMENTS = fileURLToPath(
  new URL('../../apps/desktop/entitlements.plist', import.meta.url),
);

// Resource access belongs to the app executable, not its bundled ONNX library.
// These declarations do not replace TCC consent or browser per-site permission.
export function macosCodeSigningArguments(path, identity, {
  application = false, entitlementsPath = MACOS_APPLICATION_ENTITLEMENTS,
} = {}) {
  return ['--force', ...(identity === '-' ? [] : ['--options', 'runtime', '--timestamp']),
    ...(application ? ['--entitlements', entitlementsPath] : []), '--sign', identity, path];
}
