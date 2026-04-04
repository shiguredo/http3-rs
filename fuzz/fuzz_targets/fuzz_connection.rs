#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use shiguredo_http3::{Connection, Settings};

/// 単方向ストリーム ID を生成 (クライアント開始: base * 4 + 2)
fn uni_stream_id(base: u8) -> u64 {
    (base as u64) * 4 + 2
}

/// 双方向ストリーム ID を生成 (クライアント開始: base * 4)
fn bidi_stream_id(base: u8) -> u64 {
    (base as u64) * 4
}

#[derive(Debug, Arbitrary)]
enum FuzzInput {
    /// 単方向ストリームに任意データ
    UniStream {
        stream_id_base: u8,
        data: Vec<u8>,
        fin: bool,
    },
    /// 双方向ストリームに任意データ
    BidiStream {
        stream_id_base: u8,
        data: Vec<u8>,
        fin: bool,
    },
    /// 複数ストリームに連続データ
    MultipleFeeds {
        feeds: Vec<(u8, Vec<u8>, bool)>,
    },
}

fuzz_target!(|input: FuzzInput| {
    let mut conn = Connection::server(Settings::default());

    match input {
        FuzzInput::UniStream {
            stream_id_base,
            data,
            fin,
        } => {
            let stream_id = uni_stream_id(stream_id_base);
            let _ = conn.feed_stream(stream_id, &data, fin);
            while let Ok(Some(_)) = conn.poll_event() {}
        }
        FuzzInput::BidiStream {
            stream_id_base,
            data,
            fin,
        } => {
            let stream_id = bidi_stream_id(stream_id_base);
            let _ = conn.feed_stream(stream_id, &data, fin);
            while let Ok(Some(_)) = conn.poll_event() {}
        }
        FuzzInput::MultipleFeeds { feeds } => {
            for (base, data, fin) in feeds {
                // 偶数はユニ、奇数はバイディ
                let stream_id = if base % 2 == 0 {
                    uni_stream_id(base)
                } else {
                    bidi_stream_id(base)
                };
                let _ = conn.feed_stream(stream_id, &data, fin);
            }
            while let Ok(Some(_)) = conn.poll_event() {}
        }
    }
});
