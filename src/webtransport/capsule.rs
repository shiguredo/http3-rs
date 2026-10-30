//! WebTransport Capsule Protocol (draft-ietf-webtrans-http3-15 Section 5.6, 6)
//!
//! WebTransport セッション管理とフロー制御のための Capsule を定義。
//!
//! # Panics
//!
//! 本モジュールの `Capsule` 各 variant の `u64` フィールド
//! (`MaxData::maximum`, `MaxStreams::maximum`, `DataBlocked::maximum`,
//! `StreamsBlocked::maximum`, `Unknown::payload.len()` 等) は RFC 9000 Section 16 の
//! VarInt 範囲 (`0..=2^62 - 1`) を超えると `encode` / `encode_as_data_frame` で
//! panic する。これらの値域検査は現時点では未対応だが Capsule の構築時検査型化を行う際に
//! 構造的に保証する予定。それまでは利用側で範囲外値を渡さないこと。

use crate::varint::{self, VarInt};

/// Maximum Streams の上限値 (2^60)
/// draft-ietf-webtrans-http3-15 Section 5.6.2
/// 将来のドラフトで変更される可能性がある
pub const MAX_STREAMS_LIMIT: u64 = 1u64 << 60;

/// WT_CLOSE_SESSION の Application Error Message の最大長 (バイト)
///
/// `encode` はこの長さを超えるメッセージを UTF-8 文字境界で切り詰める。
/// `decode` はこの長さを超えるメッセージを malformed として拒否する。
/// draft-ietf-webtrans-http3-16 Section 6
/// 将来のドラフトで変更される可能性がある
pub(crate) const MAX_CLOSE_SESSION_MESSAGE_LEN: usize = 1024;

/// Capsule デコードエラー
///
/// RFC 9297 Section 3.2: Capsule payload は定義されたフィールドだけを
/// exactly 含まなければならない。余剰バイト・不足バイト・値の不整合は
/// すべて malformed として扱う。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapsuleDecodeError {
    /// 受信済み payload が定義されたフィールドと一致しない
    Malformed,
}

/// 正確に payload 全体を消費する varint デコード
///
/// RFC 9297 Section 3.2 の "exactly the fields" 要件を守るため、
/// varint 1 個を読んで余剰バイトが残る場合は malformed として扱う。
fn decode_exact_varint(payload: &[u8]) -> Result<u64, CapsuleDecodeError> {
    let (value, consumed) = decode_varint(payload).ok_or(CapsuleDecodeError::Malformed)?;
    if consumed != payload.len() {
        return Err(CapsuleDecodeError::Malformed);
    }
    Ok(value)
}

/// Capsule バリデーションエラー
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapsuleValidationError {
    /// WT_MAX_STREAMS の値が 2^60 を超えている (WT_FLOW_CONTROL_ERROR として扱う)
    /// draft-ietf-webtrans-http3-16 Section 5.6.2
    /// 将来のドラフトで変更される可能性がある
    MaxStreamsExceedsLimit,
    /// WT_MAX_STREAMS の値が以前の値から増加していない (WT_FLOW_CONTROL_ERROR)
    /// draft-ietf-webtrans-http3-16 Section 5.6.2
    /// 将来のドラフトで変更される可能性がある
    MaxStreamsDecreased,
    /// WT_MAX_DATA の値が 2^62-1 を超えている (H3_DATAGRAM_ERROR として扱う)
    MaxDataExceedsLimit,
    /// WT_MAX_DATA の値が以前の値から増加していない (WT_FLOW_CONTROL_ERROR)
    /// draft-ietf-webtrans-http3-16 Section 5.6.4
    /// 将来のドラフトで変更される可能性がある
    MaxDataDecreased,
}

/// HTTP/3 では禁止される WT_MAX_STREAM_DATA Capsule タイプ (draft-ietf-webtrans-http2)
pub const PROHIBITED_WT_MAX_STREAM_DATA_CAPSULE_TYPE: u64 = 0x190B4D3E;

/// HTTP/3 では禁止される WT_STREAM_DATA_BLOCKED Capsule タイプ (draft-ietf-webtrans-http2)
pub const PROHIBITED_WT_STREAM_DATA_BLOCKED_CAPSULE_TYPE: u64 = 0x190B4D42;

/// Capsule タイプ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u64)]
pub enum CapsuleType {
    /// セッションクローズ
    CloseSession = 0x2843,
    /// セッションドレイン (グレースフルシャットダウン)
    DrainSession = 0x78ae,
    /// 最大データ量
    MaxData = 0x190B4D3D,
    /// 最大ストリーム数 (双方向)
    MaxStreamsBidi = 0x190B4D3F,
    /// 最大ストリーム数 (単方向)
    MaxStreamsUni = 0x190B4D40,
    /// データブロック
    DataBlocked = 0x190B4D41,
    /// ストリームブロック (双方向)
    StreamsBlockedBidi = 0x190B4D43,
    /// ストリームブロック (単方向)
    StreamsBlockedUni = 0x190B4D44,
}

impl CapsuleType {
    /// タイプ値から `CapsuleType` を作成
    pub fn from_type(t: u64) -> Option<Self> {
        match t {
            0x2843 => Some(Self::CloseSession),
            0x78ae => Some(Self::DrainSession),
            0x190B4D3D => Some(Self::MaxData),
            0x190B4D3F => Some(Self::MaxStreamsBidi),
            0x190B4D40 => Some(Self::MaxStreamsUni),
            0x190B4D41 => Some(Self::DataBlocked),
            0x190B4D43 => Some(Self::StreamsBlockedBidi),
            0x190B4D44 => Some(Self::StreamsBlockedUni),
            _ => None,
        }
    }
}

/// WebTransport Capsule
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capsule {
    /// WT_CLOSE_SESSION: セッションを終了
    CloseSession {
        /// アプリケーションエラーコード (32-bit)
        error_code: u32,
        /// エラーメッセージ (UTF-8)
        ///
        /// 1024 バイトを超える場合、[`Capsule::encode`] は UTF-8 文字境界で
        /// 切り詰めてからエンコードする
        /// (draft-ietf-webtrans-http3-16 Section 6)。
        message: String,
    },

    /// WT_DRAIN_SESSION: セッションのグレースフルシャットダウン
    DrainSession,

    /// WT_MAX_DATA: セッションレベルの最大データ量
    MaxData {
        /// 最大データ量 (バイト)
        maximum: u64,
    },

    /// WT_MAX_STREAMS: ストリーム上限
    MaxStreams {
        /// 双方向ストリームかどうか
        bidirectional: bool,
        /// 最大ストリーム数
        maximum: u64,
    },

    /// WT_DATA_BLOCKED: データ送信がブロックされた
    DataBlocked {
        /// ブロック時の最大データ量
        maximum: u64,
    },

    /// WT_STREAMS_BLOCKED: ストリーム作成がブロックされた
    StreamsBlocked {
        /// 双方向ストリームかどうか
        bidirectional: bool,
        /// ブロック時の最大ストリーム数
        maximum: u64,
    },

    /// 不明な Capsule
    Unknown {
        /// Capsule タイプ
        capsule_type: u64,
        /// ペイロード
        payload: Vec<u8>,
    },
}

/// 可変長整数を Vec にエンコード
///
/// 呼び出し側は値が VarInt 範囲内 (RFC 9000 Section 16) であることを保証する。
/// 範囲外の値が渡された場合は panic する。
fn encode_varint(buf: &mut Vec<u8>, value: u64) {
    let value = VarInt::new(value).expect("capsule field value fits in VarInt");
    varint::encode_into_vec(buf, value);
}

/// 可変長整数をエンコードしたサイズを返す
fn varint_encoded_len(value: u64) -> usize {
    VarInt::new(value)
        .expect("capsule field value fits in VarInt")
        .encoded_len()
}

/// 可変長整数をデコード (生の `u64` 値を返す)
fn decode_varint(buf: &[u8]) -> Option<(u64, usize)> {
    varint::decode(buf).ok().map(|(v, n)| (v.get(), n))
}

/// WT_CLOSE_SESSION の Application Error Message を 1024 バイト以下に切り詰める
///
/// 送信側がアプリケーション由来のメッセージを切り詰める場合は
/// UTF-8 文字境界で行わなければならない。
/// (draft-ietf-webtrans-http3-16 Section 6)
/// 将来のドラフトで変更される可能性がある
///
/// 受信側 (`decode_payload`) は 1024 バイト超のメッセージを malformed として
/// 拒否するため、送信側でこの上限を守ることで全送信経路が
/// デコード可能なカプセルを生成することを保証する。
fn truncate_close_session_message(message: &str) -> &[u8] {
    if message.len() <= MAX_CLOSE_SESSION_MESSAGE_LEN {
        return message.as_bytes();
    }
    let mut end = MAX_CLOSE_SESSION_MESSAGE_LEN;
    // UTF-8 の 1 文字は最大 4 バイトなので、境界合わせは最大 3 回で終わる
    while end > 0 && !message.is_char_boundary(end) {
        end -= 1;
    }
    &message.as_bytes()[..end]
}

impl Capsule {
    /// Capsule タイプが HTTP/3 で禁止されるかどうか
    ///
    /// WebTransport over HTTP/3 では WT_MAX_STREAM_DATA と WT_STREAM_DATA_BLOCKED の
    /// 受信は禁止される (draft-ietf-webtrans-http3-15 Section 5.4)。
    pub fn is_prohibited_in_http3_type(capsule_type: u64) -> bool {
        matches!(
            capsule_type,
            PROHIBITED_WT_MAX_STREAM_DATA_CAPSULE_TYPE
                | PROHIBITED_WT_STREAM_DATA_BLOCKED_CAPSULE_TYPE
        )
    }

    /// この Capsule が HTTP/3 で禁止されるタイプかどうか
    pub fn is_prohibited_in_http3(&self) -> bool {
        Self::is_prohibited_in_http3_type(self.capsule_type())
    }

    /// カプセルを HTTP/3 DATA フレームとしてエンコードする
    ///
    /// 1 カプセル = 1 DATA フレームとして buf に追記する。
    /// CONNECT ストリーム上での WebTransport カプセル送信に使用する。
    /// 内部で [`Capsule::encode`] を呼ぶため、WT_CLOSE_SESSION の
    /// メッセージ長切り詰めも本メソッド経由で適用される。
    pub fn encode_as_data_frame(&self, buf: &mut Vec<u8>) {
        let mut capsule_bytes = Vec::new();
        self.encode(&mut capsule_bytes);
        // DATA フレーム: タイプ (0x00) + ペイロード長 + ペイロード
        varint::encode_into_vec(buf, VarInt::ZERO);
        encode_varint(buf, capsule_bytes.len() as u64);
        buf.extend_from_slice(&capsule_bytes);
    }

    /// Capsule をエンコード
    ///
    /// WT_CLOSE_SESSION の Application Error Message は 1024 バイトを超えない。
    /// `message` が 1024 バイトを超える場合は UTF-8 文字境界で切り詰める。
    /// (draft-ietf-webtrans-http3-16 Section 6)
    /// 将来のドラフトで変更される可能性がある
    pub fn encode(&self, buf: &mut Vec<u8>) {
        match self {
            Self::CloseSession {
                error_code,
                message,
            } => {
                let message_bytes = truncate_close_session_message(message);
                Self::encode_capsule_header(
                    buf,
                    CapsuleType::CloseSession as u64,
                    4 + message_bytes.len(),
                );
                buf.extend_from_slice(&error_code.to_be_bytes());
                buf.extend_from_slice(message_bytes);
            }

            Self::DrainSession => {
                Self::encode_capsule_header(buf, CapsuleType::DrainSession as u64, 0);
            }

            Self::MaxData { maximum } => {
                let payload_len = varint_encoded_len(*maximum);
                Self::encode_capsule_header(buf, CapsuleType::MaxData as u64, payload_len);
                encode_varint(buf, *maximum);
            }

            Self::MaxStreams {
                bidirectional,
                maximum,
            } => {
                let capsule_type = if *bidirectional {
                    CapsuleType::MaxStreamsBidi as u64
                } else {
                    CapsuleType::MaxStreamsUni as u64
                };
                let payload_len = varint_encoded_len(*maximum);
                Self::encode_capsule_header(buf, capsule_type, payload_len);
                encode_varint(buf, *maximum);
            }

            Self::DataBlocked { maximum } => {
                let payload_len = varint_encoded_len(*maximum);
                Self::encode_capsule_header(buf, CapsuleType::DataBlocked as u64, payload_len);
                encode_varint(buf, *maximum);
            }

            Self::StreamsBlocked {
                bidirectional,
                maximum,
            } => {
                let capsule_type = if *bidirectional {
                    CapsuleType::StreamsBlockedBidi as u64
                } else {
                    CapsuleType::StreamsBlockedUni as u64
                };
                let payload_len = varint_encoded_len(*maximum);
                Self::encode_capsule_header(buf, capsule_type, payload_len);
                encode_varint(buf, *maximum);
            }

            Self::Unknown {
                capsule_type,
                payload,
            } => {
                Self::encode_capsule_header(buf, *capsule_type, payload.len());
                buf.extend_from_slice(payload);
            }
        }
    }

    /// Capsule ヘッダーをエンコード
    fn encode_capsule_header(buf: &mut Vec<u8>, capsule_type: u64, length: usize) {
        encode_varint(buf, capsule_type);
        encode_varint(buf, length as u64);
    }

    /// Capsule をデコード
    ///
    /// # Returns
    ///
    /// - `Ok(Some((capsule, consumed)))`: デコード成功
    /// - `Ok(None)`: バッファが不足している (incomplete)
    /// - `Err(CapsuleDecodeError)`: 受信済みバイトが malformed
    ///
    /// RFC 9297 Section 3.2: Capsule payload は定義されたフィールドだけを
    /// exactly 含まなければならない。余剰バイトはすべて malformed として扱う。
    pub fn decode(buf: &[u8]) -> Result<Option<(Self, usize)>, CapsuleDecodeError> {
        let mut offset = 0;

        // Capsule タイプ
        let Some((capsule_type, len)) = decode_varint(&buf[offset..]) else {
            return Ok(None);
        };
        offset += len;

        // ペイロード長
        let Some((length, len)) = decode_varint(&buf[offset..]) else {
            return Ok(None);
        };
        offset += len;

        let Some(length) = usize::try_from(length).ok() else {
            // 32-bit 環境で usize を超える length はデコード不能
            return Ok(None);
        };
        let Some(end) = offset.checked_add(length) else {
            return Ok(None);
        };
        if buf.len() < end {
            return Ok(None);
        }

        let payload = &buf[offset..end];
        let capsule = Self::decode_payload(capsule_type, payload)?;

        Ok(Some((capsule, end)))
    }

    /// ペイロードから Capsule をデコード
    ///
    /// payload は length-framed で完全に受信済みであるため、足りないバイトも
    /// 余剰バイトもすべて malformed として扱う。
    /// (RFC 9297 Section 3.2: "exactly the fields")
    fn decode_payload(capsule_type: u64, payload: &[u8]) -> Result<Self, CapsuleDecodeError> {
        match CapsuleType::from_type(capsule_type) {
            Some(CapsuleType::CloseSession) => {
                if payload.len() < 4 {
                    return Err(CapsuleDecodeError::Malformed);
                }
                let error_code = u32::from_be_bytes(
                    payload[..4]
                        .try_into()
                        .map_err(|_| CapsuleDecodeError::Malformed)?,
                );
                let message_bytes = &payload[4..];
                // メッセージ長は 1024 バイトを超えてはならない
                // (draft-ietf-webtrans-http3-15 Section 6)
                if message_bytes.len() > MAX_CLOSE_SESSION_MESSAGE_LEN {
                    return Err(CapsuleDecodeError::Malformed);
                }
                let message = String::from_utf8(message_bytes.to_vec())
                    .map_err(|_| CapsuleDecodeError::Malformed)?;
                Ok(Self::CloseSession {
                    error_code,
                    message,
                })
            }

            Some(CapsuleType::DrainSession) => {
                if !payload.is_empty() {
                    return Err(CapsuleDecodeError::Malformed);
                }
                Ok(Self::DrainSession)
            }

            Some(CapsuleType::MaxData) => {
                let maximum = decode_exact_varint(payload)?;
                Ok(Self::MaxData { maximum })
            }

            Some(CapsuleType::MaxStreamsBidi) => {
                let maximum = decode_exact_varint(payload)?;
                Ok(Self::MaxStreams {
                    bidirectional: true,
                    maximum,
                })
            }

            Some(CapsuleType::MaxStreamsUni) => {
                let maximum = decode_exact_varint(payload)?;
                Ok(Self::MaxStreams {
                    bidirectional: false,
                    maximum,
                })
            }

            Some(CapsuleType::DataBlocked) => {
                let maximum = decode_exact_varint(payload)?;
                Ok(Self::DataBlocked { maximum })
            }

            Some(CapsuleType::StreamsBlockedBidi) => {
                let maximum = decode_exact_varint(payload)?;
                Ok(Self::StreamsBlocked {
                    bidirectional: true,
                    maximum,
                })
            }

            Some(CapsuleType::StreamsBlockedUni) => {
                let maximum = decode_exact_varint(payload)?;
                Ok(Self::StreamsBlocked {
                    bidirectional: false,
                    maximum,
                })
            }

            None => Ok(Self::Unknown {
                capsule_type,
                payload: payload.to_vec(),
            }),
        }
    }

    /// Capsule タイプを取得
    pub fn capsule_type(&self) -> u64 {
        match self {
            Self::CloseSession { .. } => CapsuleType::CloseSession as u64,
            Self::DrainSession => CapsuleType::DrainSession as u64,
            Self::MaxData { .. } => CapsuleType::MaxData as u64,
            Self::MaxStreams { bidirectional, .. } => {
                if *bidirectional {
                    CapsuleType::MaxStreamsBidi as u64
                } else {
                    CapsuleType::MaxStreamsUni as u64
                }
            }
            Self::DataBlocked { .. } => CapsuleType::DataBlocked as u64,
            Self::StreamsBlocked { bidirectional, .. } => {
                if *bidirectional {
                    CapsuleType::StreamsBlockedBidi as u64
                } else {
                    CapsuleType::StreamsBlockedUni as u64
                }
            }
            Self::Unknown { capsule_type, .. } => *capsule_type,
        }
    }

    /// WT_MAX_STREAMS の値を検証する
    ///
    /// - `maximum` が 2^60 を超える場合: `MaxStreamsExceedsLimit` (WT_FLOW_CONTROL_ERROR)
    /// - `maximum` が `current_max` から増加しない場合: `MaxStreamsDecreased` (WT_FLOW_CONTROL_ERROR)
    ///
    /// draft-ietf-webtrans-http3-16 Section 5.6.2
    /// 将来のドラフトで変更される可能性がある
    pub fn validate_max_streams(
        maximum: u64,
        current_max: u64,
    ) -> Result<(), CapsuleValidationError> {
        if maximum > MAX_STREAMS_LIMIT {
            return Err(CapsuleValidationError::MaxStreamsExceedsLimit);
        }
        if maximum <= current_max {
            return Err(CapsuleValidationError::MaxStreamsDecreased);
        }
        Ok(())
    }

    /// WT_MAX_DATA の値を検証する
    ///
    /// - `maximum` が VarInt 上限 (`2^62-1`) を超える場合: `MaxDataExceedsLimit` (H3_DATAGRAM_ERROR)
    /// - `maximum` が `current_max` から増加しない場合: `MaxDataDecreased` (WT_FLOW_CONTROL_ERROR)
    ///
    /// draft-ietf-webtrans-http3-16 Section 5.6.4
    /// 将来のドラフトで変更される可能性がある
    pub fn validate_max_data(maximum: u64, current_max: u64) -> Result<(), CapsuleValidationError> {
        if maximum > crate::VarInt::MAX.get() {
            return Err(CapsuleValidationError::MaxDataExceedsLimit);
        }
        if maximum <= current_max {
            return Err(CapsuleValidationError::MaxDataDecreased);
        }
        Ok(())
    }

    /// フロー制御 Capsule かどうか
    pub fn is_flow_control(&self) -> bool {
        matches!(
            self,
            Self::MaxData { .. }
                | Self::MaxStreams { .. }
                | Self::DataBlocked { .. }
                | Self::StreamsBlocked { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_empty_buffer() {
        let result = Capsule::decode(&[]);
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn test_capsule_type_from_type() {
        assert_eq!(
            CapsuleType::from_type(0x2843),
            Some(CapsuleType::CloseSession)
        );
        assert_eq!(
            CapsuleType::from_type(0x78ae),
            Some(CapsuleType::DrainSession)
        );
        assert_eq!(
            CapsuleType::from_type(0x190B4D3D),
            Some(CapsuleType::MaxData)
        );
        assert_eq!(
            CapsuleType::from_type(0x190B4D3F),
            Some(CapsuleType::MaxStreamsBidi)
        );
        assert_eq!(
            CapsuleType::from_type(0x190B4D40),
            Some(CapsuleType::MaxStreamsUni)
        );
        assert_eq!(CapsuleType::from_type(0x99), None);
    }

    #[test]
    fn test_is_flow_control() {
        assert!(
            !Capsule::CloseSession {
                error_code: 0,
                message: String::new()
            }
            .is_flow_control()
        );
        assert!(!Capsule::DrainSession.is_flow_control());
        assert!(Capsule::MaxData { maximum: 0 }.is_flow_control());
        assert!(
            Capsule::MaxStreams {
                bidirectional: true,
                maximum: 0
            }
            .is_flow_control()
        );
        assert!(Capsule::DataBlocked { maximum: 0 }.is_flow_control());
        assert!(
            Capsule::StreamsBlocked {
                bidirectional: true,
                maximum: 0
            }
            .is_flow_control()
        );
    }

    #[test]
    fn test_is_prohibited_in_http3_type() {
        assert!(Capsule::is_prohibited_in_http3_type(
            PROHIBITED_WT_MAX_STREAM_DATA_CAPSULE_TYPE
        ));
        assert!(Capsule::is_prohibited_in_http3_type(
            PROHIBITED_WT_STREAM_DATA_BLOCKED_CAPSULE_TYPE
        ));
        assert!(!Capsule::is_prohibited_in_http3_type(
            CapsuleType::MaxData as u64
        ));
    }

    #[test]
    fn test_validate_max_streams() {
        // 正常: 増加
        assert!(Capsule::validate_max_streams(10, 5).is_ok());
        // 正常: 上限値
        assert!(Capsule::validate_max_streams(MAX_STREAMS_LIMIT, 0).is_ok());
        // エラー: 上限超過
        assert_eq!(
            Capsule::validate_max_streams(MAX_STREAMS_LIMIT + 1, 0),
            Err(CapsuleValidationError::MaxStreamsExceedsLimit)
        );
        // エラー: 同値 (draft-16: "does not increase")
        assert_eq!(
            Capsule::validate_max_streams(10, 10),
            Err(CapsuleValidationError::MaxStreamsDecreased)
        );
        // エラー: 減少
        assert_eq!(
            Capsule::validate_max_streams(5, 10),
            Err(CapsuleValidationError::MaxStreamsDecreased)
        );
    }

    #[test]
    fn test_validate_max_data() {
        // 正常: 増加
        assert!(Capsule::validate_max_data(100, 50).is_ok());
        // 正常: VarInt 上限値
        assert!(Capsule::validate_max_data(crate::VarInt::MAX.get(), 0).is_ok());
        // エラー: 同値 (draft-16: "does not increase")
        assert_eq!(
            Capsule::validate_max_data(100, 100),
            Err(CapsuleValidationError::MaxDataDecreased)
        );
        // エラー: 減少
        assert_eq!(
            Capsule::validate_max_data(50, 100),
            Err(CapsuleValidationError::MaxDataDecreased)
        );
        // エラー: VarInt 上限超過
        assert_eq!(
            Capsule::validate_max_data(crate::VarInt::MAX.get() + 1, 0),
            Err(CapsuleValidationError::MaxDataExceedsLimit)
        );
    }

    #[test]
    fn test_is_prohibited_in_http3() {
        let c = Capsule::Unknown {
            capsule_type: PROHIBITED_WT_MAX_STREAM_DATA_CAPSULE_TYPE,
            payload: vec![],
        };
        assert!(c.is_prohibited_in_http3());
    }

    // ---------------------------------------------------------------
    // WT_CLOSE_SESSION の Application Error Message 長
    // (draft-ietf-webtrans-http3-16 Section 6)
    // ---------------------------------------------------------------

    /// エンコード済みバイト列がちょうど 1 カプセルであることを確認してデコードする
    fn decode_single_capsule(buf: &[u8]) -> Capsule {
        let (capsule, consumed) = Capsule::decode(buf)
            .expect("テスト用のカプセルデコードに成功すること")
            .expect("テスト用のカプセルが 1 つ分揃っていること");
        assert_eq!(
            consumed,
            buf.len(),
            "エンコード結果はちょうど 1 カプセルであること"
        );
        capsule
    }

    /// CloseSession カプセルをデコードして (エラーコード, メッセージ) を取り出す
    fn decode_close_session(buf: &[u8]) -> (u32, String) {
        match decode_single_capsule(buf) {
            Capsule::CloseSession {
                error_code,
                message,
            } => (error_code, message),
            other => panic!("CloseSession にデコードされること: {other:?}"),
        }
    }

    /// 任意のメッセージバイト列を持つ WT_CLOSE_SESSION カプセルを組み立てる
    fn build_close_session_capsule(error_code: u32, message_bytes: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        encode_varint(&mut buf, CapsuleType::CloseSession as u64);
        encode_varint(&mut buf, (4 + message_bytes.len()) as u64);
        buf.extend_from_slice(&error_code.to_be_bytes());
        buf.extend_from_slice(message_bytes);
        buf
    }

    #[test]
    fn test_encode_close_session_message_1023_and_1024_bytes() {
        // 1023 / 1024 バイトは上限以内なので切り詰められない
        for len in [1023usize, 1024] {
            let message = "a".repeat(len);
            let capsule = Capsule::CloseSession {
                error_code: 7,
                message: message.clone(),
            };
            let mut buf = Vec::new();
            capsule.encode(&mut buf);
            let (error_code, decoded) = decode_close_session(&buf);
            assert_eq!(error_code, 7);
            assert_eq!(
                decoded, message,
                "{len} バイトのメッセージは切り詰められないこと"
            );
            assert_eq!(decoded.len(), len);
        }
    }

    #[test]
    fn test_encode_close_session_message_1025_bytes() {
        // 1025 バイトは 1024 バイトに切り詰められる
        let message = "a".repeat(1025);
        let capsule = Capsule::CloseSession {
            error_code: 9,
            message,
        };
        let mut buf = Vec::new();
        capsule.encode(&mut buf);
        let (error_code, decoded) = decode_close_session(&buf);
        assert_eq!(error_code, 9);
        assert_eq!(decoded.len(), 1024, "切り詰め後のメッセージは 1024 バイト");
        assert_eq!(decoded, "a".repeat(1024));
    }

    #[test]
    fn test_encode_close_session_message_empty() {
        // 空文字列はそのままエンコードされる
        let capsule = Capsule::CloseSession {
            error_code: 0,
            message: String::new(),
        };
        let mut buf = Vec::new();
        capsule.encode(&mut buf);
        let (error_code, decoded) = decode_close_session(&buf);
        assert_eq!(error_code, 0);
        assert!(decoded.is_empty());
    }

    #[test]
    fn test_encode_close_session_message_truncates_at_utf8_boundary() {
        // 1024 境界をまたぐマルチバイト文字は、文字全体を捨てて
        // UTF-8 文字境界で切り詰められる (2 / 3 / 4 バイト文字を網羅する)
        for (ch, char_len) in [('é', 2usize), ('あ', 3), ('😀', 4)] {
            for offset in 0..char_len {
                // base バイトの ASCII の直後にマルチバイト文字を置く。
                // offset = 0 のとき全長はちょうど 1024 バイトになる。
                let base = 1024 - char_len + offset;
                let message = format!("{}{}", "a".repeat(base), ch);
                let capsule = Capsule::CloseSession {
                    error_code: 1,
                    message: message.clone(),
                };
                let mut buf = Vec::new();
                capsule.encode(&mut buf);
                let (_, decoded) = decode_close_session(&buf);

                if offset == 0 {
                    // 境界ちょうどに収まる場合は切り詰められない
                    assert_eq!(
                        decoded, message,
                        "全長 1024 バイトのメッセージは切り詰められないこと"
                    );
                } else {
                    // 文字の途中で切らず、文字全体を捨てる
                    assert_eq!(
                        decoded,
                        "a".repeat(base),
                        "マルチバイト文字 ({char_len} バイト, offset {offset}) は文字境界で切り詰められること"
                    );
                }
                assert!(
                    decoded.len() <= MAX_CLOSE_SESSION_MESSAGE_LEN,
                    "切り詰め結果は 1024 バイト以下であること"
                );
            }
        }
    }

    #[test]
    fn test_encode_as_data_frame_truncates_close_session_message() {
        // encode_as_data_frame は encode 経由なので、DATA フレーム経路でも
        // 1024 バイト上限が適用される
        let capsule = Capsule::CloseSession {
            error_code: 3,
            message: "あ".repeat(500), // 1500 バイト
        };
        let mut buf = Vec::new();
        capsule.encode_as_data_frame(&mut buf);

        // DATA フレーム: タイプ (0x00) + ペイロード長 + ペイロード
        let (frame_type, type_len) =
            decode_varint(&buf).expect("DATA フレームタイプをデコードできること");
        assert_eq!(frame_type, 0, "DATA フレームのタイプは 0x00");
        let (frame_len, len_len) =
            decode_varint(&buf[type_len..]).expect("DATA フレーム長をデコードできること");
        let payload_start = type_len + len_len;
        assert_eq!(
            payload_start + frame_len as usize,
            buf.len(),
            "DATA フレーム長はペイロード全体と一致すること"
        );

        let (error_code, decoded) = decode_close_session(&buf[payload_start..]);
        assert_eq!(error_code, 3);
        assert_eq!(
            decoded.len(),
            1023,
            "3 バイト文字 'あ' は 1024 バイトを超えない 341 文字に切り詰められること"
        );
        assert_eq!(decoded, "あ".repeat(341), "342 文字目以降の 'あ' は捨てる");
    }

    #[test]
    fn test_decode_close_session_message_over_1024_bytes_is_malformed() {
        // decode 側は 1024 バイト超のメッセージを malformed として拒否する
        // (変更しない既存挙動の確認)
        let over = build_close_session_capsule(0, &vec![b'a'; 1025]);
        assert_eq!(
            Capsule::decode(&over),
            Err(CapsuleDecodeError::Malformed),
            "1025 バイトのメッセージは malformed"
        );

        let at_limit = build_close_session_capsule(0, &vec![b'a'; 1024]);
        let (capsule, consumed) = Capsule::decode(&at_limit)
            .expect("テスト用のカプセルデコードに成功すること")
            .expect("テスト用のカプセルが 1 つ分揃っていること");
        assert_eq!(consumed, at_limit.len());
        assert!(matches!(capsule, Capsule::CloseSession { .. }));
    }
}
