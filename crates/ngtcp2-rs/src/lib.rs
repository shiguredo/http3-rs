//! shiguredo_ngtcp2 - ngtcp2/nghttp3 Rust バインディング
//!
//! ngtcp2 (QUIC プロトコル実装) と nghttp3 (HTTP/3 + WebTransport 実装) の
//! Rust バインディングを提供する。

mod config;
mod conn;
mod crypto;
mod error;
mod h3;
mod types;
pub mod varint;

pub use config::{Http3SettingsExt, TransportParamsExt};
pub use conn::{Connection, Datagram, StreamData};
pub use crypto::{TlsContext, TlsSession};
pub use error::{Error, Result};
pub use h3::Http3Connection;
pub use types::{
    ConnectionId, Header, Http3Event, PacketInfo, PathInfo, QuicVersion, SessionId,
    StreamDirection, StreamId, StreamType,
};

// ngtcp2-sys / nghttp3-sys の型を再エクスポート
pub use nghttp3_sys::{nghttp3_settings, nghttp3_vec};
pub use ngtcp2_sys::ngtcp2_transport_params;

/// ngtcp2 のバージョン文字列を取得
pub fn ngtcp2_version() -> &'static str {
    unsafe {
        let info = ngtcp2_sys::ngtcp2_version(0);
        if info.is_null() {
            "unknown"
        } else {
            let version_str = (*info).version_str;
            if version_str.is_null() {
                "unknown"
            } else {
                std::ffi::CStr::from_ptr(version_str)
                    .to_str()
                    .unwrap_or("unknown")
            }
        }
    }
}

/// nghttp3 のバージョン文字列を取得
pub fn nghttp3_version() -> &'static str {
    unsafe {
        let info = nghttp3_sys::nghttp3_version(0);
        if info.is_null() {
            "unknown"
        } else {
            let version_str = (*info).version_str;
            if version_str.is_null() {
                "unknown"
            } else {
                std::ffi::CStr::from_ptr(version_str)
                    .to_str()
                    .unwrap_or("unknown")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connection_id_new() {
        let cid = ConnectionId::new(&[1, 2, 3, 4]).unwrap();
        assert_eq!(cid.len(), 4);
        assert_eq!(cid.as_bytes(), &[1, 2, 3, 4]);
    }

    #[test]
    fn test_connection_id_random() {
        let cid = ConnectionId::random(16).unwrap();
        assert_eq!(cid.len(), 16);
    }

    #[test]
    fn test_connection_id_too_long() {
        let data = vec![0u8; 21];
        assert!(ConnectionId::new(&data).is_none());
    }

    #[test]
    fn test_stream_type() {
        // Client-initiated bidirectional
        assert_eq!(StreamType::from_stream_id(0), StreamType::Bidirectional);
        assert_eq!(
            StreamDirection::from_stream_id(0),
            StreamDirection::ClientInitiated
        );

        // Server-initiated bidirectional
        assert_eq!(StreamType::from_stream_id(1), StreamType::Bidirectional);
        assert_eq!(
            StreamDirection::from_stream_id(1),
            StreamDirection::ServerInitiated
        );

        // Client-initiated unidirectional
        assert_eq!(StreamType::from_stream_id(2), StreamType::Unidirectional);
        assert_eq!(
            StreamDirection::from_stream_id(2),
            StreamDirection::ClientInitiated
        );

        // Server-initiated unidirectional
        assert_eq!(StreamType::from_stream_id(3), StreamType::Unidirectional);
        assert_eq!(
            StreamDirection::from_stream_id(3),
            StreamDirection::ServerInitiated
        );
    }

    #[test]
    fn test_header() {
        let header = Header::method("GET");
        assert_eq!(header.name_str(), Some(":method"));
        assert_eq!(header.value_str(), Some("GET"));

        let header = Header::status(200);
        assert_eq!(header.name_str(), Some(":status"));
        assert_eq!(header.value_str(), Some("200"));
    }

    #[test]
    fn test_transport_params_default() {
        let params = ngtcp2_transport_params::default_params();
        assert_eq!(params.initial_max_streams_bidi, 100);
        assert_eq!(params.initial_max_data, 10 * 1024 * 1024);
    }

    #[test]
    fn test_http3_settings_default() {
        let settings = nghttp3_settings::default_settings();
        assert_eq!(settings.max_field_section_size, 64 * 1024);
        assert_eq!(settings.enable_connect_protocol, 0);
    }

    #[test]
    fn test_http3_settings_webtransport() {
        let settings = nghttp3_settings::default_settings().with_webtransport();
        assert_eq!(settings.enable_connect_protocol, 1);
        assert_eq!(settings.h3_datagram, 1);
    }

    #[test]
    fn test_transport_params_datagram() {
        let params = ngtcp2_transport_params::default_params().with_datagram(65535);
        assert_eq!(params.max_datagram_frame_size, 65535);
    }

    #[test]
    fn test_datagram_struct() {
        let datagram = Datagram {
            data: vec![1, 2, 3, 4],
        };
        assert_eq!(datagram.data, vec![1, 2, 3, 4]);
    }
}
