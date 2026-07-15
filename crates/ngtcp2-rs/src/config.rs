//! FFI 設定構造体の構築ヘルパー
//!
//! ngtcp2-sys / nghttp3-sys の設定構造体は orphan rule により inherent method を
//! 実装できないため、本モジュールに free function とビルダー用 newtype を置く。

use nghttp3_sys::nghttp3_settings;
use ngtcp2_sys::ngtcp2_transport_params;

use crate::types::ConnectionId;

/// `nghttp3_settings` のビルダー
///
/// FFI 構造体はポインタフィールドを含み得るため `Copy` は付けない。
#[derive(Clone)]
pub struct Http3Settings(nghttp3_settings);

impl Http3Settings {
    /// デフォルト設定を作成する
    pub fn new() -> Self {
        // nghttp3_settings_default_versioned で初期化してから上書きする。
        // nghttp3 のバージョンアップでフィールドが追加されても初期化漏れを防げる。
        let mut settings: nghttp3_settings = unsafe { std::mem::zeroed() };
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
        Self(settings)
    }

    /// WebTransport サポートを有効化する
    pub fn with_webtransport(mut self) -> Self {
        self.0.enable_connect_protocol = 1;
        self.0.h3_datagram = 1;
        self.0.wt_enabled = 1;
        self
    }

    /// 最大ヘッダーリストサイズを設定する
    pub fn with_max_field_section_size(mut self, size: u64) -> Self {
        self.0.max_field_section_size = size;
        self
    }

    /// WebTransport を有効化する
    pub fn with_wt_enabled(mut self, enabled: u8) -> Self {
        self.0.wt_enabled = enabled;
        self
    }

    /// 内側の FFI 構造体への参照を返す
    pub fn as_raw(&self) -> &nghttp3_settings {
        &self.0
    }

    /// 内側の FFI 構造体を取り出す
    pub fn into_raw(self) -> nghttp3_settings {
        self.0
    }
}

impl Default for Http3Settings {
    fn default() -> Self {
        Self::new()
    }
}

impl AsRef<nghttp3_settings> for Http3Settings {
    fn as_ref(&self) -> &nghttp3_settings {
        &self.0
    }
}

/// `ngtcp2_transport_params` のビルダー
///
/// FFI 構造体はポインタフィールドを含み得るため `Copy` は付けない。
#[derive(Clone)]
pub struct TransportParams(ngtcp2_transport_params);

impl TransportParams {
    /// デフォルト設定を作成する
    pub fn new() -> Self {
        let mut params: ngtcp2_transport_params = unsafe { std::mem::zeroed() };
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
        Self(params)
    }

    /// 最大アイドルタイムアウトを設定する (ナノ秒)
    pub fn with_max_idle_timeout(mut self, timeout_ns: u64) -> Self {
        self.0.max_idle_timeout = timeout_ns;
        self
    }

    /// 初期の最大データ量を設定する
    pub fn with_initial_max_data(mut self, max_data: u64) -> Self {
        self.0.initial_max_data = max_data;
        self
    }

    /// 最大双方向ストリーム数を設定する
    pub fn with_max_streams_bidi(mut self, max_streams: u64) -> Self {
        self.0.initial_max_streams_bidi = max_streams;
        self
    }

    /// 最大単方向ストリーム数を設定する
    pub fn with_max_streams_uni(mut self, max_streams: u64) -> Self {
        self.0.initial_max_streams_uni = max_streams;
        self
    }

    /// DATAGRAM を有効化する
    pub fn with_datagram(mut self, max_size: u64) -> Self {
        self.0.max_datagram_frame_size = max_size;
        self
    }

    /// original_dcid を設定する (サーバー用)
    ///
    /// サーバーは、クライアントからの最初の Initial パケットの
    /// Destination Connection ID をこのフィールドに設定する必要がある。
    pub fn with_original_dcid(mut self, dcid: &ConnectionId) -> Self {
        self.0.original_dcid.datalen = dcid.len();
        self.0.original_dcid.data[..dcid.len()].copy_from_slice(dcid.as_bytes());
        self.0.original_dcid_present = 1;
        self
    }

    /// 既存の FFI 構造体をラップする
    pub fn from_raw(params: ngtcp2_transport_params) -> Self {
        Self(params)
    }

    /// 内側の FFI 構造体への参照を返す
    pub fn as_raw(&self) -> &ngtcp2_transport_params {
        &self.0
    }

    /// 内側の FFI 構造体を取り出す
    pub fn into_raw(self) -> ngtcp2_transport_params {
        self.0
    }
}

impl Default for TransportParams {
    fn default() -> Self {
        Self::new()
    }
}

impl AsRef<ngtcp2_transport_params> for TransportParams {
    fn as_ref(&self) -> &ngtcp2_transport_params {
        &self.0
    }
}
