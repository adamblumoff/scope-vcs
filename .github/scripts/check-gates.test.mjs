import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { cpSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import test from 'node:test';
import { classifyChanges } from './plan-production-deployment.mjs';

const root = resolve(import.meta.dirname, '../..');
const read = (path) => readFileSync(resolve(root, path), 'utf8');
const manifest = JSON.parse(read('.github/deployment-services.json'));
const gates = ['backend', 'cli', 'web', 'contract', 'policy', 'integration', 'ops'];

// Capture the commands actually executed, without requiring installed toolchains,
// credentials, or a running stack. The scripts remain the command inventory.
function commands(gate, ...args) {
  const dir = mkdtempSync(resolve(tmpdir(), 'scope-gates-'));
  try {
    for (const tool of ['cargo', 'pnpm', 'node', 'bash']) {
      writeFileSync(resolve(dir, tool), '#!/bin/sh\ncase "$1" in *dev/checks/*) exec /bin/bash "$@" ;; esac\nprintf "%s" "$(basename "$0")"\nprintf " %s" "$@"\nprintf "\\n"\n', { mode: 0o755 });
    }
    return execFileSync('/bin/bash', [resolve(root, `dev/checks/${gate}`), ...args], {
      env: { ...process.env, PATH: `${dir}:${process.env.PATH}` }, encoding: 'utf8',
    }).trim().split('\n');
  } finally { rmSync(dir, { recursive: true, force: true }); }
}

test('backend variants preserve API feature coverage explicitly', () => {
  const withApi = commands('backend', 'with-api');
  const withoutApi = commands('backend', 'without-api');
  assert.ok(withApi.includes('cargo test --workspace --features api/test-support --locked'));
  assert.ok(withApi.includes('cargo test -p api --features local-dev --locked dev::'));
  assert.ok(withoutApi.includes('cargo test --workspace --exclude api --locked'));
  assert.ok(withoutApi.every((line) => !line.includes('--features')));
  assert.equal(spawnSync('dev/checks/backend', ['invalid'], { cwd: root }).status, 2);
});

test('web gate includes contract and observer rules; CLI and integration retain their coverage', () => {
  assert.deepEqual(commands('web'), [
    'pnpm test', 'pnpm check', 'pnpm build',
  ]);
  const webChecks = JSON.parse(read('web/package.json')).scripts.check;
  assert.equal(webChecks, 'pnpm typecheck && ../dev/checks/contract && pnpm check:observer-boundary && pnpm check:react-doctor && pnpm check:konsistent');
  assert.deepEqual(commands('contract'), ['pnpm check:api-contract']);
  assert.ok(commands('cli').includes('cargo build --manifest-path cli/Cargo.toml --release --locked --bin scope --bin scope-cli-service'));
  assert.deepEqual(commands('integration', 'cli'), ['cargo test --manifest-path cli/Cargo.toml --test contribution_flow --locked -- --nocapture']);
  assert.deepEqual(commands('integration', 'web'), ['pnpm test:smoke']);
});

test('local and both CI callers use the shared inventory', () => {
  const github = ['rust-workspace-checks', 'scope-api-ci', 'scope-cli-build', 'scope-web-ci', 'scope-production-deploy', 'scope-integration-ci']
    .map((name) => read(`.github/workflows/${name}.yml`)).join('\n');
  const scope = read('.scope/runs/checks.yml');
  for (const gate of gates.filter((gate) => gate !== 'contract')) {
    assert.ok(github.includes(`dev/checks/${gate}`), `GitHub: ${gate}`);
    assert.ok(scope.includes(`dev/checks/${gate}`), `Scope: ${gate}`);
  }
  assert.ok(read('dev/check').includes('dev/checks/policy'));
  assert.ok(read('web/package.json').includes('dev/checks/contract'));
  assert.ok(github.includes('dev/checks/contract'));
});

test('gate inputs select checks through change scopes', () => {
  const paths = execFileSync('git', ['ls-files', '--cached', '--others', '--exclude-standard'], { cwd: root, encoding: 'utf8' }).trim().split('\n');
  const gateInputs = paths.filter((path) => /^(dev\/|\.github\/scripts\/|bench\/|deploy\/aws\/)/.test(path));
  gateInputs.push('.scope/runs/checks.yml', '.github/source-size-audit.json');
  for (const path of gateInputs) {
    const selected = classifyChanges(manifest, [path]);
    assert.ok(Object.values(selected).some(Boolean), `${path} must select checks`);
  }
});

test('policy rejects oversized non-web source in a complete checkout', () => {
  assert.ok(commands('policy').includes('node .github/scripts/check-source-size.mjs'));
  const dir = mkdtempSync(resolve(tmpdir(), 'scope-size-gate-'));
  try {
    mkdirSync(resolve(dir, '.github/scripts'), { recursive: true });
    mkdirSync(resolve(dir, 'worker/src'), { recursive: true });
    cpSync(resolve(root, '.github/scripts/check-source-size.mjs'), resolve(dir, '.github/scripts/check-source-size.mjs'));
    writeFileSync(resolve(dir, '.github/source-size-audit.json'), JSON.stringify({ version: 1, production: [] }));
    writeFileSync(resolve(dir, 'worker/src/oversized.rs'), '// outside web\n'.repeat(1001));
    execFileSync('git', ['init', '--quiet', dir]);
    const result = spawnSync(process.execPath, ['.github/scripts/check-source-size.mjs'], { cwd: dir, encoding: 'utf8' });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /worker\/src\/oversized.rs/);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});
