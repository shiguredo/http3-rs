//! 内部モジュール (pub(crate))

use bytes::Bytes;

pub(crate) mod connection_state;

/// 単方向ストリーム受信結果に応じた sans-I/O 層への伝達方法
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum UniRecvAction {
    /// データを feed する (FIN なし)
    Data(Bytes),
    /// FIN を伝達する
    Fin,
    /// RESET_STREAM (アプリケーションエラーコード) を伝達する
    Reset(u64),
    /// 何も伝達しない (接続エラー等)
    Ignore,
}

/// 単方向ストリームの受信結果を sans-I/O 層への伝達方法に分類する
///
/// `StreamReset` 以外の `Err` (`ConnectionError` 等の接続エラー) を FIN として
/// 伝達すると、クリティカルストリーム (制御 / QPACK) で
/// `H3_CLOSED_CRITICAL_STREAM` を誤ラッチするため、何も伝達しない
/// (RFC 9114 Section 6.2.1 / RFC 9204 Section 4.2)。
pub(crate) fn classify_uni_recv(
    result: Result<Option<Bytes>, s2n_quic::stream::Error>,
) -> UniRecvAction {
    match result {
        Ok(Some(data)) => UniRecvAction::Data(data),
        Ok(None) => UniRecvAction::Fin,
        Err(s2n_quic::stream::Error::StreamReset { error, .. }) => UniRecvAction::Reset(*error),
        Err(_) => UniRecvAction::Ignore,
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_uni_recv_data_and_fin() {
        // Ok(Some) はデータ、Ok(None) は FIN として伝達する
        assert_eq!(
            classify_uni_recv(Ok(Some(Bytes::from_static(b"abc")))),
            UniRecvAction::Data(Bytes::from_static(b"abc"))
        );
        assert_eq!(classify_uni_recv(Ok(None)), UniRecvAction::Fin);
    }

    #[test]
    fn test_classify_uni_recv_stream_reset() {
        // ピアの RESET_STREAM はエラーコード付きで伝達する
        let err = s2n_quic::stream::Error::stream_reset(
            s2n_quic::application::Error::new(0x10e).expect("0x10e は VarInt 範囲内"),
        );
        assert_eq!(classify_uni_recv(Err(err)), UniRecvAction::Reset(0x10e));
    }

    #[test]
    fn test_classify_uni_recv_connection_error_is_ignored() {
        // 接続エラー (誤ラッチの実トリガー) は FIN として伝達しない
        let err = s2n_quic::stream::Error::from(s2n_quic::connection::Error::unspecified());
        assert!(
            matches!(err, s2n_quic::stream::Error::ConnectionError { .. }),
            "ConnectionError が生成されること"
        );
        assert_eq!(classify_uni_recv(Err(err)), UniRecvAction::Ignore);
    }

    #[test]
    fn test_classify_uni_recv_other_stream_error_is_ignored() {
        // ConnectionError 以外の StreamReset でないエラーも同様に伝達しない
        let err = s2n_quic::stream::Error::non_readable();
        assert_eq!(classify_uni_recv(Err(err)), UniRecvAction::Ignore);
    }
}
