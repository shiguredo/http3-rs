//! HTTP/3 フレームデコーダー

use bytes::{Buf, BytesMut};

use crate::error::FrameDecodeError;
use crate::settings::SettingsId;
use crate::varint;

use super::{DataPayload, Frame, FrameType, GoawayPayload, HeadersPayload, SettingsPayload};

/// フレームヘッダー
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    /// フレームタイプ
    pub frame_type: u64,
    /// ペイロード長
    pub payload_len: u64,
    /// ヘッダーのバイト長
    pub header_len: usize,
}

impl FrameHeader {
    /// フレーム全体のバイト長
    pub fn total_len(&self) -> usize {
        self.header_len + self.payload_len as usize
    }
}

/// フレームヘッダーをデコードする
///
/// 成功時は `FrameHeader` を返す
pub fn decode_frame_header(buf: &[u8]) -> Result<FrameHeader, FrameDecodeError> {
    if buf.is_empty() {
        return Err(FrameDecodeError::BufferTooShort);
    }

    let (frame_type, type_len) =
        varint::decode(buf).map_err(|_| FrameDecodeError::BufferTooShort)?;

    if buf.len() < type_len {
        return Err(FrameDecodeError::BufferTooShort);
    }

    let (payload_len, len_len) =
        varint::decode(&buf[type_len..]).map_err(|_| FrameDecodeError::BufferTooShort)?;

    // HTTP/2 専用フレームのチェック
    if FrameType::is_http2_only(frame_type) {
        return Err(FrameDecodeError::Http2Frame(frame_type));
    }

    Ok(FrameHeader {
        frame_type,
        payload_len,
        header_len: type_len + len_len,
    })
}

/// フレームを `BytesMut` から破壊的にデコードする (issue 0059 Phase 4)
///
/// - `Ok(Some(frame))`: フレームを 1 つ取り出した。`buf` は frame_total_len 分前進している
/// - `Ok(None)`: 必要なバイト数が揃っていない (呼び出し側は `is_fin()` 等を確認すること)
/// - `Err(_)`: パースエラー
///
/// DATA フレームの payload は `BytesMut::split_to(len).freeze()` で取り出すため、
/// 内部バッファからの実コピーが発生しない (zero-copy)。
pub fn decode_frame(buf: &mut BytesMut) -> Result<Option<Frame>, FrameDecodeError> {
    let header = match decode_frame_header(buf.as_ref()) {
        Ok(h) => h,
        Err(FrameDecodeError::BufferTooShort) => return Ok(None),
        Err(e) => return Err(e),
    };

    let total_len = header.total_len();
    if buf.len() < total_len {
        return Ok(None);
    }

    // ヘッダ部分を読み飛ばし、payload を切り出す
    buf.advance(header.header_len);
    let payload = buf.split_to(header.payload_len as usize).freeze();

    let frame = match FrameType::from_type(header.frame_type) {
        Some(FrameType::Data) => Frame::Data(DataPayload::new(payload)),
        Some(FrameType::Headers) => Frame::Headers(HeadersPayload::new(payload)),
        Some(FrameType::Settings) => decode_settings_frame(&payload)?,
        Some(FrameType::Goaway) => decode_goaway_frame(&payload)?,
        Some(FrameType::MaxPushId) => decode_max_push_id_frame(&payload)?,
        Some(FrameType::CancelPush | FrameType::PushPromise) => {
            // サーバープッシュはサポートしない
            //
            // RFC 9114 上の配置ルール:
            //   - CANCEL_PUSH (0x03): control stream でのみ送受信 (Section 7.2.3)
            //   - PUSH_PROMISE (0x05): request stream でのみ送信 (Section 7.2.5)
            //
            // 本実装ではサーバープッシュ機能を提供しないため、これらは stream 種別を
            // 問わず H3_FRAME_UNEXPECTED で拒否する。
            // MAX_PUSH_ID は control stream 上で正当に受信されうるため別経路で扱う
            // (Section 7.2.7)。
            return Err(FrameDecodeError::ServerPushNotSupported(header.frame_type));
        }
        None => Frame::Unknown {
            frame_type: header.frame_type,
            payload,
        },
    };

    Ok(Some(frame))
}

fn decode_settings_frame(payload: &[u8]) -> Result<Frame, FrameDecodeError> {
    let mut settings = SettingsPayload::new();
    let mut offset = 0;
    let mut seen_ids = std::collections::HashSet::new();

    while offset < payload.len() {
        // payload は完全に受信済みなので、途中切れは H3_FRAME_ERROR (RFC 9114 Section 7.1)
        let (id, id_len) =
            varint::decode(&payload[offset..]).map_err(|_| FrameDecodeError::InvalidLength)?;
        offset += id_len;

        let (value, value_len) =
            varint::decode(&payload[offset..]).map_err(|_| FrameDecodeError::InvalidLength)?;
        offset += value_len;

        // HTTP/2 専用設定のチェック (RFC 9114 Section 7.2.4)
        if SettingsId::is_http2_only(id) {
            return Err(FrameDecodeError::InvalidSettingsId(id));
        }

        // 同一フレーム内の重複 ID チェック (RFC 9114 Section 7.2.4)
        if !seen_ids.insert(id) {
            return Err(FrameDecodeError::InvalidSettingsId(id));
        }

        settings.add(id, value);
    }

    Ok(Frame::Settings(settings))
}

fn decode_goaway_frame(payload: &[u8]) -> Result<Frame, FrameDecodeError> {
    let (id, consumed) = varint::decode(payload).map_err(|_| FrameDecodeError::BufferTooShort)?;
    // ペイロードに余剰バイトがあれば不正 (RFC 9114 Section 7.1)
    if consumed != payload.len() {
        return Err(FrameDecodeError::InvalidLength);
    }
    Ok(Frame::Goaway(GoawayPayload::new(id)))
}

fn decode_max_push_id_frame(payload: &[u8]) -> Result<Frame, FrameDecodeError> {
    // MAX_PUSH_ID ペイロードは push ID の単一 varint (RFC 9114 Section 7.2.7)
    let (id, consumed) = varint::decode(payload).map_err(|_| FrameDecodeError::InvalidLength)?;
    if consumed != payload.len() {
        return Err(FrameDecodeError::InvalidLength);
    }
    Ok(Frame::MaxPushId(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_frame_header() {
        // Type=0 (DATA), Length=5
        let buf = [0x00, 0x05];
        let header = decode_frame_header(&buf).unwrap();
        assert_eq!(header.frame_type, 0);
        assert_eq!(header.payload_len, 5);
        assert_eq!(header.header_len, 2);

        // Type=4 (SETTINGS), Length=10
        let buf = [0x04, 0x0a];
        let header = decode_frame_header(&buf).unwrap();
        assert_eq!(header.frame_type, 4);
        assert_eq!(header.payload_len, 10);
    }

    #[test]
    fn test_decode_frame_header_http2_frame() {
        // Type=2 (PRIORITY - HTTP/2 only)
        let buf = [0x02, 0x05];
        let result = decode_frame_header(&buf);
        assert_eq!(result, Err(FrameDecodeError::Http2Frame(0x02)));

        // Type=6 (PING - HTTP/2 only)
        let buf = [0x06, 0x08];
        let result = decode_frame_header(&buf);
        assert_eq!(result, Err(FrameDecodeError::Http2Frame(0x06)));
    }

    #[test]
    fn test_decode_settings_frame_http2_setting() {
        // SETTINGS frame with HTTP/2-only setting (ENABLE_PUSH=0x02)
        let mut buf = BytesMut::from(&[0x04, 0x02, 0x02, 0x01][..]);
        let result = decode_frame(&mut buf);
        assert_eq!(result, Err(FrameDecodeError::InvalidSettingsId(0x02)));
    }

    #[test]
    fn test_decode_frame_buffer_too_short() {
        // データ不足は Ok(None) を返し、buf は前進しない
        let mut buf = BytesMut::from(&[0x00][..]); // Missing length
        assert_eq!(decode_frame(&mut buf), Ok(None));
        assert_eq!(buf.len(), 1);

        let mut buf = BytesMut::from(&[0x00, 0x05, 0x01, 0x02][..]); // Payload too short
        assert_eq!(decode_frame(&mut buf), Ok(None));
        assert_eq!(buf.len(), 4);
    }

    #[test]
    fn test_frame_header_total_len() {
        let header = FrameHeader {
            frame_type: 0,
            payload_len: 100,
            header_len: 2,
        };
        assert_eq!(header.total_len(), 102);
    }

    #[test]
    fn test_decode_settings_frame_duplicate_id() {
        // SETTINGS フレームに同一 ID が 2 回含まれる場合 (RFC 9114 Section 7.2.4)
        // QPACK_MAX_TABLE_CAPACITY (0x01) = 4, QPACK_MAX_TABLE_CAPACITY (0x01) = 8
        // Frame: type=0x04, len=4, payload=[0x01, 0x04, 0x01, 0x08]
        let mut buf = BytesMut::from(&[0x04, 0x04, 0x01, 0x04, 0x01, 0x08][..]);
        let result = decode_frame(&mut buf);
        assert_eq!(result, Err(FrameDecodeError::InvalidSettingsId(0x01)));
    }

    #[test]
    fn test_decode_server_push_frames_not_supported() {
        // CANCEL_PUSH (0x03) - サーバープッシュはサポートしない
        let mut buf = BytesMut::from(&[0x03, 0x01, 0x00][..]);
        let result = decode_frame(&mut buf);
        assert_eq!(result, Err(FrameDecodeError::ServerPushNotSupported(0x03)));

        // PUSH_PROMISE (0x05) - サーバープッシュはサポートしない
        let mut buf = BytesMut::from(&[0x05, 0x02, 0x00, 0x00][..]);
        let result = decode_frame(&mut buf);
        assert_eq!(result, Err(FrameDecodeError::ServerPushNotSupported(0x05)));
    }

    #[test]
    fn test_decode_max_push_id_frame() {
        // MAX_PUSH_ID (0x0d): サーバープッシュ非対応でも control stream 上では
        // 受信できなければならない (RFC 9114 Section 7.2.7)。デコード自体は成功する。
        let mut buf = BytesMut::from(&[0x0d, 0x01, 0x05][..]);
        let result = decode_frame(&mut buf).unwrap();
        assert_eq!(result, Some(Frame::MaxPushId(5)));
        assert!(
            buf.is_empty(),
            "decode_frame should consume the entire frame"
        );
    }
}
