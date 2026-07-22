use shiguredo_http3::QpackError;
use shiguredo_http3::qpack::{DynamicTable, EncoderStreamReceiver};

#[test]
fn insert_with_name_ref_static_invalid_index_does_not_drain() {
    // 静的テーブルに存在しないインデックス 99 で Insert with Name Reference を送信する
    // QPACK 静的テーブルは 0-98 の 99 エントリのみ
    // 名前参照の段階でエラーになるため消費バイト数が確定せず drain しない
    let mut receiver = EncoderStreamReceiver::new();
    receiver.set_max_table_capacity(4096);
    let mut table = DynamicTable::with_capacity(4096);

    // 名前参照による挿入 (static=1, name_index=99, value=空)
    // 6-bit prefix: 99 >= 63 → 0xFF, 0x24 (99-63=36)
    // 値文字列 (7-bit prefix): 長さ=0 → 0x00
    let data: &[u8] = &[0xFF, 0x24, 0x00];
    receiver.receive(data);

    let result = receiver.process(&mut table);
    assert_eq!(
        result,
        Err(QpackError::InvalidIndex(99)),
        "不正な静的テーブルインデックスでエラーが返ること"
    );
    assert_eq!(
        receiver.buffer(),
        data,
        "名前参照エラー時は消費バイト数が確定しないため drain しないこと"
    );
}

#[test]
fn insert_with_name_ref_dynamic_invalid_index_does_not_drain() {
    // 空の動的テーブルに対して relative_index=5 で Insert with Name Reference を送信する
    // 名前参照の段階でエラーになるため消費バイト数が確定せず drain しない
    let mut receiver = EncoderStreamReceiver::new();
    receiver.set_max_table_capacity(4096);
    let mut table = DynamicTable::with_capacity(4096);

    // 名前参照による挿入 (static=0, name_index=5, value=空)
    // 6-bit prefix: 5 < 63 → 0x80 | 5 = 0x85
    // 値文字列 (7-bit prefix): 長さ=0 → 0x00
    let data: &[u8] = &[0x85, 0x00];
    receiver.receive(data);

    let result = receiver.process(&mut table);
    assert_eq!(
        result,
        Err(QpackError::InvalidIndex(5)),
        "不正な動的テーブル相対インデックスでエラーが返ること"
    );
    assert_eq!(
        receiver.buffer(),
        data,
        "名前参照エラー時は消費バイト数が確定しないため drain しないこと"
    );
}

#[test]
fn insert_with_name_ref_capacity_overflow_drains_on_error() {
    // 容量不足で insert が失敗するケース
    // 静的テーブル index 0 (:authority) のエントリサイズ = 10 + 0 + 32 = 42 > 32
    let mut receiver = EncoderStreamReceiver::new();
    receiver.set_max_table_capacity(32);
    let mut table = DynamicTable::with_capacity(32);

    // 名前参照による挿入 (static=1, name_index=0, value=空)
    // 6-bit prefix: 0 < 63 → 0xC0 | 0 = 0xC0
    // 値文字列 (7-bit prefix): 長さ=0 → 0x00
    let data: &[u8] = &[0xC0, 0x00];
    receiver.receive(data);

    let result = receiver.process(&mut table);
    assert_eq!(
        result,
        Err(QpackError::DecodeFailed),
        "容量超過で挿入が失敗しエラーが返ること"
    );
    // 0120: infinite loop 防止のため、insert 失敗時もバッファを drain する
    assert!(
        receiver.buffer().is_empty(),
        "insert 失敗時にバッファが drain されていること (infinite loop 防止)"
    );
}

#[test]
fn insert_with_literal_name_capacity_overflow_drains_on_error() {
    // 容量不足で insert が失敗するケース
    // エントリサイズ = 1 (name "a") + 0 (empty value) + 32 = 33 > 32
    let mut receiver = EncoderStreamReceiver::new();
    receiver.set_max_table_capacity(32);
    let mut table = DynamicTable::with_capacity(32);

    // リテラル名による挿入 (01 prefix, H=0, name 長さ=1)
    // 5-bit prefix: 1 < 31 → 0x40 | 1 = 0x41
    // name バイト列: 0x61 ('a')
    // 値文字列 (7-bit prefix): 長さ=0 → 0x00
    let data: &[u8] = &[0x41, 0x61, 0x00];
    receiver.receive(data);

    let result = receiver.process(&mut table);
    assert_eq!(
        result,
        Err(QpackError::DecodeFailed),
        "容量超過で挿入が失敗しエラーが返ること"
    );
    // 0120: infinite loop 防止のため、insert 失敗時もバッファを drain する
    assert!(
        receiver.buffer().is_empty(),
        "insert 失敗時にバッファが drain されていること (infinite loop 防止)"
    );
}

#[test]
fn duplicate_invalid_index_drains_on_error() {
    // 空の動的テーブルに対して relative_index=5 で Duplicate を送信する
    let mut receiver = EncoderStreamReceiver::new();
    receiver.set_max_table_capacity(4096);
    let mut table = DynamicTable::with_capacity(4096);

    // 複製命令 (000 prefix, 5-bit prefix: 5 < 31 → 0x00 | 5 = 0x05)
    let data: &[u8] = &[0x05];
    receiver.receive(data);

    let result = receiver.process(&mut table);
    assert_eq!(
        result,
        Err(QpackError::InvalidIndex(5)),
        "不正な相対インデックスでエラーが返ること"
    );
    // 0120: infinite loop 防止のため、duplicate 失敗時もバッファを drain する
    assert!(
        receiver.buffer().is_empty(),
        "duplicate 失敗時にバッファが drain されていること (infinite loop 防止)"
    );
}
