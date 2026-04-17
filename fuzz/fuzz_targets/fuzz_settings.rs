#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use shiguredo_http3::{Settings, SettingsPayload};

#[derive(Debug, Arbitrary)]
enum FuzzInput {
    /// 任意のペイロードから Settings をパース
    FromPayload { entries: Vec<(u64, u64)> },
    /// Settings のラウンドトリップ
    Roundtrip {
        qpack_max_table_capacity: Option<u64>,
        max_field_section_size: Option<u64>,
        qpack_blocked_streams: Option<u64>,
        enable_connect_protocol: Option<bool>,
        h3_datagram: Option<bool>,
    },
}

fuzz_target!(|input: FuzzInput| {
    match input {
        FuzzInput::FromPayload { entries } => {
            let mut payload = SettingsPayload::new();
            for (id, value) in entries {
                // HTTP/2 専用設定 ID (0x02-0x05) を除外
                if matches!(id, 0x02..=0x05) {
                    continue;
                }
                payload.add(id, value);
            }
            // 任意のペイロードに対してパニックしないことを検証
            let _ = Settings::from_payload(&payload);
        }
        FuzzInput::Roundtrip {
            qpack_max_table_capacity,
            max_field_section_size,
            qpack_blocked_streams,
            enable_connect_protocol,
            h3_datagram,
        } => {
            // ビルダーで Settings を構築
            let mut settings = Settings::new();
            if let Some(v) = qpack_max_table_capacity {
                settings = settings.qpack_max_table_capacity(v);
            }
            if let Some(v) = max_field_section_size {
                settings = settings.max_field_section_size(v);
            }
            if let Some(v) = qpack_blocked_streams {
                settings = settings.qpack_blocked_streams(v);
            }
            if let Some(v) = enable_connect_protocol {
                settings = settings.enable_connect_protocol(v);
            }
            if let Some(v) = h3_datagram {
                settings = settings.h3_datagram(v);
            }

            // iter() でエントリを取得し SettingsPayload を構築
            let mut payload = SettingsPayload::new();
            for (id, value) in settings.iter() {
                payload.add(id, value);
            }

            // from_payload でラウンドトリップ
            let decoded = Settings::from_payload(&payload).expect("roundtrip must succeed");

            // 基本設定がラウンドトリップで一致することを検証
            assert_eq!(settings.qpack_max_table_capacity, decoded.qpack_max_table_capacity);
            assert_eq!(settings.max_field_section_size, decoded.max_field_section_size);
            assert_eq!(settings.qpack_blocked_streams, decoded.qpack_blocked_streams);
            assert_eq!(settings.enable_connect_protocol, decoded.enable_connect_protocol);
            assert_eq!(settings.h3_datagram, decoded.h3_datagram);
        }
    }
});
