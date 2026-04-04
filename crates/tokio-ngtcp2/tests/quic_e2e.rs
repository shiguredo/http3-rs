//! QUIC クライアント/サーバー I/O テスト
//!
//! 実際のネットワーク I/O を使用した QUIC ハンドシェイクテスト

use std::path::PathBuf;
use std::time::Duration;

use rcgen::{CertificateParams, KeyPair};
use tokio::time::timeout;

use tokio_ngtcp2::{Client, Server};

/// テスト用の証明書と秘密鍵を動的に生成
fn generate_test_certs() -> (PathBuf, PathBuf) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique_id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let temp_dir =
        std::env::temp_dir().join(format!("quic_test_{}_{}", std::process::id(), unique_id));
    std::fs::create_dir_all(&temp_dir).expect("一時ディレクトリ作成失敗");

    let cert_path = temp_dir.join("cert.pem");
    let key_path = temp_dir.join("key.pem");

    // 証明書パラメータを設定
    let mut params =
        CertificateParams::new(vec!["localhost".to_string()]).expect("CertificateParams 作成失敗");
    params.distinguished_name.push(
        rcgen::DnType::CommonName,
        rcgen::DnValue::Utf8String("localhost".to_string()),
    );

    // 鍵ペアを生成
    let key_pair = KeyPair::generate().expect("鍵ペア生成失敗");

    // 自己署名証明書を生成
    let cert = params.self_signed(&key_pair).expect("証明書生成失敗");

    // PEM 形式で保存
    std::fs::write(&cert_path, cert.pem()).expect("証明書ファイル書き込み失敗");
    std::fs::write(&key_path, key_pair.serialize_pem()).expect("秘密鍵ファイル書き込み失敗");

    (cert_path, key_path)
}

/// QUIC ハンドシェイクテスト (証明書検証なし)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_quic_handshake_insecure() {
    let (cert_path, key_path) = generate_test_certs();

    // サーバーを起動
    let mut server = Server::bind(
        "127.0.0.1:0".parse().unwrap(),
        &cert_path,
        &key_path,
        None,
        None,
    )
    .await
    .expect("サーバー作成失敗");

    let server_addr = server.local_addr();
    eprintln!("[test] サーバー起動: {}", server_addr);

    // サーバータスク
    let server_task = tokio::spawn(async move {
        let result = timeout(
            Duration::from_secs(10),
            server.run(|addr, event| {
                eprintln!(
                    "[server] イベント受信: addr = {}, event = {:?}",
                    addr, event
                );
                None
            }),
        )
        .await;

        match result {
            Ok(r) => r,
            Err(_) => {
                eprintln!("[server] タイムアウト");
                Ok(())
            }
        }
    });

    // クライアントを作成 (証明書検証なし)
    let client_result = timeout(Duration::from_secs(10), async {
        let mut client = Client::connect_insecure_default(server_addr, "localhost")
            .await
            .expect("クライアント作成失敗");

        eprintln!("[client] ハンドシェイク開始");

        // ハンドシェイクを実行
        match client.handshake().await {
            Ok(()) => {
                eprintln!("[client] ハンドシェイク成功");
                true
            }
            Err(e) => {
                eprintln!("[client] ハンドシェイクエラー: {:?}", e);
                false
            }
        }
    })
    .await;

    server_task.abort();

    match client_result {
        Ok(success) => {
            assert!(success, "ハンドシェイクが成功するべき");
            eprintln!("[test] QUIC ハンドシェイクテスト成功");
        }
        Err(_) => {
            panic!("テストタイムアウト");
        }
    }
}

/// 複数クライアントの同時接続テスト
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_quic_multiple_clients() {
    let (cert_path, key_path) = generate_test_certs();

    // サーバーを起動
    let mut server = Server::bind(
        "127.0.0.1:0".parse().unwrap(),
        &cert_path,
        &key_path,
        None,
        None,
    )
    .await
    .expect("サーバー作成失敗");

    let server_addr = server.local_addr();
    eprintln!("[test] サーバー起動: {}", server_addr);

    // サーバータスク
    let server_task = tokio::spawn(async move {
        let result = timeout(
            Duration::from_secs(10),
            server.run(|addr, event| {
                eprintln!(
                    "[server] イベント受信: addr = {}, event = {:?}",
                    addr, event
                );
                None
            }),
        )
        .await;

        match result {
            Ok(r) => r,
            Err(_) => Ok(()),
        }
    });

    // 複数クライアントを並行して接続
    let client_count = 3;
    let mut handles = Vec::new();

    for i in 0..client_count {
        let addr = server_addr;
        let handle = tokio::spawn(async move {
            let mut client = Client::connect_insecure_default(addr, "localhost")
                .await
                .expect("クライアント作成失敗");

            eprintln!("[client {}] ハンドシェイク開始", i);

            match timeout(Duration::from_secs(5), client.handshake()).await {
                Ok(Ok(())) => {
                    eprintln!("[client {}] ハンドシェイク成功", i);
                    true
                }
                Ok(Err(e)) => {
                    eprintln!("[client {}] ハンドシェイクエラー: {:?}", i, e);
                    false
                }
                Err(_) => {
                    eprintln!("[client {}] タイムアウト", i);
                    false
                }
            }
        });
        handles.push(handle);
    }

    // 全クライアントの結果を待つ
    let mut success_count = 0;
    for handle in handles {
        if handle.await.unwrap_or(false) {
            success_count += 1;
        }
    }

    server_task.abort();

    eprintln!(
        "[test] 成功したクライアント: {}/{}",
        success_count, client_count
    );
    assert!(
        success_count >= 1,
        "少なくとも 1 クライアントが成功するべき"
    );
}

/// クライアント/サーバーの正常終了テスト
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn test_quic_connection_close() {
    let (cert_path, key_path) = generate_test_certs();

    // サーバーを起動
    let mut server = Server::bind(
        "127.0.0.1:0".parse().unwrap(),
        &cert_path,
        &key_path,
        None,
        None,
    )
    .await
    .expect("サーバー作成失敗");

    let server_addr = server.local_addr();

    // サーバータスク
    let server_task = tokio::spawn(async move {
        let _ = timeout(Duration::from_secs(5), server.run(|_addr, _event| None)).await;
    });

    // クライアントを作成してハンドシェイク
    let mut client = Client::connect_insecure_default(server_addr, "localhost")
        .await
        .expect("クライアント作成失敗");

    // ハンドシェイク
    let handshake_result = timeout(Duration::from_secs(5), client.handshake()).await;
    assert!(
        handshake_result.is_ok(),
        "ハンドシェイクがタイムアウトしないこと"
    );

    // クライアントをドロップ (接続クローズ)
    drop(client);

    // サーバーを終了
    server_task.abort();

    eprintln!("[test] 接続クローズテスト完了");
}
