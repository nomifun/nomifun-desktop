import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'bun:test';

const manifest = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'));

describe('release installation upgrade gate', () => {
  test('runs the locked startup integration suite serially', () => {
    expect(manifest.scripts['test:upgrade']).toBe(
      'cargo test --locked -p nomifun-app --test startup_smoke --no-default-features -- --test-threads=1',
    );
  });

  for (const [platform, file, dryRunPattern, bumpEndPattern, gatePattern, build] of [
    ['macOS', './release-mac.sh', /^if \[\[ "\$DryRun" -eq 1 \]\]; then\n[\s\S]*?\nfi/m, /\[\[ "\$CurVer" == "\$TargetVersion" \]\] \|\| fail [^\n]+\nfi/, /^bun run test:upgrade \|\| fail "安装升级回归测试失败，停止发布。"$/m, 'bun run build:mac --signed'],
    ['Linux', './release-linux.sh', /^if \[\[ "\$DryRun" -eq 1 \]\]; then\n[\s\S]*?\nfi/m, /\[\[ "\$CurVer" == "\$TargetVersion" \]\] \|\| fail [^\n]+\nfi/, /^bun run test:upgrade \|\| fail "安装升级回归测试失败，停止发布。"$/m, 'bash "$BuildLinuxScript" "${BuildArgs[@]}"'],
    ['Windows', './release-win.ps1', /if \(\$DryRun\) \{\n[\s\S]*?\n\}/, /if \(\$CurVer -ne \$TargetVersion\) \{ Fail [^\n]+\n\}/, /^& bun run test:upgrade\nif \(\$LASTEXITCODE -ne 0\) \{ Fail "安装升级回归测试失败，停止发布。" \}$/m, '& bun run build:win'],
  ]) {
    test(`${platform} skips the gate during dry runs and blocks packaging on failure`, () => {
      const source = readFileSync(new URL(file, import.meta.url), 'utf8').replaceAll('\r\n', '\n');
      const dryRun = source.match(dryRunPattern);
      const bumpEnd = source.match(bumpEndPattern);
      const gate = source.match(gatePattern);
      expect(dryRun).not.toBeNull();
      expect(dryRun[0]).toContain('exit 0');
      expect(dryRun[0]).not.toContain('test:upgrade');
      expect(bumpEnd).not.toBeNull();
      expect(gate).not.toBeNull();
      expect(source.match(/^\s*(?:& )?bun run test:upgrade\b/gm)).toHaveLength(1);
      expect(bumpEnd.index).toBeGreaterThan(dryRun.index + dryRun[0].length);
      expect(gate.index).toBeGreaterThan(bumpEnd.index + bumpEnd[0].length);
      expect(source.indexOf(build)).toBeGreaterThan(gate.index + gate[0].length);
    });
  }
});
