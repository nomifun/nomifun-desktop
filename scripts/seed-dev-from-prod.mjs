#!/usr/bin/env bun
/**
 * Seed the dev-channel data dir (…/NomiFun/Nomi-dev) from production
 * (…/NomiFun/Nomi), so an auto-isolated dev build can reproduce prod state.
 *
 * Auto-isolation (NOMI_CHANNEL=dev → `Nomi-dev`) gives a dev build its own empty
 * DB. This is the escape hatch for when you need prod's conversations /
 * providers / login in dev to reproduce a bug — it restores the "troubleshoot
 * one place" convenience that channel isolation otherwise trades away.
 *
 * SAFETY: close ALL NomiFun instances (the installed app, `bun run serve:web`,
 * `nomicore`, and any running dev build) before seeding — copying a live SQLite
 * database yields a torn snapshot. Lock and runtime files are never copied.
 *
 * Usage: bun scripts/seed-dev-from-prod.mjs [--force]
 *   --force  overwrite an existing non-empty Nomi-dev (its state is discarded)
 */
import { cpSync, existsSync, readdirSync, rmSync } from 'node:fs';
import { homedir, platform } from 'node:os';
import { basename, join } from 'node:path';

/** Mirror `nomifun_app::cli::default_data_dir`'s vendor base, per-OS. */
function nomifunBase() {
  const home = homedir();
  switch (platform()) {
    case 'darwin':
      return join(home, 'Library', 'Application Support', 'NomiFun');
    case 'win32':
      return join(process.env.LOCALAPPDATA ?? join(home, 'AppData', 'Local'), 'NomiFun');
    default:
      return join(process.env.XDG_DATA_HOME ?? join(home, '.local', 'share'), 'NomiFun');
  }
}

// Lock + runtime artifacts that must never be copied (mirrors relocate.rs's
// EXCLUDED_ENTRIES intent: the lock lives on the handle, not the file).
const EXCLUDED = new Set(['server.lock', 'server.lock.info', 'port.json', '.relocating.lock', '.relocating']);

const force = process.argv.includes('--force');
const base = nomifunBase();
const prod = join(base, 'Nomi');
const dev = join(base, 'Nomi-dev');

if (!existsSync(prod)) {
  console.error(`✗ prod data dir not found: ${prod}`);
  console.error('  Nothing to seed from — launch the installed app once to create it.');
  process.exit(1);
}

if (existsSync(dev) && readdirSync(dev).length > 0) {
  if (!force) {
    console.error(`✗ dev data dir already exists and is non-empty: ${dev}`);
    console.error('  Re-run with --force to overwrite it (the current dev state is discarded).');
    process.exit(1);
  }
  console.warn(`! --force: removing existing dev data dir ${dev}`);
  rmSync(dev, { recursive: true, force: true });
}

console.log('Seeding dev from prod:');
console.log(`    ${prod}`);
console.log(`  → ${dev}`);
console.log('  Ensure ALL NomiFun instances are closed; lock/runtime files are skipped.');

cpSync(prod, dev, {
  recursive: true,
  filter: (src) => !EXCLUDED.has(basename(src)),
});

console.log('✓ done. Run `bun run dev` to launch the dev build on the seeded state.');
