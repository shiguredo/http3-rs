//! サーバー実装で共有する接続関連ヘルパー
//!
//! `server` と `webtransport` のサーバー実装で共通利用する、
//! DCID ルーティングと CONNECTION_CLOSE 送信のヘルパーを置く。

use std::collections::{BTreeSet, HashMap};
use std::net::SocketAddr;

use shiguredo_ngtcp2::{Connection, ConnectionId, Error, Http3Connection, Result};

use crate::{Socket, timestamp};

/// サーバーが生成する SCID の長さ (バイト)
///
/// サーバーが発行する CID はすべてこの長さのため、Short header パケットの
/// DCID も原則この長さになる (RFC 9000 Section 17.3 は Short header に
/// DCID 長を含めないため、受信側は既知の長さで照合する)。
pub(crate) const SERVER_SCID_LEN: usize = 16;

/// 新規接続パケットから取り出した情報
pub(crate) struct NewConnectionInfo {
    /// QUIC バージョン
    pub(crate) version: u32,
    /// クライアントが選んだ DCID (Original Destination Connection ID)
    ///
    /// クライアントは Initial の再送時に同じ DCID を使うため、
    /// 既存接続へのルーティングキーとしても使用する。
    pub(crate) original_dcid: ConnectionId,
    /// クライアントの SCID
    ///
    /// サーバーがクライアントに送るパケットの DCID になる。
    pub(crate) client_scid: ConnectionId,
}

/// Long header の新規接続パケットをパースする (RFC 9000 Section 17.2)
///
/// Long header のレイアウトは以下:
/// type(1) + version(4) + DCID Length(1) + DCID + SCID Length(1) + SCID
///
/// 以下のパケットは None を返す (RFC 9000 Section 5.2.2):
///
/// - Initial 以外の Long header (Handshake / 0-RTT / Retry)。
///   Handshake は SHOULD ignore、0-RTT のバッファリングは MAY だが、
///   本実装ではバッファリングを行わず破棄する。状態を持たないパケットで
///   サーバーの接続状態を消費させないため
/// - パースできないパケット (短すぎる・CID 長が不正)
/// - クライアントの最初の Initial として DCID が 8 バイト未満のパケット
///   (RFC 9000 Section 7.2 の MUST に反するため「仕様に完全に準拠した
///   Initial」ではない。5.2.2 により drop する)
/// - CID 長が 20 を超える Long header (RFC 9000 Section 17.2 の MUST drop)
/// - ゼロ長 CID (サーバーとして扱わない。ConnectionId が拒否する)
/// - 1200 バイト未満の UDP datagram で運ばれた Initial (RFC 9000 Section 14.1 の
///   MUST discard。ngtcp2 も read_pkt 内で enforce するが、TLS セッション生成
///   前に破棄して不正パケットあたりのコストを削減する)
pub(crate) fn parse_new_connection_packet(data: &[u8]) -> Option<NewConnectionInfo> {
    if data.len() < ngtcp2_sys::NGTCP2_MAX_UDP_PAYLOAD_SIZE as usize {
        return None;
    }
    if data.len() < 6 {
        return None;
    }

    // Long header の type 上位 4 ビットが Initial (0xC0) であることを確認する
    if data[0] & 0xF0 != 0xC0 {
        return None;
    }

    let version = u32::from_be_bytes([data[1], data[2], data[3], data[4]]);

    let dcid_len = data[5] as usize;
    // クライアントの最初の Initial の DCID は 8 バイト以上が MUST (RFC 9000 Section 7.2)
    if dcid_len < ngtcp2_sys::NGTCP2_MIN_INITIAL_DCIDLEN as usize {
        return None;
    }
    if data.len() < 6 + dcid_len {
        return None;
    }
    let original_dcid = ConnectionId::new(&data[6..6 + dcid_len])?;

    let scid_offset = 6 + dcid_len;
    if data.len() < scid_offset + 1 {
        return None;
    }
    let client_scid_len = data[scid_offset] as usize;
    if data.len() < scid_offset + 1 + client_scid_len {
        return None;
    }
    let client_scid = ConnectionId::new(&data[scid_offset + 1..scid_offset + 1 + client_scid_len])?;

    Some(NewConnectionInfo {
        version,
        original_dcid,
        client_scid,
    })
}

/// 到着パケットの DCID から接続キーを解決する (RFC 9000 Section 5.2)
///
/// - Long header: DCID 長はヘッダーに含まれるため、そのまま照合する
/// - Short header: DCID 長はヘッダーに含まれないため (RFC 9000 Section 17.3)、
///   サーバーが発行した CID の長さの集合で照合する
///
/// DCID がルーティングテーブルに無い場合は None を返す。
/// 呼び出し側は None の場合に Long header なら新規接続、Short header なら破棄を
/// 判断する (Short header の破棄は RFC 9000 Section 5.2.2 の MUST drop に従う)。
/// Stateless Reset (RFC 9000 Section 10.3) は実装しないため、未知 DCID の
/// パケットには応答せず黙って破棄する。
pub(crate) fn resolve_dcid(
    cid_map: &HashMap<ConnectionId, ConnectionId>,
    short_cid_lengths: &BTreeSet<usize>,
    data: &[u8],
) -> Option<ConnectionId> {
    if data.is_empty() {
        return None;
    }

    if data[0] & 0x80 != 0 {
        // Long header (RFC 9000 Section 17.2)
        if data.len() < 6 {
            return None;
        }
        let dcid_len = data[5] as usize;
        if data.len() < 6 + dcid_len {
            return None;
        }
        let dcid = ConnectionId::new(&data[6..6 + dcid_len])?;
        return cid_map.get(&dcid).cloned();
    }

    // Short header (RFC 9000 Section 17.3)
    // 長い CID から順に照合する。Short header は DCID 長を運ばないため、
    // パケットの先頭が別長の CID と偶然 prefix 一致した場合の誤ルーティングを
    // 減らすため、発行済み CID の長いものから優先して一致を見る
    for len in short_cid_lengths.iter().rev() {
        let len = *len;
        if data.len() < 1 + len {
            continue;
        }
        if let Some(dcid) = ConnectionId::new(&data[1..1 + len])
            && let Some(key) = cid_map.get(&dcid)
        {
            return Some(key.clone());
        }
    }

    None
}

/// エラーを受けて CONNECTION_CLOSE を送信する
///
/// 致命的な接続エラーは CONNECTION_CLOSE でピアに通知する (RFC 9000 Section 11.1)。
/// nghttp3 (HTTP/3 層) のエラーは QUIC アプリケーションエラーコード (0x1d)、
/// ngtcp2 (トランスポート層) のエラーはトランスポートエラーコード (0x1c) に
/// それぞれ変換して送信する。
///
/// 戻り値は CONNECTION_CLOSE パケットを送信できたかどうか
/// (ソケット送信自体の失敗は戻り値に反映せず、ログ出力のみ)。
/// NGTCP2_ERR_NOBUF などで書き込めない場合は false を返し、呼び出し側は
/// 接続を黙って破棄する。NOBUF は ngtcp2 が実施する anti-amplification 制限
/// (RFC 9000 Section 8.1: アドレス検証前は受信 3 倍までしか送信できない)
/// の超過でも発生する。
pub(crate) async fn send_connection_close(
    conn: &mut Connection,
    socket: &Socket,
    remote: SocketAddr,
    send_buf: &mut [u8],
    err: &Error,
) -> bool {
    let ts = timestamp();

    // エラー種別に応じた CONNECTION_CLOSE を書き込む
    // 呼び出し側は TransportClose / ApplicationClose に分類されたエラーだけを
    // 渡す契約のため、それ以外のエラーはここでは送信できない
    let result = match err {
        Error::Nghttp3(_, code) => {
            // nghttp3 エラーコードから QUIC アプリケーションエラーコードを導出する
            let app_code = unsafe { nghttp3_sys::nghttp3_err_infer_quic_app_error_code(*code) };
            conn.write_connection_close_app(send_buf, app_code, b"", ts)
        }
        Error::Ngtcp2(_, code) => {
            // ngtcp2 エラーコードから QUIC トランスポートエラーコードを導出する
            let transport_code =
                unsafe { ngtcp2_sys::ngtcp2_err_infer_quic_transport_error_code(*code) };
            conn.write_connection_close(send_buf, transport_code, b"", ts)
        }
        _ => return false,
    };

    match result {
        Ok(written) => {
            if written == 0 {
                // 書き込めるパケットがない (closing / draining 状態)
                return false;
            }
            if let Err(e) = socket.send_to(&send_buf[..written], remote).await {
                eprintln!("[tokio-ngtcp2] send error: {}", e);
            }
            true
        }
        Err(e) => {
            eprintln!("[tokio-ngtcp2] failed to write CONNECTION_CLOSE: {}", e);
            false
        }
    }
}

/// 受信したストリームデータを HTTP/3 に渡す
///
/// QUIC の受信ストリームデータを順に取り出し、HTTP/3 に読み込ませて
/// 消費した分のフロー制御クレジットを拡張する。エラーは呼び出し側で
/// 接続単位に処理する。
pub(crate) fn feed_stream_data_to_h3(
    conn: &mut Connection,
    h3_conn: &mut Http3Connection,
    ts: u64,
) -> Result<()> {
    while let Some(stream_data) = conn.poll_stream_data() {
        let consumed = h3_conn.read_stream(
            stream_data.stream_id,
            &stream_data.data,
            stream_data.fin,
            ts,
        )?;

        if consumed > 0 {
            conn.extend_max_stream_offset(stream_data.stream_id, consumed as u64)?;
            conn.extend_max_offset(consumed as u64);
        }
    }

    Ok(())
}
