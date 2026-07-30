#!/usr/bin/env bun
/**
 * Seed the dev-channel data dir (…/NomiFun-dev) from production
 * (…/NomiFun), so an auto-isolated dev build can reproduce prod state.
 *
 * Auto-isolation (NOMI_CHANNEL=dev → the `NomiFun-dev` sibling dir) gives a
 * dev build its own empty DB. This is the escape hatch for when you need
 * prod's conversations / providers / login in dev to reproduce a bug — it
 * restores the "troubleshoot one place" convenience that channel isolation
 * otherwise trades away.
 *
 * Legacy layouts (pre data-root move): prod used to live in `NomiFun/Nomi`
 * and dev in `NomiFun/Nomi-dev`. If the new prod root has no database yet
 * (the app has not migrated it), the legacy prod dir is used as the source.
 *
 * SAFETY: close ALL NomiFun instances (the installed app, `bun run serve:web`,
 * `nomicore`, and any running dev build) before seeding — copying a live SQLite
 * database yields a torn snapshot. Lock and runtime files are never copied.
 *
 * Usage: bun scripts/seed-dev-from-prod.mjs [--force]
 *   --force  overwrite an existing non-empty NomiFun-dev (its state is discarded)
 */
import { cpSync, existsSync, readdirSync, rmSync } from 'node:fs';
import { homedir, platform } from 'node:os';
import { basename, join } from 'node:path';

/** Mirror `nomifun_app::cli::default_data_dir`'s per-user base dir, per-OS. */
function appDataBase() {
  const home = homedir();
  switch (platform()) {
    case 'darwin':
      return join(home, 'Library', 'Application Support');
    case 'win32':
      return process.env.LOCALAPPDATA ?? join(home, 'AppData', 'Local');
    default:
      return process.env.XDG_DATA_HOME ?? join(home, '.local', 'share');
  }
}

// Lock + runtime artifacts that must never be copied (the lock lives on the
// handle, not the file) — including layout-migration control files.
const EXCLUDED = new Set([
  'server.lock',
  'server.lock.info',
  'port.json',
  '.nomifun-work-root.lock',
  '.relocating.lock',
  '.relocating',
  '.relocated-from',
  '.relocated-done',
  '.nomifun-layout-migration.pending',
]);

const force = process.argv.includes('--force');
const base = appDataBase();
const prodCurrent = join(base, 'NomiFun');
const prodLegacy = join(base, 'NomiFun', 'Nomi');
const dev = join(base, 'NomiFun-dev');

// Prefer the current layout; fall back to a not-yet-migrated legacy dataset.
const prod = existsSync(join(prodCurrent, 'nomifun-backend.db'))
  ? prodCurrent
  : existsSync(join(prodLegacy, 'nomifun-backend.db'))
    ? prodLegacy
    : null;

if (!prod) {
  console.error(`✗ prod data dir not found: ${prodCurrent} (nor legacy ${prodLegacy})`);
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
  filter: (src) => {
    if (EXCLUDED.has(basename(src))) return false;
    // When seeding from the current layout, never drag legacy channel
    // subdirectories (NomiFun/Nomi, NomiFun/Nomi-dev) into the dev root.
    if (prod === prodCurrent) {
      const name = basename(src);
      if ((name === 'Nomi' || name.startsWith('Nomi-')) && join(prod, name) === src) {
        return false;
      }
    }
    return true;
  },
});

console.log('✓ done. Run `bun run dev` to launch the dev build on the seeded state.');
