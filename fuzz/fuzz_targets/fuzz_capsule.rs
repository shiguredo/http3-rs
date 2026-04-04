#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use shiguredo_http3::webtransport::Capsule;

#[derive(Debug, Arbitrary)]
enum FuzzInput {
    // 任意のバイト列でのデコードテスト
    RawBytes(Vec<u8>),
    // 構造化された入力でのラウンドトリップテスト
    CloseSession { error_code: u32, message: String },
    DrainSession,
    MaxData { maximum: u32 },
    MaxStreamsBidi { maximum: u32 },
    MaxStreamsUni { maximum: u32 },
    DataBlocked { maximum: u32 },
    StreamsBlockedBidi { maximum: u32 },
    StreamsBlockedUni { maximum: u32 },
}

fuzz_target!(|input: FuzzInput| {
    match input {
        FuzzInput::RawBytes(data) => {
            // デコードテスト: 任意のバイト列に対してパニックしないことを検証
            let _ = Capsule::decode(&data);

            // ラウンドトリップテスト
            if let Ok(Some((capsule, _consumed))) = Capsule::decode(&data) {
                // エンコード
                let mut buf = Vec::new();
                capsule.encode(&mut buf);

                // 再デコード
                if let Ok(Some((decoded_capsule, decoded_consumed))) = Capsule::decode(&buf) {
                    // Capsule が一致することを確認
                    assert_eq!(capsule, decoded_capsule);
                    assert_eq!(buf.len(), decoded_consumed);
                }
            }
        }
        _ => {
            // 構造化された入力でのラウンドトリップテスト
            let capsule = match input {
                FuzzInput::CloseSession {
                    error_code,
                    message,
                } => {
                    // メッセージを 1024 バイト以下に制限
                    let truncated_message: String = message.chars().take(1024).collect();
                    Capsule::CloseSession {
                        error_code,
                        message: truncated_message,
                    }
                }
                FuzzInput::DrainSession => Capsule::DrainSession,
                FuzzInput::MaxData { maximum } => Capsule::MaxData {
                    maximum: maximum as u64,
                },
                FuzzInput::MaxStreamsBidi { maximum } => Capsule::MaxStreams {
                    bidirectional: true,
                    maximum: maximum as u64,
                },
                FuzzInput::MaxStreamsUni { maximum } => Capsule::MaxStreams {
                    bidirectional: false,
                    maximum: maximum as u64,
                },
                FuzzInput::DataBlocked { maximum } => Capsule::DataBlocked {
                    maximum: maximum as u64,
                },
                FuzzInput::StreamsBlockedBidi { maximum } => Capsule::StreamsBlocked {
                    bidirectional: true,
                    maximum: maximum as u64,
                },
                FuzzInput::StreamsBlockedUni { maximum } => Capsule::StreamsBlocked {
                    bidirectional: false,
                    maximum: maximum as u64,
                },
                FuzzInput::RawBytes(_) => unreachable!(),
            };

            // エンコード
            let mut buf = Vec::new();
            capsule.encode(&mut buf);

            // デコード
            if let Ok(Some((decoded_capsule, decoded_consumed))) = Capsule::decode(&buf) {
                assert_eq!(capsule, decoded_capsule);
                assert_eq!(buf.len(), decoded_consumed);
            }
        }
    }
});
