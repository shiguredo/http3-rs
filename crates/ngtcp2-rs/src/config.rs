//! 設定の拡張トレイト
//!
//! ngtcp2-sys / nghttp3-sys の設定構造体に対する拡張トレイトを提供する。
//!
//! # なぜ拡張トレイトを使用するのか
//!
//! Rust では外部クレートで定義された型に対して直接メソッドを実装することができない
//! (orphan rule)。ngtcp2_transport_params や nghttp3_settings は *-sys クレートで
//! bindgen により生成された FFI 構造体であるため、拡張トレイトパターンを使用して
//! ビルダースタイルの設定 API を提供する。
//!
//! これにより:
//!
//! - `ngtcp2_transport_params::default_params().with_datagram(65535)` のような流暢な API が可能
//! - FFI 構造体の初期化に必要なボイラープレートを隠蔽できる
//! - ngtcp2_transport_params_default_versioned などの versioned API を適切に呼び出せる

use nghttp3_sys::nghttp3_settings;
use ngtcp2_sys::ngtcp2_transport_params;

use crate::types::ConnectionId;

/// nghttp3_settings の拡張トレイト
pub trait Http3SettingsExt {
    /// デフォルト設定を作成
    fn default_settings() -> Self;

    /// WebTransport サポートを有効化
    fn with_webtransport(self) -> Self;

    /// 最大ヘッダーリストサイズを設定
    fn with_max_field_section_size(self, size: u64) -> Self;

    /// WebTransport を有効化
    fn with_wt_enabled(self, enabled: u8) -> Self;
}

impl Http3SettingsExt for nghttp3_settings {
    fn default_settings() -> Self {
        // nghttp3_settings_default_versioned で初期化してから上書きする。
        // nghttp3 のバージョンアップでフィールドが追加されても初期化漏れを防げる。
        let mut settings: Self = unsafe { std::mem::zeroed() };
        unsafe {
            nghttp3_sys::nghttp3_settings_default_versioned(
                nghttp3_sys::NGHTTP3_SETTINGS_VERSION as i32,
                &mut settings,
            );
        }
        settings.max_field_section_size = 64 * 1024; // 64 KB
        settings.qpack_max_dtable_capacity = 4096;
        settings.qpack_encoder_max_dtable_capacity = 4096;
        settings.qpack_blocked_streams = 100;
        settings
    }

    fn with_webtransport(mut self) -> Self {
        self.enable_connect_protocol = 1;
        self.h3_datagram = 1;
        self.wt_enabled = 1;
        self
    }

    fn with_max_field_section_size(mut self, size: u64) -> Self {
        self.max_field_section_size = size;
        self
    }

    fn with_wt_enabled(mut self, enabled: u8) -> Self {
        self.wt_enabled = enabled;
        self
    }
}

/// ngtcp2_transport_params の拡張トレイト
pub trait TransportParamsExt {
    /// デフォルト設定を作成
    fn default_params() -> Self;

    /// 最大アイドルタイムアウトを設定 (ナノ秒)
    fn with_max_idle_timeout(self, timeout_ns: u64) -> Self;

    /// 初期の最大データ量を設定
    fn with_initial_max_data(self, max_data: u64) -> Self;

    /// 最大双方向ストリーム数を設定
    fn with_max_streams_bidi(self, max_streams: u64) -> Self;

    /// 最大単方向ストリーム数を設定
    fn with_max_streams_uni(self, max_streams: u64) -> Self;

    /// DATAGRAM を有効化
    fn with_datagram(self, max_size: u64) -> Self;

    /// original_dcid を設定 (サーバー用)
    ///
    /// サーバーは、クライアントからの最初の Initial パケットの
    /// Destination Connection ID をこのフィールドに設定する必要がある。
    fn with_original_dcid(self, dcid: &ConnectionId) -> Self;
}

impl TransportParamsExt for ngtcp2_transport_params {
    fn default_params() -> Self {
        // ngtcp2_transport_params_default を使用してデフォルト値を取得
        let mut params: Self = unsafe { std::mem::zeroed() };
        unsafe {
            ngtcp2_sys::ngtcp2_transport_params_default_versioned(
                ngtcp2_sys::NGTCP2_TRANSPORT_PARAMS_VERSION as i32,
                &mut params,
            );
        }
        // アプリケーション固有の設定を上書き
        params.initial_max_stream_data_bidi_local = 1024 * 1024; // 1 MB
        params.initial_max_stream_data_bidi_remote = 1024 * 1024; // 1 MB
        params.initial_max_stream_data_uni = 1024 * 1024; // 1 MB
        params.initial_max_data = 10 * 1024 * 1024; // 10 MB
        params.initial_max_streams_bidi = 100;
        params.initial_max_streams_uni = 100;
        params.max_idle_timeout = 30 * 1000 * 1000 * 1000; // 30 秒 (ナノ秒)
        params.active_connection_id_limit = 8;
        params
    }

    fn with_max_idle_timeout(mut self, timeout_ns: u64) -> Self {
        self.max_idle_timeout = timeout_ns;
        self
    }

    fn with_initial_max_data(mut self, max_data: u64) -> Self {
        self.initial_max_data = max_data;
        self
    }

    fn with_max_streams_bidi(mut self, max_streams: u64) -> Self {
        self.initial_max_streams_bidi = max_streams;
        self
    }

    fn with_max_streams_uni(mut self, max_streams: u64) -> Self {
        self.initial_max_streams_uni = max_streams;
        self
    }

    fn with_datagram(mut self, max_size: u64) -> Self {
        self.max_datagram_frame_size = max_size;
        self
    }

    fn with_original_dcid(mut self, dcid: &ConnectionId) -> Self {
        self.original_dcid.datalen = dcid.len();
        self.original_dcid.data[..dcid.len()].copy_from_slice(dcid.as_bytes());
        self.original_dcid_present = 1;
        self
    }
}
