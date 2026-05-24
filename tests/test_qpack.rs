use shiguredo_http3::qpack::huffman;

#[test]
fn decode_eos_only_returns_error() {
    // EOS のみ: 30 ビット EOS (全て 1) + 2 ビットパディング (11)
    let data = [0xff, 0xff, 0xff, 0xff];
    assert!(huffman::decode(&data).is_err());
}

#[test]
fn decode_valid_symbol_then_eos_returns_error() {
    // 'a' (5 ビット) + EOS (30 ビット) + パディング (5 ビット, 全て 1)
    let data = [0x1f, 0xff, 0xff, 0xff, 0xff];
    assert!(huffman::decode(&data).is_err());
}
