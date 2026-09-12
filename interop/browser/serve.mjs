// 検証ページを HTTPS で配信する
//
// WebTransport は secure context を要求するため、検証ページも HTTPS で配信する
// 必要がある。ここで使う証明書はページ配信専用であり、WebTransport サーバーの
// 証明書とは別物である (接続先の検証はページ側の serverCertificateHashes が担う)。
//
// 使い方: node serve.mjs <port> <dir>

import { createServer } from 'node:https';
import { readFileSync } from 'node:fs';
import { extname, join } from 'node:path';

// ページ配信用の自己署名証明書 (初回起動時に生成する)
export function createPageServer(port, dir, key, cert) {
  const types = {
    '.html': 'text/html; charset=utf-8',
    '.js': 'text/javascript; charset=utf-8',
  };

  const server = createServer({ key, cert }, (req, res) => {
    const path = req.url === '/' ? '/index.html' : req.url;
    try {
      const body = readFileSync(join(dir, path));
      res.writeHead(200, {
        'content-type': types[extname(path)] || 'application/octet-stream',
      });
      res.end(body);
    } catch {
      res.writeHead(404);
      res.end('not found');
    }
  });

  return new Promise((resolve, reject) => {
    server.on('error', reject);
    server.listen(port, '127.0.0.1', () => resolve(server));
  });
}
