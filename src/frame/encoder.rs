//! HTTP/3 フレームエンコーダー

use crate::varint;

use super::{DataPayload, Frame, FrameType, GoawayPayload, HeadersPayload, SettingsPayload};

/// フレームヘッダーをエンコードする
///
/// 成功時はエンコードしたバイト数を返す
pub fn encode_frame_header(buf: &mut [u8], frame_type: u64, payload_len: u64) -> Option<usize> {
    let type_len = varint::encoded_len(frame_type);
    let len_len = varint::encoded_len(payload_len);
    let total = type_len + len_len;

    if buf.len() < total {
        return None;
    }

    let mut offset = 0;
    offset += varint::encode(&mut buf[offset..], frame_type).ok()?;
    offset += varint::encode(&mut buf[offset..], payload_len).ok()?;

    Some(offset)
}

/// フレームをエンコードするのに必要なバイト数を計算
pub fn encoded_frame_len(frame: &Frame) -> usize {
    let (frame_type, payload_len) = match frame {
        Frame::Data(p) => (FrameType::Data as u64, p.data.len() as u64),
        Frame::Headers(p) => (
            FrameType::Headers as u64,
            p.encoded_field_section.len() as u64,
        ),
        Frame::Settings(p) => (FrameType::Settings as u64, encoded_settings_payload_len(p)),
        Frame::Goaway(p) => (FrameType::Goaway as u64, varint::encoded_len(p.id) as u64),
        Frame::MaxPushId(id) => (FrameType::MaxPushId as u64, varint::encoded_len(*id) as u64),
        Frame::Unknown {
            frame_type,
            payload,
        } => (*frame_type, payload.len() as u64),
    };

    let header_len = varint::encoded_len(frame_type) + varint::encoded_len(payload_len);
    header_len + payload_len as usize
}

/// SETTINGS ペイロードのエンコード長を計算
fn encoded_settings_payload_len(payload: &SettingsPayload) -> u64 {
    payload
        .entries
        .iter()
        .map(|(id, value)| varint::encoded_len(*id) + varint::encoded_len(*value))
        .sum::<usize>() as u64
}

/// フレームをエンコードする
///
/// 成功時はエンコードしたバイト数を返す
pub fn encode_frame(buf: &mut [u8], frame: &Frame) -> Option<usize> {
    let required = encoded_frame_len(frame);
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
    let payload_len = encoded_settings_payload_len(payload);

    let mut offset = encode_frame_header(buf, frame_type, payload_len)?;

    for (id, value) in &payload.entries {
        offset += varint::encode(&mut buf[offset..], *id).ok()?;
        offset += varint::encode(&mut buf[offset..], *value).ok()?;
    }

    Some(offset)
}

fn encode_goaway_frame(buf: &mut [u8], payload: &GoawayPayload) -> Option<usize> {
    let frame_type = FrameType::Goaway as u64;
    let payload_len = varint::encoded_len(payload.id) as u64;

    let mut offset = encode_frame_header(buf, frame_type, payload_len)?;
    offset += varint::encode(&mut buf[offset..], payload.id).ok()?;

    Some(offset)
}

fn encode_max_push_id_frame(buf: &mut [u8], id: u64) -> Option<usize> {
    let frame_type = FrameType::MaxPushId as u64;
    let payload_len = varint::encoded_len(id) as u64;

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
}
