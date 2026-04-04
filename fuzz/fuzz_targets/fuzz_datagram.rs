#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use shiguredo_http3::webtransport::Datagram;

#[derive(Debug, Arbitrary)]
enum FuzzInput {
    /// 任意バイト列のデコード
    RawDecode(Vec<u8>),
    /// 構造化入力のラウンドトリップ
    Roundtrip {
        session_id_quarter: u32,
        payload: Vec<u8>,
    },
}

fuzz_target!(|input: FuzzInput| {
    match input {
        FuzzInput::RawDecode(data) => {
            // 任意バイト列に対してパニックしないことを検証
            let _ = Datagram::decode(&data);

            // デコード成功時はラウンドトリップを検証
            if let Some((datagram, consumed)) = Datagram::decode(&data) {
                assert_eq!(consumed, data.len());

                let mut buf = Vec::new();
                datagram.encode(&mut buf);

                if let Some((decoded, decoded_consumed)) = Datagram::decode(&buf) {
                    assert_eq!(decoded_consumed, buf.len());
                    assert_eq!(datagram, decoded);
                }
            }
        }
        FuzzInput::Roundtrip {
            session_id_quarter,
            payload,
        } => {
            // session_id は 4 の倍数でなければならない
            let session_id = (session_id_quarter as u64) * 4;

            // ペイロードサイズを制限
            let payload = if payload.len() > 4096 {
                payload[..4096].to_vec()
            } else {
                payload
            };

            let datagram = Datagram::new(session_id, payload).unwrap();

            // エンコード
            let mut buf = Vec::new();
            datagram.encode(&mut buf);

            // デコードしてラウンドトリップを検証
            let (decoded, consumed) = Datagram::decode(&buf).expect("エンコードしたデータは必ずデコードできる");
            assert_eq!(consumed, buf.len());
            assert_eq!(datagram.session_id, decoded.session_id);
            assert_eq!(datagram.payload, decoded.payload);
        }
    }
});
