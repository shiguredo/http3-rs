//! 内部モジュール (pub(crate))

pub(crate) mod connection_state;

/// ストリームレベルのエラーを RESET_STREAM でピアに伝える
///
/// `StreamError` はストリーム単位のエラーであり、接続を維持したまま該当
/// ストリームを RESET_STREAM で閉じる必要がある (RFC 9114 Section 8 /
/// draft-ietf-webtrans-http3-16 Section 6)。それ以外のエラー (接続エラー等) は
/// 変換しない。
///
/// RESET_STREAM の送信失敗は無視する (ストリームが既に閉じている等のため)。
pub(crate) fn reset_stream_on_stream_error(
    send_stream: &mut s2n_quic::stream::SendStream,
    err: &crate::Error,
) {
    if let crate::Error::Http3(shiguredo_http3::Error::StreamError(code)) = err {
        // H3 エラーコードは全て 2^62 未満 (RFC 9114 Section 8.1 の Error Code
        // registry は 62-bit space / RFC 9000 Section 16 の VarInt 値域内) のため
        // application::Error::new は常に成功する
        let error = s2n_quic::application::Error::new(code.code())
            .expect("H3 error code fits in VarInt range");
        let _ = send_stream.reset(error);
    }
}
