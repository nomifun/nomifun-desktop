#!/usr/bin/env node
// Packaging only. Native launch/close remains the caller's authorized computer
// use action through the existing LaunchServices controller transport.
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { packageMacosNativeFixture } from './package-macos-browser-media-fixture.mjs';

export async function main(args = process.argv.slice(2)) {
  if (args.includes('--help')) {
    console.log('Use --kind compatibility|appkit --binary /ABS/CURRENT/EXAMPLE --output-dir /ABS/NEW/ROOT [-- APP_ARGS]. Packages and signs a fixed audit App with an independent data root and retained evidence. It never launches a native application. Launch requires the existing compiled macos_application_controller, fixed descriptor transport and authenticated final cleanup/actual-exit proof.');
    return;
  }
  const separator = args.indexOf('--');
  const packagingArgs = separator < 0 ? args : args.slice(0, separator);
  const option = name => { const index = packagingArgs.indexOf(name); return index < 0 ? undefined : packagingArgs[index + 1]; };
  const kind = option('--kind');
  if (!['compatibility', 'appkit'].includes(kind)) throw new Error('--kind compatibility|appkit is required; media retains its separate fixed entry');
  const report = await packageMacosNativeFixture({ kind, binary: option('--binary'), outputRoot: option('--output-dir'),
    applicationArgs: separator < 0 ? [] : args.slice(separator + 1) });
  console.log(JSON.stringify(report));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
