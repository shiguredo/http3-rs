// Chromium / WebKit との WebTransport 相互運用テスト
//
// 検証対象のサーバーを起動し、検証ページを HTTPS で配信し、Playwright で
// Chromium と WebKit を順に駆動して、ブラウザが公開している WebTransport の
// 機能が動作することを確認する。
//
// 使い方:
//   npm ci
//   npx playwright install chromium webkit
//   npm test
//
// 環境変数:
//   WT_BROWSER_ENGINES  検証するエンジン (カンマ区切り。既定は chromium,webkit)
//   WT_SERVER_BIN       検証対象のサーバーバイナリ (既定は target/debug/wt_server)
//   WT_PORT             検証対象のサーバーのポート (既定は 4443)
//   WT_PAGE_PORT        検証ページのポート (既定は 0 = 自動割り当て)
//   WT_FORCE             1 のとき、Playwright が無くても失敗させる (既定は skip)

import { spawn } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { createPageServer } from './serve.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, '..', '..');
const require = createRequire(import.meta.url);

// 検証対象のサーバーが待ち受けるポート
const WT_PORT = Number(process.env.WT_PORT || 4443);

// Playwright が利用できるかを確認する
//
// 導入されていない環境ではテストを skip する。CI では必ず導入するため、
// skip はローカル開発時の利便のための挙動である。
function loadPlaywright() {
  try {
    return require('playwright');
  } catch {
    return null;
  }
}

function browserCacheDir() {
  const home = process.env.HOME || '';
  if (process.platform === 'darwin') {
    return join(home, 'Library', 'Caches', 'ms-playwright');
  }
  return join(home, '.cache', 'ms-playwright');
}

function hasBrowsers() {
  return existsSync(browserCacheDir());
}

// 検証対象のサーバーを起動し、証明書ハッシュがログに出るまで待つ
function startWtServer(serverBin, pagePort) {
  return new Promise((resolvePromise, rejectPromise) => {
    // ブラウザは必ず Origin を送るため、検証ページの Origin を許可する
    // (draft-ietf-webtrans-http3-16 Section 3.2)
    const child = spawn(
      serverBin,
      [
        '--listen',
        `127.0.0.1:${WT_PORT}`,
        '--allow-origin',
        `https://127.0.0.1:${pagePort}`,
      ],
      { cwd: repoRoot, env: { ...process.env, RUST_LOG: 'info' }, stdio: ['ignore', 'pipe', 'pipe'] },
    );

    let output = '';
    let settled = false;
    const timer = setTimeout(() => {
      if (!settled) {
        settled = true;
        child.kill('SIGTERM');
        rejectPromise(new Error(`サーバーの起動がタイムアウトした:\n${output}`));
      }
    }, 30000);

    const onData = (chunk) => {
      output += chunk.toString();
      // サーバーは起動時に証明書ハッシュを base64 で出力する
      const match = output.match(/Certificate hash \(SHA-256, base64\): ([A-Za-z0-9+/=]+)/);
      if (match && !settled) {
        // 証明書の準備ができてもバインドに失敗することがある
        // (別のプロセスがポートを占有している等)。起動ログで確定させる
        if (output.includes('WebTransport server listening on')) {
          settled = true;
          clearTimeout(timer);
          resolvePromise({ child, certificateHash: match[1], getOutput: () => output });
        } else if (output.includes('Server error:')) {
          settled = true;
          clearTimeout(timer);
          child.kill('SIGTERM');
          rejectPromise(new Error(`サーバーの起動に失敗した:\n${output}`));
        }
      }
    };

    child.stdout.on('data', onData);
    child.stderr.on('data', onData);
    child.on('exit', (code) => {
      if (!settled) {
        settled = true;
        clearTimeout(timer);
        rejectPromise(new Error(`サーバーが起動前に終了した (code=${code}):\n${output}`));
      }
    });
  });
}

// 1 エンジン分の検証を実行する
async function runEngine(playwright, engineName, config, pagePort) {
  const engine = playwright[engineName];
  if (!engine) {
    throw new Error(`未知のエンジン: ${engineName}`);
  }

  // Chromium はオリジンごとに証明書検証の結果をプロファイルへキャッシュするため、
  // 毎回新しいプロファイルで起動する。Playwright は launch() に --user-data-dir を
  // 渡すことを許さないので、永続コンテキストとして起動する。
  const profileDir = join(here, '.profile', `${engineName}-${Date.now()}`);
  const launchOptions = { headless: true, ignoreHTTPSErrors: true };

  const context =
    engineName === 'chromium'
      ? await engine.launchPersistentContext(profileDir, launchOptions)
      : await (await engine.launch(launchOptions)).newContext({ ignoreHTTPSErrors: true });

  try {
    const page = await context.newPage();

    const results = [];
    page.on('console', (msg) => {
      const text = msg.text();
      if (text.startsWith('RESULT') || text.startsWith('INFO')) {
        results.push(text);
      }
    });
    page.on('pageerror', (err) => {
      results.push(`RESULT FAIL harness pageerror: ${err.message}`);
    });

    await page.addInitScript((cfg) => {
      window.WT_CONFIG = cfg;
    }, config);

    await page.goto(`https://127.0.0.1:${pagePort}/`, { waitUntil: 'load', timeout: 20000 });

    // 全項目の完了 (DONE) を待つ
    await page.waitForFunction(
      () => document.getElementById('log').textContent.includes('DONE'),
      undefined,
      { timeout: 120000 },
    );

    return results;
  } finally {
    await context.close();
  }
}

async function main() {
  const playwright = loadPlaywright();
  if (!playwright || !hasBrowsers()) {
    const reason = !playwright ? 'playwright が未導入' : `ブラウザが未導入 (${browserCacheDir()})`;
    if (process.env.WT_FORCE === '1') {
      console.error(`NG ${reason}。WT_FORCE=1 のため失敗として扱う`);
      process.exit(1);
    }
    console.log(`SKIP ${reason}`);
    console.log('実行するには npm ci && npx playwright install chromium webkit');
    process.exit(0);
  }

  const serverBin = process.env.WT_SERVER_BIN || join(repoRoot, 'target', 'debug', 'wt_server');
  if (!existsSync(serverBin)) {
    console.error(`NG 検証対象のサーバーが無い: ${serverBin}`);
    console.error('先に cargo build --manifest-path examples/wt_server/Cargo.toml を実行すること');
    process.exit(1);
  }

  const key = readFileSync(join(here, 'certs', 'page-key.pem'));
  const cert = readFileSync(join(here, 'certs', 'page-cert.pem'));
  // 検証ページのポートは自動割り当てにする (固定ポートの衝突を避ける)
  const pageServer = await createPageServer(Number(process.env.WT_PAGE_PORT || 0), here, key, cert);
  const pagePort = pageServer.address().port;

  const engines = (process.env.WT_BROWSER_ENGINES || 'chromium,webkit')
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean);

  let server;
  let failed = false;
  const summary = [];

  try {
    server = await startWtServer(serverBin, pagePort);

    for (const engineName of engines) {
      const config = {
        url: `https://127.0.0.1:${WT_PORT}/wt`,
        certificateHash: server.certificateHash,
      };

      let results;
      try {
        results = await runEngine(playwright, engineName, config, pagePort);
      } catch (e) {
        failed = true;
        summary.push({ engine: engineName, result: `NG 実行エラー: ${e.message}` });
        continue;
      }

      const lines = results.filter((r) => r.startsWith('RESULT'));
      const failures = lines.filter((r) => r.startsWith('RESULT FAIL'));
      const passes = lines.filter((r) => r.startsWith('RESULT PASS'));

      console.log(`=== ${engineName} ===`);
      for (const line of lines) {
        console.log(`  ${line}`);
      }

      if (failures.length > 0 || passes.length === 0) {
        failed = true;
        summary.push({ engine: engineName, result: `NG ${failures.length} 件失敗` });
      } else {
        summary.push({ engine: engineName, result: `OK ${passes.length} 件成功` });
      }
    }
  } finally {
    if (server) {
      server.child.kill('SIGTERM');
    }
    pageServer.close();
  }

  console.log('=== 結果 ===');
  for (const line of summary) {
    console.log(`  ${line.engine}: ${line.result}`);
  }

  process.exit(failed ? 1 : 0);
}

main().catch((e) => {
  console.error(`NG 想定外のエラー: ${e.stack || e.message}`);
  process.exit(1);
});
