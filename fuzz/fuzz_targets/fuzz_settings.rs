#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use shiguredo_http3::{Setting, Settings, SettingsPayload, VarInt};

/// fuzz 入力
///
/// `Setting::from_wire` で `Err` が返るペアも意図的にスキップせず、`from_wire` 自体の
/// panic 安全性を検証する。`from_wire` が `Ok` を返したものは `SettingsPayload::add`
/// に流し込み、重複時のエラーは握り潰して `Settings::from_payload` の panic 安全性を
/// 検証する。
#[derive(Debug, Arbitrary)]
struct FuzzInput {
    entries: Vec<(u64, u64)>,
}

fuzz_target!(|input: FuzzInput| {
    let mut payload = SettingsPayload::new();
    for (id, value) in input.entries {
        let Ok(id_v) = VarInt::new(id) else { continue };
        let Ok(value_v) = VarInt::new(value) else {
            continue;
        };
        // from_wire 自体の panic 安全性も同時に検査
        match Setting::from_wire(id_v, value_v) {
            Ok(setting) => {
                // 重複は SettingsPayload::add が拒否するが、panic させない
                let _ = payload.add(setting);
            }
            Err(e) => {
                // SettingError の Display も panic させない検査
                let _ = format!("{e}");
            }
        }
    }
    let _ = Settings::from_payload(&payload);
});
