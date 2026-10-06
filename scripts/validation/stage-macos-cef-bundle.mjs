#!/usr/bin/env node
// CLI shared by production packaging and native development bundles.
import { stageMacosBrowserBundle } from '../lib/macos-browser-bundle.mjs';
const args = process.argv.slice(2);
const option = key => {
  const index = args.indexOf(key);
  if (index < 0 || !args[index + 1] || args[index + 1].startsWith('--')) throw new Error(`${key} requires a value`);
  return args[index + 1];
};
try {
  const result = await stageMacosBrowserBundle({
    appPath: option('--app'), helperPath: option('--helper'), runtimePath: option('--runtime'),
    identity: args.includes('--identity') ? option('--identity') : '-',
  });
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
} catch (error) {
  process.stderr.write(`MACOS_CEF_STAGE_FAIL ${error.message}\n`);
  process.exitCode = 1;
}
