//! HTTP/3 フレーム (RFC 9114 Section 7.2)
//!
//! HTTP/3 で使用されるフレームの型定義とエンコード/デコード機能を提供。
//!
//! ## サーバープッシュについて
//!
//! このライブラリはサーバープッシュをサポートしない。
//! CANCEL_PUSH, PUSH_PROMISE, MAX_PUSH_ID フレームを受信した場合は
//! H3_FRAME_UNEXPECTED エラーを返す。
//!
//! これは nghttp3 と同様の方針であり、主要なブラウザ (Chrome, Firefox) でも
//! サーバープッシュは無効化されているため、実用上問題ない。

mod decoder;
mod encoder;

pub use decoder::{FrameHeader, decode_frame, decode_frame_header};
pub use encoder::{encode_frame, encode_frame_header, encoded_frame_len};

use std::collections::HashSet;

use crate::settings::{Setting, SettingError};
use crate::varint::VarInt;

/// フレームタイプ (RFC 9114 Section 7.2)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum FrameType {
    /// DATA フレーム (0x00)
    Data = 0x00,
    /// HEADERS フレーム (0x01)
    Headers = 0x01,
    /// CANCEL_PUSH フレーム (0x03) - サーバープッシュはサポートしない
    CancelPush = 0x03,
    /// SETTINGS フレーム (0x04)
    Settings = 0x04,
    /// PUSH_PROMISE フレーム (0x05) - サーバープッシュはサポートしない
    PushPromise = 0x05,
    /// GOAWAY フレーム (0x07)
    Goaway = 0x07,
    /// MAX_PUSH_ID フレーム (0x0d) - サーバープッシュはサポートしない
    MaxPushId = 0x0d,
}

impl FrameType {
    /// タイプ ID から `FrameType` を作成
    pub fn from_type(t: u64) -> Option<Self> {
        match t {
            0x00 => Some(Self::Data),
            0x01 => Some(Self::Headers),
            0x03 => Some(Self::CancelPush),
            0x04 => Some(Self::Settings),
            0x05 => Some(Self::PushPromise),
            0x07 => Some(Self::Goaway),
            0x0d => Some(Self::MaxPushId),
            _ => None,
        }
    }

    /// HTTP/2 専用フレームかどうか
    pub fn is_http2_only(t: u64) -> bool {
        matches!(t, 0x02 | 0x06 | 0x08 | 0x09)
    }
}

/// HTTP/3 フレーム
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// DATA フレーム
    Data(DataPayload),
    /// HEADERS フレーム (QPACK エンコード済みヘッダーブロック)
    Headers(HeadersPayload),
    /// SETTINGS フレーム
    Settings(SettingsPayload),
    /// GOAWAY フレーム
    Goaway(GoawayPayload),
    /// MAX_PUSH_ID フレーム (RFC 9114 Section 7.2.7)
    ///
    /// サーバープッシュ自体はサポートしないが、クライアントから control stream 上で
    /// 送信される正当なフレームのため、デコードして単調性検証だけ行う。
    MaxPushId(u64),
    /// 不明なフレーム (スキップ用)
    Unknown { frame_type: u64, payload: Vec<u8> },
}

impl Frame {
    /// フレームタイプを取得
    pub fn frame_type(&self) -> u64 {
        match self {
            Self::Data(_) => FrameType::Data as u64,
            Self::Headers(_) => FrameType::Headers as u64,
            Self::Settings(_) => FrameType::Settings as u64,
            Self::Goaway(_) => FrameType::Goaway as u64,
            Self::MaxPushId(_) => FrameType::MaxPushId as u64,
            Self::Unknown { frame_type, .. } => *frame_type,
        }
    }
}

/// DATA フレームペイロード
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataPayload {
    /// ペイロードデータ
    pub data: Vec<u8>,
}

impl DataPayload {
    /// 新しい DATA ペイロードを作成
    pub fn new(data: Vec<u8>) -> Self {
        Self { data }
    }
}

/// HEADERS フレームペイロード
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadersPayload {
    /// QPACK エンコード済みヘッダーブロック
    pub encoded_field_section: Vec<u8>,
}

impl HeadersPayload {
    /// 新しい HEADERS ペイロードを作成
    pub fn new(encoded_field_section: Vec<u8>) -> Self {
        Self {
            encoded_field_section,
        }
    }
}

/// SETTINGS フレームペイロード
///
/// 内部の [`Setting`] は構築時に値検査済みかつ ID 重複が無いことを保証する。
/// wire 表現の `(id, value)` から構築するには [`Setting::from_wire`] で先に
/// [`Setting`] を作って [`SettingsPayload::add`] に渡す。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SettingsPayload {
    /// 設定エントリのリスト (構築順を維持し、ID 重複は許容しない)
    settings: Vec<Setting>,
    /// `add` 経由で投入された ID の集合 (重複検出用)
    seen_ids: HashSet<VarInt>,
}

impl SettingsPayload {
    /// 新しい SETTINGS ペイロードを作成
    pub fn new() -> Self {
        Self::default()
    }

    /// 検査済みの [`Setting`] を追加する
    ///
    /// 同一 SETTINGS ID が既に存在する場合は
    /// [`SettingError::DuplicateId`] を返す (RFC 9114 §7.2.4: MUST NOT
    /// occur more than once)。
    pub fn add(&mut self, setting: Setting) -> Result<(), SettingError> {
        let id = setting.id();
        if !self.seen_ids.insert(id) {
            return Err(SettingError::DuplicateId { id });
        }
        self.settings.push(setting);
        Ok(())
    }

    /// 保持する全 [`Setting`] のスライス
    ///
    /// 追加順を維持し、ID 重複は構築時に弾かれているため存在しない。
    pub fn settings(&self) -> &[Setting] {
        &self.settings
    }

    /// エントリ数
    pub fn len(&self) -> usize {
        self.settings.len()
    }

    /// エントリが空かどうか
    pub fn is_empty(&self) -> bool {
        self.settings.is_empty()
    }

    /// Settings から SettingsPayload を作成
    ///
    /// H3 設定と WebTransport 設定の両方を含める。`Settings` のフィールドは
    /// 各 ID と 1 対 1 に対応するため、追加時に [`SettingError::DuplicateId`] が
    /// 発生する可能性は無い (`expect` で握り潰す)。
    pub fn from_settings(settings: &crate::settings::Settings) -> Self {
        let mut payload = Self::new();
        for setting in settings.iter() {
            payload
                .add(setting)
                .expect("Settings::iter() yields unique IDs");
        }
        if let Some(wt) = &settings.wt_settings {
            for setting in wt.iter() {
                payload
                    .add(setting)
                    .expect("webtransport::Settings::iter() yields unique IDs");
            }
        }
        payload
    }
}

/// GOAWAY フレームペイロード
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GoawayPayload {
    /// ストリーム ID またはプッシュ ID
    pub id: u64,
}

impl GoawayPayload {
    /// 新しい GOAWAY ペイロードを作成
    pub fn new(id: u64) -> Self {
        Self { id }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_frame_type() {
        let data = Frame::Data(DataPayload::new(vec![1, 2, 3]));
        assert_eq!(data.frame_type(), 0x00);

        let headers = Frame::Headers(HeadersPayload::new(vec![4, 5, 6]));
        assert_eq!(headers.frame_type(), 0x01);

        let settings = Frame::Settings(SettingsPayload::new());
        assert_eq!(settings.frame_type(), 0x04);

        let goaway = Frame::Goaway(GoawayPayload::new(100));
        assert_eq!(goaway.frame_type(), 0x07);

        let unknown = Frame::Unknown {
            frame_type: 0x99,
            payload: vec![],
        };
        assert_eq!(unknown.frame_type(), 0x99);
    }
}
