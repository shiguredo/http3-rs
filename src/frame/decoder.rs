//! HTTP/3 フレームデコーダー

use crate::error::FrameDecodeError;
use crate::settings::Setting;
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
    if FrameType::is_http2_only(frame_type.get()) {
        return Err(FrameDecodeError::Http2Frame(frame_type.get()));
    }

    Ok(FrameHeader {
        frame_type: frame_type.get(),
        payload_len: payload_len.get(),
        header_len: type_len + len_len,
    })
}

/// フレームをデコードする
///
/// 成功時は `(Frame, 消費バイト数)` を返す
pub fn decode_frame(buf: &[u8]) -> Result<(Frame, usize), FrameDecodeError> {
    let header = decode_frame_header(buf)?;

    let total_len = header.total_len();
    if buf.len() < total_len {
        return Err(FrameDecodeError::BufferTooShort);
    }

    let payload = &buf[header.header_len..total_len];

    let frame = match FrameType::from_type(header.frame_type) {
        Some(FrameType::Data) => decode_data_frame(payload)?,
        Some(FrameType::Headers) => decode_headers_frame(payload)?,
        Some(FrameType::Settings) => decode_settings_frame(payload)?,
        Some(FrameType::Goaway) => decode_goaway_frame(payload)?,
        Some(FrameType::MaxPushId) => decode_max_push_id_frame(payload)?,
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
            payload: payload.to_vec(),
        },
    };

    Ok((frame, total_len))
}

fn decode_data_frame(payload: &[u8]) -> Result<Frame, FrameDecodeError> {
    Ok(Frame::Data(DataPayload::new(payload.to_vec())))
}

fn decode_headers_frame(payload: &[u8]) -> Result<Frame, FrameDecodeError> {
    Ok(Frame::Headers(HeadersPayload::new(payload.to_vec())))
}

fn decode_settings_frame(payload: &[u8]) -> Result<Frame, FrameDecodeError> {
    let mut settings = SettingsPayload::new();
    let mut offset = 0;

    while offset < payload.len() {
        // payload は完全に受信済みなので、途中切れは H3_FRAME_ERROR (RFC 9114 Section 7.1)
        let (id, id_len) =
            varint::decode(&payload[offset..]).map_err(|_| FrameDecodeError::InvalidLength)?;
        offset += id_len;

        let (value, value_len) =
            varint::decode(&payload[offset..]).map_err(|_| FrameDecodeError::InvalidLength)?;
        offset += value_len;

        // 値域 / HTTP2 専用 / 予約 / bool / 重複は全て SettingsPayload::add 経路で検査
        let setting = Setting::from_wire(id, value)?;
        settings.add(setting)?;
    }

    Ok(Frame::Settings(settings))
}

fn decode_goaway_frame(payload: &[u8]) -> Result<Frame, FrameDecodeError> {
    let (id, consumed) = varint::decode(payload).map_err(|_| FrameDecodeError::BufferTooShort)?;
    // ペイロードに余剰バイトがあれば不正 (RFC 9114 Section 7.1)
    if consumed != payload.len() {
        return Err(FrameDecodeError::InvalidLength);
    }
    Ok(Frame::Goaway(GoawayPayload::new(id.get())))
}

fn decode_max_push_id_frame(payload: &[u8]) -> Result<Frame, FrameDecodeError> {
    // MAX_PUSH_ID ペイロードは push ID の単一 varint (RFC 9114 Section 7.2.7)
    let (id, consumed) = varint::decode(payload).map_err(|_| FrameDecodeError::InvalidLength)?;
    if consumed != payload.len() {
        return Err(FrameDecodeError::InvalidLength);
    }
    Ok(Frame::MaxPushId(id.get()))
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
        use crate::settings::SettingError;
        use crate::varint::VarInt;

        // SETTINGS frame with HTTP/2-only setting (ENABLE_PUSH=0x02)
        let buf = [0x04, 0x02, 0x02, 0x01];
        let result = decode_frame(&buf);
        assert_eq!(
            result,
            Err(FrameDecodeError::InvalidSetting(
                SettingError::Http2OnlyId {
                    id: VarInt::new(0x02).unwrap()
                }
            ))
        );
    }

    #[test]
    fn test_decode_frame_buffer_too_short() {
        let buf = [0x00]; // Missing length
        assert_eq!(decode_frame(&buf), Err(FrameDecodeError::BufferTooShort));

        let buf = [0x00, 0x05, 0x01, 0x02]; // Payload too short
        assert_eq!(decode_frame(&buf), Err(FrameDecodeError::BufferTooShort));
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
        use crate::settings::SettingError;
        use crate::varint::VarInt;

        // SETTINGS フレームに同一 ID が 2 回含まれる場合 (RFC 9114 §7.2.4 MUST NOT)
        let buf = [0x04, 0x04, 0x01, 0x04, 0x01, 0x08];
        let result = decode_frame(&buf);
        assert_eq!(
            result,
            Err(FrameDecodeError::InvalidSetting(
                SettingError::DuplicateId {
                    id: VarInt::new(0x01).unwrap()
                }
            ))
        );
    }

    #[test]
    fn test_decode_settings_frame_reserved_id() {
        use crate::settings::SettingError;
        use crate::varint::VarInt;

        // SETTINGS フレームに予約 ID 0x00 が含まれる場合 (RFC 9114 §11.2.2 Table 3)
        let buf = [0x04, 0x02, 0x00, 0x00];
        let result = decode_frame(&buf);
        assert_eq!(
            result,
            Err(FrameDecodeError::InvalidSetting(SettingError::ReservedId {
                id: VarInt::ZERO
            }))
        );
    }

    #[test]
    fn test_decode_settings_frame_invalid_boolean_ecp() {
        use crate::settings::SettingError;
        use crate::varint::VarInt;

        // SETTINGS_ENABLE_CONNECT_PROTOCOL (0x08) に 2 を設定する (RFC 8441 §3 違反)
        let buf = [0x04, 0x02, 0x08, 0x02];
        let result = decode_frame(&buf);
        assert_eq!(
            result,
            Err(FrameDecodeError::InvalidSetting(
                SettingError::InvalidBooleanValue {
                    id: VarInt::new(0x08).unwrap(),
                    value: VarInt::new(0x02).unwrap(),
                }
            ))
        );
    }

    #[test]
    fn test_decode_settings_frame_invalid_boolean_h3_datagram() {
        use crate::settings::SettingError;
        use crate::varint::VarInt;

        // SETTINGS_H3_DATAGRAM (0x33) に 5 を設定する (RFC 9297 §2.1.1 違反)
        let buf = [0x04, 0x02, 0x33, 0x05];
        let result = decode_frame(&buf);
        assert_eq!(
            result,
            Err(FrameDecodeError::InvalidSetting(
                SettingError::InvalidBooleanValue {
                    id: VarInt::new(0x33).unwrap(),
                    value: VarInt::new(0x05).unwrap(),
                }
            ))
        );
    }

    #[test]
    fn test_decode_server_push_frames_not_supported() {
        // CANCEL_PUSH (0x03) - サーバープッシュはサポートしない
        let buf = [0x03, 0x01, 0x00];
        let result = decode_frame(&buf);
        assert_eq!(result, Err(FrameDecodeError::ServerPushNotSupported(0x03)));

        // PUSH_PROMISE (0x05) - サーバープッシュはサポートしない
        let buf = [0x05, 0x02, 0x00, 0x00];
        let result = decode_frame(&buf);
        assert_eq!(result, Err(FrameDecodeError::ServerPushNotSupported(0x05)));
    }

    #[test]
    fn test_decode_max_push_id_frame() {
        // MAX_PUSH_ID (0x0d): サーバープッシュ非対応でも control stream 上では
        // 受信できなければならない (RFC 9114 Section 7.2.7)。デコード自体は成功する。
        let buf = [0x0d, 0x01, 0x05];
        let result = decode_frame(&buf).unwrap();
        assert_eq!(result, (Frame::MaxPushId(5), 3));
    }
}
