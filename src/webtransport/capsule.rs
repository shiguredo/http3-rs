//! WebTransport Capsule Protocol (draft-ietf-webtrans-http3-16 Section 5.6, 6)
//!
//! WebTransport セッション管理とフロー制御のための Capsule を定義。
//!
//! # 値域の保証
//!
//! フロー制御系の `maximum` フィールドは [`crate::varint::VarInt`] 型のため、
//! RFC 9000 Section 16 の値域 (`0..=2^62 - 1`) が構築時に保証される。
//! `encode` / `encode_as_data_frame` は panic しない。
//!
//! `Unknown` の `capsule_type` と `payload.len()` は wire から復号した値または
//! 利用者が与えた生値で、VarInt 範囲外なら `encode` が何も書かずに戻る
//! (`Capsule::encode` の説明を参照)。

use crate::varint::{self, VarInt};

/// Maximum Streams の上限値 (2^60)
///
/// WT_MAX_STREAMS の `Maximum Streams` は 2^60 を超えてはならない。
/// 超過は `WT_FLOW_CONTROL_ERROR` として扱う。
/// draft-ietf-webtrans-http3-16 Section 5.6.2
/// 将来のドラフトで変更される可能性がある
pub const MAX_STREAMS_LIMIT: u64 = 1u64 << 60;

/// H3_DATAGRAM_ERROR エラーコード (RFC 9297 Section 5.2)
///
/// HTTP/3 datagram / capsule プロトコルのパースエラーを示す接続レベルエラー。
/// draft-16 では WT_MAX_STREAMS の超過は `WT_FLOW_CONTROL_ERROR` に変更されたため、
/// 本定数は draft-15 以前の相手との相互運用のために残す。
/// (draft-ietf-webtrans-http3-16 Section 5.6.2)
/// 将来のドラフトで変更される可能性がある
pub const H3_DATAGRAM_ERROR: u64 = 0x33;

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

/// Capsule エンコードエラー
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapsuleEncodeError {
    /// wire 表現にできない値 (RFC 9000 Section 16 の VarInt 範囲外)
    ///
    /// `Unknown` の `capsule_type` / ペイロード長が該当する。フロー制御系の
    /// `maximum` は `VarInt` 型のため本エラーにはならない。
    ValueOutOfVarIntRange,
}

impl core::fmt::Display for CapsuleEncodeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ValueOutOfVarIntRange => write!(
                f,
                "capsule field cannot be encoded as a QUIC varint (RFC 9000 Section 16)"
            ),
        }
    }
}

impl std::error::Error for CapsuleEncodeError {}

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
        /// エラーメッセージ (UTF-8, 最大 1024 バイト)
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

/// 可変長整数を Vec にエンコードする
///
/// RFC 9000 Section 16 の値域外 (`> 2^62 - 1`) の場合は何も書かずに `false` を返す。
/// 呼び出し側は戻り値を確認し、書けなかった場合はエンコード全体を破棄すること。
fn encode_varint(buf: &mut Vec<u8>, value: u64) -> bool {
    let Ok(value) = VarInt::new(value) else {
        return false;
    };
    varint::encode_into_vec(buf, value);
    true
}

/// 可変長整数をエンコードしたサイズを返す
///
/// 値域外の場合は `None` を返す。
fn varint_encoded_len(value: u64) -> Option<usize> {
    VarInt::new(value).ok().map(|v| v.encoded_len())
}

/// 可変長整数をデコード (生の `u64` 値を返す)
fn decode_varint(buf: &[u8]) -> Option<(u64, usize)> {
    varint::decode(buf).ok().map(|(v, n)| (v.get(), n))
}

impl Capsule {
    /// Capsule タイプが HTTP/3 で禁止されるかどうか
    ///
    /// WebTransport over HTTP/3 では WT_MAX_STREAM_DATA と WT_STREAM_DATA_BLOCKED の
    /// 受信は禁止される (draft-ietf-webtrans-http3-16 Section 5.4)。
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
    ///
    /// wire 表現にできない値の場合は [`CapsuleEncodeError`] を返し、
    /// `buf` は変更しない。
    pub fn encode_as_data_frame(&self, buf: &mut Vec<u8>) -> Result<(), CapsuleEncodeError> {
        let mut capsule_bytes = Vec::new();
        self.encode(&mut capsule_bytes)?;
        let Ok(capsule_len) = VarInt::new(capsule_bytes.len() as u64) else {
            return Err(CapsuleEncodeError::ValueOutOfVarIntRange);
        };
        // DATA フレーム: タイプ (0x00) + ペイロード長 + ペイロード
        varint::encode_into_vec(buf, VarInt::ZERO);
        varint::encode_into_vec(buf, capsule_len);
        buf.extend_from_slice(&capsule_bytes);
        Ok(())
    }

    /// Capsule をエンコード
    ///
    /// wire 表現にできない値の場合は [`CapsuleEncodeError`] を返し、
    /// `buf` は変更しない。
    pub fn encode(&self, buf: &mut Vec<u8>) -> Result<(), CapsuleEncodeError> {
        let mut out = Vec::new();
        match self {
            Self::CloseSession {
                error_code,
                message,
            } => {
                Self::encode_capsule_header(
                    &mut out,
                    CapsuleType::CloseSession as u64,
                    4 + message.len(),
                )?;
                out.extend_from_slice(&error_code.to_be_bytes());
                out.extend_from_slice(message.as_bytes());
            }

            Self::DrainSession => {
                Self::encode_capsule_header(&mut out, CapsuleType::DrainSession as u64, 0)?;
            }

            Self::MaxData { maximum } => {
                let payload_len = varint_encoded_len(*maximum)
                    .ok_or(CapsuleEncodeError::ValueOutOfVarIntRange)?;
                Self::encode_capsule_header(&mut out, CapsuleType::MaxData as u64, payload_len)?;
                encode_varint(&mut out, *maximum);
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
                let payload_len = varint_encoded_len(*maximum)
                    .ok_or(CapsuleEncodeError::ValueOutOfVarIntRange)?;
                Self::encode_capsule_header(&mut out, capsule_type, payload_len)?;
                encode_varint(&mut out, *maximum);
            }

            Self::DataBlocked { maximum } => {
                let payload_len = varint_encoded_len(*maximum)
                    .ok_or(CapsuleEncodeError::ValueOutOfVarIntRange)?;
                Self::encode_capsule_header(
                    &mut out,
                    CapsuleType::DataBlocked as u64,
                    payload_len,
                )?;
                encode_varint(&mut out, *maximum);
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
                let payload_len = varint_encoded_len(*maximum)
                    .ok_or(CapsuleEncodeError::ValueOutOfVarIntRange)?;
                Self::encode_capsule_header(&mut out, capsule_type, payload_len)?;
                encode_varint(&mut out, *maximum);
            }

            Self::Unknown {
                capsule_type,
                payload,
            } => {
                Self::encode_capsule_header(&mut out, *capsule_type, payload.len())?;
                out.extend_from_slice(payload);
            }
        }
        buf.extend_from_slice(&out);
        Ok(())
    }

    /// Capsule ヘッダーをエンコード
    ///
    /// wire 表現にできない値の場合は `ValueOutOfVarIntRange` を返す。
    fn encode_capsule_header(
        buf: &mut Vec<u8>,
        capsule_type: u64,
        length: usize,
    ) -> Result<(), CapsuleEncodeError> {
        let Ok(length) = VarInt::new(length as u64) else {
            return Err(CapsuleEncodeError::ValueOutOfVarIntRange);
        };
        if !encode_varint(buf, capsule_type) {
            return Err(CapsuleEncodeError::ValueOutOfVarIntRange);
        }
        varint::encode_into_vec(buf, length);
        Ok(())
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

        // Capsule Type
        let Some((capsule_type, len)) = decode_varint(&buf[offset..]) else {
            return Ok(None);
        };
        offset += len;

        // Length
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
                // (draft-ietf-webtrans-http3-16 Section 6)
                if message_bytes.len() > 1024 {
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
    fn test_encode_rejects_out_of_range_unknown_type() {
        // Unknown の capsule_type は生の u64 のため VarInt 範囲外があり得る。
        // encode は panic せずエラーを返し、buf を変更しない。
        let capsule = Capsule::Unknown {
            capsule_type: u64::MAX,
            payload: vec![0x01],
        };
        let mut buf = Vec::new();
        assert_eq!(
            capsule.encode(&mut buf),
            Err(CapsuleEncodeError::ValueOutOfVarIntRange)
        );
        assert!(buf.is_empty(), "失敗時に buf が変更されている: {buf:?}");

        assert_eq!(
            capsule.encode_as_data_frame(&mut buf),
            Err(CapsuleEncodeError::ValueOutOfVarIntRange)
        );
        assert!(buf.is_empty(), "失敗時に buf が変更されている: {buf:?}");
    }

    #[test]
    fn test_encode_accepts_max_varint() {
        // VarInt の最大値は wire 表現できる
        let maximum = VarInt::MAX.get();
        let capsule = Capsule::MaxData { maximum };
        let mut buf = Vec::new();
        capsule
            .encode(&mut buf)
            .expect("VarInt 最大値は符号化できる");
        assert!(!buf.is_empty());
        let mut framed = Vec::new();
        capsule
            .encode_as_data_frame(&mut framed)
            .expect("VarInt 最大値は DATA フレーム化できる");
        assert!(framed.len() > buf.len());
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
}
