#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use shiguredo_http3::{Settings, SettingsPayload};

#[derive(Debug, Arbitrary)]
struct FuzzInput {
    entries: Vec<(u64, u64)>,
}

fuzz_target!(|input: FuzzInput| {
    let mut payload = SettingsPayload::new();
    for (id, value) in input.entries {
        // HTTP/2 専用設定 ID (0x02-0x05) を除外
        if matches!(id, 0x02..=0x05) {
            continue;
        }
        payload.add(id, value);
    }
    // 任意のペイロードに対してパニックしないことを検証
    let _ = Settings::from_payload(&payload);
});
