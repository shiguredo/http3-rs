//! QPACK 静的テーブル (RFC 9204 Appendix A)
//!
//! 99 エントリの静的テーブルを提供。

/// 静的テーブルエントリ
#[derive(Debug, Clone, Copy)]
pub struct StaticEntry {
    /// ヘッダー名
    pub name: &'static [u8],
    /// ヘッダー値
    pub value: &'static [u8],
}

/// 静的テーブル (99 エントリ)
pub static STATIC_TABLE: &[StaticEntry] = &[
    // 0
    StaticEntry {
        name: b":authority",
        value: b"",
    },
    // 1
    StaticEntry {
        name: b":path",
        value: b"/",
    },
    // 2
    StaticEntry {
        name: b"age",
        value: b"0",
    },
    // 3
    StaticEntry {
        name: b"content-disposition",
        value: b"",
    },
    // 4
    StaticEntry {
        name: b"content-length",
        value: b"0",
    },
    // 5
    StaticEntry {
        name: b"cookie",
        value: b"",
    },
    // 6
    StaticEntry {
        name: b"date",
        value: b"",
    },
    // 7
    StaticEntry {
        name: b"etag",
        value: b"",
    },
    // 8
    StaticEntry {
        name: b"if-modified-since",
        value: b"",
    },
    // 9
    StaticEntry {
        name: b"if-none-match",
        value: b"",
    },
    // 10
    StaticEntry {
        name: b"last-modified",
        value: b"",
    },
    // 11
    StaticEntry {
        name: b"link",
        value: b"",
    },
    // 12
    StaticEntry {
        name: b"location",
        value: b"",
    },
    // 13
    StaticEntry {
        name: b"referer",
        value: b"",
    },
    // 14
    StaticEntry {
        name: b"set-cookie",
        value: b"",
    },
    // 15
    StaticEntry {
        name: b":method",
        value: b"CONNECT",
    },
    // 16
    StaticEntry {
        name: b":method",
        value: b"DELETE",
    },
    // 17
    StaticEntry {
        name: b":method",
        value: b"GET",
    },
    // 18
    StaticEntry {
        name: b":method",
        value: b"HEAD",
    },
    // 19
    StaticEntry {
        name: b":method",
        value: b"OPTIONS",
    },
    // 20
    StaticEntry {
        name: b":method",
        value: b"POST",
    },
    // 21
    StaticEntry {
        name: b":method",
        value: b"PUT",
    },
    // 22
    StaticEntry {
        name: b":scheme",
        value: b"http",
    },
    // 23
    StaticEntry {
        name: b":scheme",
        value: b"https",
    },
    // 24
    StaticEntry {
        name: b":status",
        value: b"103",
    },
    // 25
    StaticEntry {
        name: b":status",
        value: b"200",
    },
    // 26
    StaticEntry {
        name: b":status",
        value: b"304",
    },
    // 27
    StaticEntry {
        name: b":status",
        value: b"404",
    },
    // 28
    StaticEntry {
        name: b":status",
        value: b"503",
    },
    // 29
    StaticEntry {
        name: b"accept",
        value: b"*/*",
    },
    // 30
    StaticEntry {
        name: b"accept",
        value: b"application/dns-message",
    },
    // 31
    StaticEntry {
        name: b"accept-encoding",
        value: b"gzip, deflate, br",
    },
    // 32
    StaticEntry {
        name: b"accept-ranges",
        value: b"bytes",
    },
    // 33
    StaticEntry {
        name: b"access-control-allow-headers",
        value: b"cache-control",
    },
    // 34
    StaticEntry {
        name: b"access-control-allow-headers",
        value: b"content-type",
    },
    // 35
    StaticEntry {
        name: b"access-control-allow-origin",
        value: b"*",
    },
    // 36
    StaticEntry {
        name: b"cache-control",
        value: b"max-age=0",
    },
    // 37
    StaticEntry {
        name: b"cache-control",
        value: b"max-age=2592000",
    },
    // 38
    StaticEntry {
        name: b"cache-control",
        value: b"max-age=604800",
    },
    // 39
    StaticEntry {
        name: b"cache-control",
        value: b"no-cache",
    },
    // 40
    StaticEntry {
        name: b"cache-control",
        value: b"no-store",
    },
    // 41
    StaticEntry {
        name: b"cache-control",
        value: b"public, max-age=31536000",
    },
    // 42
    StaticEntry {
        name: b"content-encoding",
        value: b"br",
    },
    // 43
    StaticEntry {
        name: b"content-encoding",
        value: b"gzip",
    },
    // 44
    StaticEntry {
        name: b"content-type",
        value: b"application/dns-message",
    },
    // 45
    StaticEntry {
        name: b"content-type",
        value: b"application/javascript",
    },
    // 46
    StaticEntry {
        name: b"content-type",
        value: b"application/json",
    },
    // 47
    StaticEntry {
        name: b"content-type",
        value: b"application/x-www-form-urlencoded",
    },
    // 48
    StaticEntry {
        name: b"content-type",
        value: b"image/gif",
    },
    // 49
    StaticEntry {
        name: b"content-type",
        value: b"image/jpeg",
    },
    // 50
    StaticEntry {
        name: b"content-type",
        value: b"image/png",
    },
    // 51
    StaticEntry {
        name: b"content-type",
        value: b"text/css",
    },
    // 52
    StaticEntry {
        name: b"content-type",
        value: b"text/html; charset=utf-8",
    },
    // 53
    StaticEntry {
        name: b"content-type",
        value: b"text/plain",
    },
    // 54
    StaticEntry {
        name: b"content-type",
        value: b"text/plain;charset=utf-8",
    },
    // 55
    StaticEntry {
        name: b"range",
        value: b"bytes=0-",
    },
    // 56
    StaticEntry {
        name: b"strict-transport-security",
        value: b"max-age=31536000",
    },
    // 57
    StaticEntry {
        name: b"strict-transport-security",
        value: b"max-age=31536000; includesubdomains",
    },
    // 58
    StaticEntry {
        name: b"strict-transport-security",
        value: b"max-age=31536000; includesubdomains; preload",
    },
    // 59
    StaticEntry {
        name: b"vary",
        value: b"accept-encoding",
    },
    // 60
    StaticEntry {
        name: b"vary",
        value: b"origin",
    },
    // 61
    StaticEntry {
        name: b"x-content-type-options",
        value: b"nosniff",
    },
    // 62
    StaticEntry {
        name: b"x-xss-protection",
        value: b"1; mode=block",
    },
    // 63
    StaticEntry {
        name: b":status",
        value: b"100",
    },
    // 64
    StaticEntry {
        name: b":status",
        value: b"204",
    },
    // 65
    StaticEntry {
        name: b":status",
        value: b"206",
    },
    // 66
    StaticEntry {
        name: b":status",
        value: b"302",
    },
    // 67
    StaticEntry {
        name: b":status",
        value: b"400",
    },
    // 68
    StaticEntry {
        name: b":status",
        value: b"403",
    },
    // 69
    StaticEntry {
        name: b":status",
        value: b"421",
    },
    // 70
    StaticEntry {
        name: b":status",
        value: b"425",
    },
    // 71
    StaticEntry {
        name: b":status",
        value: b"500",
    },
    // 72
    StaticEntry {
        name: b"accept-language",
        value: b"",
    },
    // 73
    StaticEntry {
        name: b"access-control-allow-credentials",
        value: b"FALSE",
    },
    // 74
    StaticEntry {
        name: b"access-control-allow-credentials",
        value: b"TRUE",
    },
    // 75
    StaticEntry {
        name: b"access-control-allow-headers",
        value: b"*",
    },
    // 76
    StaticEntry {
        name: b"access-control-allow-methods",
        value: b"get",
    },
    // 77
    StaticEntry {
        name: b"access-control-allow-methods",
        value: b"get, post, options",
    },
    // 78
    StaticEntry {
        name: b"access-control-allow-methods",
        value: b"options",
    },
    // 79
    StaticEntry {
        name: b"access-control-expose-headers",
        value: b"content-length",
    },
    // 80
    StaticEntry {
        name: b"access-control-request-headers",
        value: b"content-type",
    },
    // 81
    StaticEntry {
        name: b"access-control-request-method",
        value: b"get",
    },
    // 82
    StaticEntry {
        name: b"access-control-request-method",
        value: b"post",
    },
    // 83
    StaticEntry {
        name: b"alt-svc",
        value: b"clear",
    },
    // 84
    StaticEntry {
        name: b"authorization",
        value: b"",
    },
    // 85
    StaticEntry {
        name: b"content-security-policy",
        value: b"script-src 'none'; object-src 'none'; base-uri 'none'",
    },
    // 86
    StaticEntry {
        name: b"early-data",
        value: b"1",
    },
    // 87
    StaticEntry {
        name: b"expect-ct",
        value: b"",
    },
    // 88
    StaticEntry {
        name: b"forwarded",
        value: b"",
    },
    // 89
    StaticEntry {
        name: b"if-range",
        value: b"",
    },
    // 90
    StaticEntry {
        name: b"origin",
        value: b"",
    },
    // 91
    StaticEntry {
        name: b"purpose",
        value: b"prefetch",
    },
    // 92
    StaticEntry {
        name: b"server",
        value: b"",
    },
    // 93
    StaticEntry {
        name: b"timing-allow-origin",
        value: b"*",
    },
    // 94
    StaticEntry {
        name: b"upgrade-insecure-requests",
        value: b"1",
    },
    // 95
    StaticEntry {
        name: b"user-agent",
        value: b"",
    },
    // 96
    StaticEntry {
        name: b"x-forwarded-for",
        value: b"",
    },
    // 97
    StaticEntry {
        name: b"x-frame-options",
        value: b"deny",
    },
    // 98
    StaticEntry {
        name: b"x-frame-options",
        value: b"sameorigin",
    },
];

/// 静的テーブルのエントリ数
pub const STATIC_TABLE_LEN: usize = 99;

/// インデックスから静的テーブルエントリを取得
#[inline]
pub fn get_static_entry(index: usize) -> Option<&'static StaticEntry> {
    STATIC_TABLE.get(index)
}

/// 名前と値のペアで静的テーブルを検索
///
/// 完全一致のインデックス、または名前のみ一致のインデックスを返す
pub fn find_static_entry(name: &[u8], value: &[u8]) -> (Option<usize>, Option<usize>) {
    let mut name_match = None;

    for (index, entry) in STATIC_TABLE.iter().enumerate() {
        if entry.name == name {
            if entry.value == value {
                return (Some(index), Some(index));
            }
            if name_match.is_none() {
                name_match = Some(index);
            }
        }
    }

    (None, name_match)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_static_table_len() {
        assert_eq!(STATIC_TABLE.len(), STATIC_TABLE_LEN);
        assert_eq!(STATIC_TABLE_LEN, 99);
    }

    #[test]
    fn test_get_static_entry() {
        let entry = get_static_entry(0).unwrap();
        assert_eq!(entry.name, b":authority");
        assert_eq!(entry.value, b"");

        let entry = get_static_entry(17).unwrap();
        assert_eq!(entry.name, b":method");
        assert_eq!(entry.value, b"GET");

        let entry = get_static_entry(25).unwrap();
        assert_eq!(entry.name, b":status");
        assert_eq!(entry.value, b"200");

        let entry = get_static_entry(98).unwrap();
        assert_eq!(entry.name, b"x-frame-options");
        assert_eq!(entry.value, b"sameorigin");

        assert!(get_static_entry(99).is_none());
    }

    #[test]
    fn test_find_static_entry_exact_match() {
        let (exact, name_only) = find_static_entry(b":method", b"GET");
        assert_eq!(exact, Some(17));
        assert_eq!(name_only, Some(17));

        let (exact, name_only) = find_static_entry(b":status", b"200");
        assert_eq!(exact, Some(25));
        assert_eq!(name_only, Some(25));
    }

    #[test]
    fn test_find_static_entry_name_only() {
        let (exact, name_only) = find_static_entry(b":method", b"PATCH");
        assert_eq!(exact, None);
        assert_eq!(name_only, Some(15)); // First :method entry

        let (exact, name_only) = find_static_entry(b":status", b"201");
        assert_eq!(exact, None);
        assert_eq!(name_only, Some(24)); // First :status entry
    }

    #[test]
    fn test_find_static_entry_not_found() {
        let (exact, name_only) = find_static_entry(b"x-custom-header", b"value");
        assert_eq!(exact, None);
        assert_eq!(name_only, None);
    }
}
