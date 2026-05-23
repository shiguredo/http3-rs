//! HTTP/3 フレームエンコーダー

use crate::varint::{self, VarInt};

use super::{DataPayload, Frame, FrameType, GoawayPayload, HeadersPayload, SettingsPayload};

/// フレームヘッダーをエンコードする
///
/// 成功時はエンコードしたバイト数を返す。
/// `frame_type` または `payload_len` が VarInt 範囲 (RFC 9000 Section 16) を
/// 超える場合は `None` を返す。
pub fn encode_frame_header(buf: &mut [u8], frame_type: u64, payload_len: u64) -> Option<usize> {
    let frame_type = VarInt::new(frame_type).ok()?;
    let payload_len = VarInt::new(payload_len).ok()?;

    let total = frame_type.encoded_len() + payload_len.encoded_len();
    if buf.len() < total {
        return None;
    }

    let mut offset = 0;
    offset += varint::encode(&mut buf[offset..], frame_type).ok()?;
    offset += varint::encode(&mut buf[offset..], payload_len).ok()?;

    Some(offset)
}

/// フレームをエンコードするのに必要なバイト数を計算する
///
/// `Frame::Unknown` / `Frame::Goaway` / `Frame::MaxPushId` 等で
/// `u64` フィールドが VarInt 範囲を超える場合は `None` を返す。
/// 呼び出し側は本値を根拠にバッファを確保するため、嘘の長さを返さない。
pub fn encoded_frame_len(frame: &Frame) -> Option<usize> {
    let (frame_type, payload_len) = match frame {
        Frame::Data(p) => (FrameType::Data as u64, p.data.len() as u64),
        Frame::Headers(p) => (
            FrameType::Headers as u64,
            p.encoded_field_section.len() as u64,
        ),
        Frame::Settings(p) => (FrameType::Settings as u64, encoded_settings_payload_len(p)?),
        Frame::Goaway(p) => (FrameType::Goaway as u64, encoded_varint_len(p.id)? as u64),
        Frame::MaxPushId(id) => (FrameType::MaxPushId as u64, encoded_varint_len(*id)? as u64),
        Frame::Unknown {
            frame_type,
            payload,
        } => (*frame_type, payload.len() as u64),
    };

    let header_len = encoded_varint_len(frame_type)? + encoded_varint_len(payload_len)?;
    Some(header_len + payload_len as usize)
}

/// `u64` を VarInt とみなしたエンコード長を返す
///
/// 値が VarInt 範囲外の場合は `None` を返し、呼び出し側で短絡する。
fn encoded_varint_len(value: u64) -> Option<usize> {
    VarInt::new(value).ok().map(|v| v.encoded_len())
}

/// SETTINGS ペイロードのエンコード長を計算
///
/// 各 [`Setting`](crate::settings::Setting) の wire 表現が VarInt 範囲内である
/// ことは構築時に保証されているため、本関数はバッファサイズの合算のみを行う。
/// 内部総和の `usize` オーバーフローのみ `None` で返す。
fn encoded_settings_payload_len(payload: &SettingsPayload) -> Option<u64> {
    let mut total: usize = 0;
    for setting in payload.settings() {
        let (id, value) = setting.as_wire();
        total = total.checked_add(id.encoded_len())?;
        total = total.checked_add(value.encoded_len())?;
    }
    Some(total as u64)
}

/// フレームをエンコードする
///
/// 成功時はエンコードしたバイト数を返す。
/// バッファ不足、または `Frame::Unknown` 等の `u64` フィールドが VarInt 範囲外の場合は
/// `None` を返す。
pub fn encode_frame(buf: &mut [u8], frame: &Frame) -> Option<usize> {
    let required = encoded_frame_len(frame)?;
    if buf.len() < required {
        return None;
    }

    match frame {
        Frame::Data(p) => encode_data_frame(buf, p),
        Frame::Headers(p) => encode_headers_frame(buf, p),
        Frame::Settings(p) => encode_settings_frame(buf, p),
        Frame::Goaway(p) => encode_goaway_frame(buf, p),
        Frame::MaxPushId(value) => encode_max_push_id_frame(buf, *value),
        Frame::Unknown {
            frame_type,
            payload,
        } => encode_unknown_frame(buf, *frame_type, payload),
    }
}

fn encode_data_frame(buf: &mut [u8], payload: &DataPayload) -> Option<usize> {
    let frame_type = FrameType::Data as u64;
    let payload_len = payload.data.len() as u64;

    let mut offset = encode_frame_header(buf, frame_type, payload_len)?;
    buf[offset..offset + payload.data.len()].copy_from_slice(&payload.data);
    offset += payload.data.len();

    Some(offset)
}

fn encode_headers_frame(buf: &mut [u8], payload: &HeadersPayload) -> Option<usize> {
    let frame_type = FrameType::Headers as u64;
    let payload_len = payload.encoded_field_section.len() as u64;

    let mut offset = encode_frame_header(buf, frame_type, payload_len)?;
    buf[offset..offset + payload.encoded_field_section.len()]
        .copy_from_slice(&payload.encoded_field_section);
    offset += payload.encoded_field_section.len();

    Some(offset)
}

fn encode_settings_frame(buf: &mut [u8], payload: &SettingsPayload) -> Option<usize> {
    let frame_type = FrameType::Settings as u64;
    let payload_len = encoded_settings_payload_len(payload)?;

    let mut offset = encode_frame_header(buf, frame_type, payload_len)?;

    for setting in payload.settings() {
        let (id, value) = setting.as_wire();
        offset += varint::encode(&mut buf[offset..], id).ok()?;
        offset += varint::encode(&mut buf[offset..], value).ok()?;
    }

    Some(offset)
}

fn encode_goaway_frame(buf: &mut [u8], payload: &GoawayPayload) -> Option<usize> {
    let frame_type = FrameType::Goaway as u64;
    let id = VarInt::new(payload.id).ok()?;
    let payload_len = id.encoded_len() as u64;

    let mut offset = encode_frame_header(buf, frame_type, payload_len)?;
    offset += varint::encode(&mut buf[offset..], id).ok()?;

    Some(offset)
}

fn encode_max_push_id_frame(buf: &mut [u8], id: u64) -> Option<usize> {
    let frame_type = FrameType::MaxPushId as u64;
    let id = VarInt::new(id).ok()?;
    let payload_len = id.encoded_len() as u64;

    let mut offset = encode_frame_header(buf, frame_type, payload_len)?;
    offset += varint::encode(&mut buf[offset..], id).ok()?;

    Some(offset)
}

fn encode_unknown_frame(buf: &mut [u8], frame_type: u64, payload: &[u8]) -> Option<usize> {
    let payload_len = payload.len() as u64;

    let mut offset = encode_frame_header(buf, frame_type, payload_len)?;
    buf[offset..offset + payload.len()].copy_from_slice(payload);
    offset += payload.len();

    Some(offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_frame_header() {
        let mut buf = [0u8; 16];

        // Type=0, Length=0
        let len = encode_frame_header(&mut buf, 0, 0).unwrap();
        assert_eq!(len, 2);
        assert_eq!(&buf[..2], &[0x00, 0x00]);

        // Type=4 (SETTINGS), Length=10
        let len = encode_frame_header(&mut buf, 4, 10).unwrap();
        assert_eq!(len, 2);
        assert_eq!(&buf[..2], &[0x04, 0x0a]);
    }

    #[test]
    fn test_encode_frame_header_rejects_out_of_range() {
        // frame_type が VarInt 範囲外の場合は None
        let mut buf = [0u8; 16];
        assert_eq!(encode_frame_header(&mut buf, 1u64 << 62, 0), None);
        // payload_len が VarInt 範囲外の場合は None
        assert_eq!(encode_frame_header(&mut buf, 0, 1u64 << 62), None);
    }

    #[test]
    fn test_encoded_frame_len_rejects_unknown_out_of_range() {
        // Frame::Unknown で frame_type が VarInt 範囲外の場合は None
        let frame = Frame::Unknown {
            frame_type: 1u64 << 62,
            payload: vec![],
        };
        assert_eq!(encoded_frame_len(&frame), None);
        let mut buf = vec![0u8; 16];
        assert_eq!(encode_frame(&mut buf, &frame), None);
    }
}
