#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use shiguredo_http3::stream::RequestStream;
use shiguredo_http3::{Connection, Role, Settings};

#[derive(Debug, Arbitrary)]
enum FuzzInput {
    /// リクエストストリーム (クライアントロール) に任意データを投入し process_raw を呼ぶ
    RequestStreamRecv { chunks: Vec<(Vec<u8>, bool)> },
    /// リクエストストリーム (サーバーロール) に任意データを投入し process_raw を呼ぶ
    ///
    /// サーバーロールでは先頭位置の WT_STREAM (0x41) が無視される分岐を網羅する。
    RequestStreamRecvServer { chunks: Vec<(Vec<u8>, bool)> },
    /// 制御ストリーム受信側に任意データを投入する (Connection 経由)
    ControlStreamRecv { data: Vec<u8> },
    /// リクエストストリームの送信状態遷移
    RequestStreamSend { operations: Vec<SendOp> },
}

#[derive(Debug, Arbitrary)]
enum SendOp {
    /// エンコード済みヘッダーを送信
    SendHeaders {
        data: Vec<u8>,
        fin: bool,
        is_interim: bool,
    },
    /// ボディデータを送信
    SendBody { data: Vec<u8>, fin: bool },
}

fuzz_target!(|input: FuzzInput| {
    match input {
        FuzzInput::RequestStreamRecv { chunks } => {
            let mut stream = RequestStream::new(0, Role::Client);
            for (data, fin) in chunks {
                // データサイズを制限して処理速度を確保
                let data = if data.len() > 4096 {
                    &data[..4096]
                } else {
                    &data
                };
                stream.receive(data, fin);
                // process_raw を繰り返し呼んで全データを消費
                loop {
                    match stream.process_raw() {
                        Ok(Some(_)) => continue,
                        Ok(None) => break,
                        Err(_) => break,
                    }
                }
            }
        }
        FuzzInput::RequestStreamRecvServer { chunks } => {
            let mut stream = RequestStream::new(0, Role::Server);
            for (data, fin) in chunks {
                // データサイズを制限して処理速度を確保
                let data = if data.len() > 4096 {
                    &data[..4096]
                } else {
                    &data
                };
                stream.receive(data, fin);
                // process_raw を繰り返し呼んで全データを消費
                loop {
                    match stream.process_raw() {
                        Ok(Some(_)) => continue,
                        Ok(None) => break,
                        Err(_) => break,
                    }
                }
            }
        }
        FuzzInput::ControlStreamRecv { data } => {
            // Connection 公開 API 経由で制御ストリームにデータを投入
            let mut conn = Connection::server(Settings::default());
            // クライアント開始の単方向ストリーム (stream_id = 2)
            let control_stream_id = 2u64;
            // データサイズを制限
            let data = if data.len() > 4096 {
                &data[..4096]
            } else {
                &data
            };
            let _ = conn.feed_stream(control_stream_id, data, false);
            while let Ok(Some(_)) = conn.poll_event() {}
        }
        FuzzInput::RequestStreamSend { operations } => {
            let mut stream = RequestStream::new(0, Role::Client);
            for op in operations {
                match op {
                    SendOp::SendHeaders {
                        data,
                        fin,
                        is_interim,
                    } => {
                        // データサイズを制限
                        let data = if data.len() > 4096 {
                            &data[..4096]
                        } else {
                            &data
                        };
                        // エラーは無視して状態遷移のパニック安全性を検証
                        let _ = stream.send_encoded_headers(data, fin, is_interim);
                    }
                    SendOp::SendBody { data, fin } => {
                        let data = if data.len() > 4096 {
                            &data[..4096]
                        } else {
                            &data
                        };
                        let _ = stream.send_body(data, fin);
                    }
                }
            }
        }
    }
});
