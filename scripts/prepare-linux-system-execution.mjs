#!/usr/bin/env bun
import { prepareLinuxSystemExecution } from './lib/linux-system-execution.mjs';

const args = process.argv.slice(2);
const options = {};
for (let index = 0; index < args.length; index += 2) {
  if (!['--target', '--profile'].includes(args[index]) || !args[index + 1]) throw new Error(`unsupported argument: ${args[index]}`);
  options[args[index].slice(2)] = args[index + 1];
}
const prepared = await prepareLinuxSystemExecution(options);
console.error(`[linux-system-execution] staged ${prepared.directory}`);
console.log(prepared.config);
