#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use shiguredo_http3::webtransport::{Capsule, Session, Stream};

#[derive(Debug, Arbitrary)]
enum SessionOp {
    /// ストリームを追加
    AddStream { stream_id: u64, bidi: bool },
    /// ストリームを削除
    RemoveStream { stream_id: u64 },
    /// Capsule を処理
    ProcessCapsule { capsule_data: Vec<u8> },
    /// ドレイン
    Drain,
    /// クローズ
    Close,
    /// データグラムをバッファリング
    BufferDatagram { data: Vec<u8> },
    /// ストリームをバッファリング
    BufferStream { stream_id: u64, bidi: bool },
}

#[derive(Debug, Arbitrary)]
enum FuzzInput {
    /// 任意 Capsule のデコード + process
    RawCapsule { data: Vec<u8> },
    /// 構造化された操作列
    Operations { ops: Vec<SessionOp> },
}

fuzz_target!(|input: FuzzInput| {
    match input {
        FuzzInput::RawCapsule { data } => {
            let mut session = Session::new(0);
            session.set_established();
            if let Ok(Some((capsule, _))) = Capsule::decode(&data) {
                let _ = session.process_capsule(&capsule);
            }
        }
        FuzzInput::Operations { ops } => {
            let mut session = Session::new(0);
            session.set_established();

            for op in ops {
                match op {
                    SessionOp::AddStream { stream_id, bidi } => {
                        session.add_stream(Stream::new(stream_id, 0, bidi));
                    }
                    SessionOp::RemoveStream { stream_id } => {
                        let _ = session.remove_stream(stream_id);
                    }
                    SessionOp::ProcessCapsule { capsule_data } => {
                        if let Ok(Some((capsule, _))) = Capsule::decode(&capsule_data) {
                            let _ = session.process_capsule(&capsule);
                        }
                    }
                    SessionOp::Drain => {
                        session.drain();
                    }
                    SessionOp::Close => {
                        session.close(None);
                    }
                    SessionOp::BufferDatagram { data } => {
                        let _ = session.buffer_datagram(data);
                    }
                    SessionOp::BufferStream { stream_id, bidi } => {
                        let _ = session.buffer_incoming_stream(stream_id, bidi);
                    }
                }
            }

            // 最後にペンディング Capsule を取り出す
            let _ = session.take_pending_capsules();
            let _ = session.take_buffered_streams();
            let _ = session.take_buffered_datagrams();
        }
    }
});
