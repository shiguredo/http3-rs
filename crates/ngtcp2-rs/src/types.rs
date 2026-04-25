use std::net::SocketAddr;

// ============================================================================
// QUIC バージョン定数
// ============================================================================
//
// ngtcp2.h では NGTCP2_PROTO_VER_V1/V2 がキャスト式を含むマクロで定義されている:
//   #define NGTCP2_PROTO_VER_V1 ((uint32_t)0x00000001u)
//   #define NGTCP2_PROTO_VER_V2 ((uint32_t)0x6b3343cfu)
//
// bindgen は単純なリテラルマクロ (#define FOO 42) は Rust 定数として生成できるが、
// キャスト式 ((uint32_t)...) を含むマクロは処理できないため、
// ここで同等の定数を独自に定義する。

/// QUIC v1 (RFC 9000)
pub const NGTCP2_PROTO_VER_V1: u32 = 0x00000001;
/// QUIC v2 (RFC 9369)
pub const NGTCP2_PROTO_VER_V2: u32 = 0x6b3343cf;

/// QUIC バージョン
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u32)]
pub enum QuicVersion {
    /// QUIC v1 (RFC 9000)
    #[default]
    V1 = NGTCP2_PROTO_VER_V1,
    /// QUIC v2 (RFC 9369)
    V2 = NGTCP2_PROTO_VER_V2,
}

// ============================================================================
// コネクション ID
// ============================================================================
//
// ngtcp2_cid は固定サイズ配列 (data: [u8; 20], datalen: usize) を持つ C 構造体。
// Rust では Vec<u8> を使用することで:
// - 可変長データの自然な表現
// - Clone, PartialEq, Eq, Hash の derive が可能
// - メモリ安全なインターフェース
// を提供する。

/// コネクション ID
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ConnectionId {
    data: Vec<u8>,
}

impl ConnectionId {
    /// 新しいコネクション ID を作成
    pub fn new(data: &[u8]) -> Option<Self> {
        let min = ngtcp2_sys::NGTCP2_MIN_CIDLEN as usize;
        let max = ngtcp2_sys::NGTCP2_MAX_CIDLEN as usize;
        if data.len() < min || data.len() > max {
            return None;
        }
        Some(Self {
            data: data.to_vec(),
        })
    }

    /// ランダムなコネクション ID を生成
    pub fn random(len: usize) -> Option<Self> {
        let min = ngtcp2_sys::NGTCP2_MIN_CIDLEN as usize;
        let max = ngtcp2_sys::NGTCP2_MAX_CIDLEN as usize;
        if len < min || len > max {
            return None;
        }
        let mut data = vec![0u8; len];
        aws_lc_rs::rand::fill(&mut data).ok()?;
        Some(Self { data })
    }

    /// バイト列として取得
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// 長さを取得
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// 空かどうか
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

impl std::fmt::Debug for ConnectionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ConnectionId(")?;
        for b in &self.data {
            write!(f, "{:02x}", b)?;
        }
        write!(f, ")")
    }
}

impl std::fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for b in &self.data {
            write!(f, "{:02x}", b)?;
        }
        Ok(())
    }
}

// ============================================================================
// パス情報
// ============================================================================
//
// ngtcp2_path は ngtcp2_addr 構造体へのポインタを持ち、
// ngtcp2_addr は sockaddr* (生ポインタ) を使用する。
// Rust の SocketAddr を使用することで型安全なインターフェースを提供する。

/// パス情報
#[derive(Debug, Clone)]
pub struct PathInfo {
    /// ローカルアドレス
    pub local: SocketAddr,
    /// リモートアドレス
    pub remote: SocketAddr,
}

// ============================================================================
// パケット情報
// ============================================================================
//
// ngtcp2_pkt_info は ecn フィールドのみを持つ単純な構造体。
// FFI 境界を超えるため、独自の Rust 構造体として再定義し、
// Default trait などの Rust 慣用的なインターフェースを提供する。

/// パケット情報
#[derive(Debug, Clone, Copy, Default)]
pub struct PacketInfo {
    /// ECN マーキング
    pub ecn: u8,
}

// ============================================================================
// ストリーム関連の型
// ============================================================================
//
// ngtcp2/nghttp3 はストリーム ID として int64_t を使用する。
// 型エイリアスにより可読性を向上させる。
//
// ストリームタイプと方向は ngtcp2 ではストリーム ID のビットフラグで判定する:
// - bit 0: 0 = クライアント開始, 1 = サーバー開始
// - bit 1: 0 = 双方向, 1 = 単方向
// Rust の enum で型安全に表現することで、誤用を防止する。

/// ストリーム ID
pub type StreamId = i64;

/// ストリームタイプ
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamType {
    /// 双方向ストリーム
    Bidirectional,
    /// 単方向ストリーム
    Unidirectional,
}

impl StreamType {
    /// ストリーム ID からタイプを判定
    pub fn from_stream_id(stream_id: StreamId) -> Self {
        if stream_id & 0x2 == 0 {
            Self::Bidirectional
        } else {
            Self::Unidirectional
        }
    }
}

/// ストリーム方向 (クライアント開始 or サーバー開始)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamDirection {
    /// クライアント開始
    ClientInitiated,
    /// サーバー開始
    ServerInitiated,
}

impl StreamDirection {
    /// ストリーム ID から方向を判定
    pub fn from_stream_id(stream_id: StreamId) -> Self {
        if stream_id & 0x1 == 0 {
            Self::ClientInitiated
        } else {
            Self::ServerInitiated
        }
    }
}

// ============================================================================
// HTTP/3 ヘッダー
// ============================================================================
//
// nghttp3_nv は生ポインタ (name, value が uint8_t*) と長さを別々に持つ C 構造体。
// Rust では Vec<u8> を使用することで:
// - メモリ安全性の保証
// - 所有権の明確化
// - Clone や Debug の自然な実装
// を提供する。また、疑似ヘッダー (:method, :path 等) を作成する
// ヘルパーメソッドも追加している。

/// HTTP/3 ヘッダー
#[derive(Debug, Clone)]
pub struct Header {
    /// ヘッダー名
    pub name: Vec<u8>,
    /// ヘッダー値
    pub value: Vec<u8>,
}

impl Header {
    /// 新しいヘッダーを作成
    pub fn new(name: impl Into<Vec<u8>>, value: impl Into<Vec<u8>>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }

    /// 疑似ヘッダー :method
    pub fn method(method: &str) -> Self {
        Self::new(b":method".to_vec(), method.as_bytes().to_vec())
    }

    /// 疑似ヘッダー :scheme
    pub fn scheme(scheme: &str) -> Self {
        Self::new(b":scheme".to_vec(), scheme.as_bytes().to_vec())
    }

    /// 疑似ヘッダー :authority
    pub fn authority(authority: &str) -> Self {
        Self::new(b":authority".to_vec(), authority.as_bytes().to_vec())
    }

    /// 疑似ヘッダー :path
    pub fn path(path: &str) -> Self {
        Self::new(b":path".to_vec(), path.as_bytes().to_vec())
    }

    /// 疑似ヘッダー :status
    pub fn status(status: u16) -> Self {
        Self::new(b":status".to_vec(), status.to_string().into_bytes())
    }

    /// ヘッダー名を文字列として取得
    pub fn name_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.name).ok()
    }

    /// ヘッダー値を文字列として取得
    pub fn value_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.value).ok()
    }
}

// ============================================================================
// WebTransport セッション ID
// ============================================================================
//
// nghttp3 は WebTransport セッション ID として int64_t を使用する。
// 型エイリアスにより可読性を向上させる。

/// WebTransport セッション ID
pub type SessionId = i64;

// ============================================================================
// HTTP/3 イベント
// ============================================================================
//
// nghttp3 はコールバックベースの API を提供する。
// コールバック内でイベントを enum として蓄積し、後からポーリングで取得する
// パターンを採用することで:
// - 非同期処理との統合が容易
// - Sans-I/O アーキテクチャとの親和性
// - イベント駆動型プログラミングの実現
// を可能にする。

/// HTTP/3 イベント
#[derive(Debug)]
pub enum Http3Event {
    /// ヘッダー受信開始
    HeadersBegin { stream_id: StreamId },
    /// ヘッダー受信
    Header { stream_id: StreamId, header: Header },
    /// ヘッダー受信完了
    HeadersEnd { stream_id: StreamId, fin: bool },
    /// データ受信
    Data { stream_id: StreamId, data: Vec<u8> },
    /// ストリーム終了
    StreamEnd { stream_id: StreamId },
    /// ストリームクローズ
    StreamClose {
        stream_id: StreamId,
        error_code: u64,
    },
    /// リセット
    Reset {
        stream_id: StreamId,
        error_code: u64,
    },
    /// トレーラー受信開始
    TrailersBegin { stream_id: StreamId },
    /// トレーラー受信
    Trailer { stream_id: StreamId, header: Header },
    /// トレーラー受信完了
    TrailersEnd { stream_id: StreamId },
    /// WebTransport データ受信
    WebTransportData {
        session_id: SessionId,
        stream_id: StreamId,
        data: Vec<u8>,
    },
}
