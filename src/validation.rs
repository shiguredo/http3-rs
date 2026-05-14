//! HTTP/3 メッセージ検証 (RFC 9114 Section 4.1.2)
//!
//! 送受信するヘッダーセクションが malformed でないかを検証する。

use crate::connection::Role;
use crate::error::{Error, ErrorCode};
use crate::qpack::DecodedHeader;
use crate::qpack::Header;

/// ヘッダーフィールドの名前と値へのアクセスを提供するトレイト
///
/// `Header` (送信用) と `DecodedHeader` (受信用) の両方で検証関数を共用するために使用する。
pub trait HeaderField {
    /// ヘッダー名を返す
    fn name(&self) -> &[u8];
    /// ヘッダー値を返す
    fn value(&self) -> &[u8];
}

impl HeaderField for Header {
    fn name(&self) -> &[u8] {
        &self.name
    }
    fn value(&self) -> &[u8] {
        &self.value
    }
}

impl HeaderField for DecodedHeader {
    fn name(&self) -> &[u8] {
        &self.name
    }
    fn value(&self) -> &[u8] {
        &self.value
    }
}

/// 接続固有フィールド (RFC 9114 Section 4.2)
///
/// HTTP/3 では接続固有フィールドは禁止されている。
const CONNECTION_SPECIFIC_FIELDS: &[&[u8]] = &[
    b"connection",
    b"keep-alive",
    b"proxy-connection",
    b"transfer-encoding",
    b"upgrade",
];

/// HTTP token の 1 バイトが tchar であるかを検証する (RFC 9110 Section 5.6.2)
///
/// token = 1*tchar
/// tchar = "!" / "#" / "$" / "%" / "&" / "'" / "*" / "+" / "-" / "." /
///         "^" / "_" / "`" / "|" / "~" / DIGIT / ALPHA
///
/// 大文字・小文字の両方を含む。method 等の検証に使用する。
fn is_tchar(b: u8) -> bool {
    matches!(b,
        b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+' | b'-' | b'.' |
        b'^' | b'_' | b'`' | b'|' | b'~' |
        b'0'..=b'9' |
        b'A'..=b'Z' |
        b'a'..=b'z'
    )
}

/// HTTP method が有効な token であるかを検証する (RFC 9110 Section 9.1)
///
/// method = token = 1*tchar
fn is_valid_method(method: &[u8]) -> bool {
    !method.is_empty() && method.iter().all(|&b| is_tchar(b))
}

/// URI scheme が RFC 3986 Section 3.1 の文法に準拠するかを検証する
///
/// scheme = ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )
fn is_valid_scheme(scheme: &[u8]) -> bool {
    if scheme.is_empty() {
        return false;
    }
    if !scheme[0].is_ascii_alphabetic() {
        return false;
    }
    scheme[1..]
        .iter()
        .all(|&b| b.is_ascii_alphanumeric() || b == b'+' || b == b'-' || b == b'.')
}

/// :path が http/https scheme で有効かを検証する (RFC 9114 Section 4.3.1)
///
/// path-absolute (RFC 3986 Section 3.3) = "/" *( "/" segment ) で始まるか、
/// OPTIONS リクエストの場合は "*" (RFC 9110 Section 7.1) も許可する。
fn is_valid_http_path(path: &[u8], method: &[u8]) -> bool {
    if path.is_empty() {
        return false;
    }
    // OPTIONS の asterisk-form
    if method == b"OPTIONS" && path == b"*" {
        return true;
    }
    // path-absolute: "/" で始まる
    path[0] == b'/'
}

/// フィールド名の 1 バイトが token 文字 (tchar) であるかを検証する (RFC 9110 Section 5.6.2)
///
/// token = 1*tchar
/// tchar = "!" / "#" / "$" / "%" / "&" / "'" / "*" / "+" / "-" / "." /
///         "^" / "_" / "`" / "|" / "~" / DIGIT / ALPHA
///
/// 大文字の ALPHA は HTTP/3 では禁止 (RFC 9114 Section 4.2) なので含めない。
fn is_valid_field_name_byte(b: u8) -> bool {
    matches!(b,
        b'!' | b'#' | b'$' | b'%' | b'&' | b'\'' | b'*' | b'+' | b'-' | b'.' |
        b'^' | b'_' | b'`' | b'|' | b'~' |
        b'0'..=b'9' |
        b'a'..=b'z'
    )
}

/// フィールド名全体が有効な token かを検証する (RFC 9110 Section 5.1, RFC 9114 Section 4.2)
///
/// - 空のフィールド名は不正
/// - 大文字を含むフィールド名は不正 (RFC 9114 Section 4.2)
/// - token 文字以外を含むフィールド名は不正 (RFC 9114 Section 10.3)
fn is_valid_field_name(name: &[u8]) -> bool {
    !name.is_empty() && name.iter().all(|&b| is_valid_field_name_byte(b))
}

/// フィールド値が field-content に準拠するかを検証する (RFC 9110 Section 5.5, RFC 9114 Section 10.3)
///
/// field-value = *field-content
/// field-content = field-vchar [ 1*( SP / HTAB / field-vchar ) field-vchar ]
/// field-vchar = VCHAR / obs-text
/// VCHAR = %x21-7E
/// obs-text = %x80-FF
///
/// 空のフィールド値は許可 (field-value = *field-content)。
/// 非空の場合、先頭と末尾は field-vchar でなければならない。
/// 途中では SP (0x20) / HTAB (0x09) も許可される。
fn is_valid_field_value(value: &[u8]) -> bool {
    if value.is_empty() {
        return true;
    }

    // field-vchar かどうかの判定
    let is_field_vchar = |b: u8| -> bool { matches!(b, 0x21..=0x7e | 0x80..=0xff) };

    // 先頭と末尾は field-vchar でなければならない
    if !is_field_vchar(value[0]) || !is_field_vchar(value[value.len() - 1]) {
        return false;
    }

    // 途中は field-vchar / SP / HTAB のみ許可
    value
        .iter()
        .all(|&b| is_field_vchar(b) || b == b' ' || b == b'\t')
}

/// URI-host の reg-name として妥当な文字かを判定する (RFC 3986 Section 3.2.2)
///
/// reg-name = *( unreserved / pct-encoded / sub-delims )
/// unreserved = ALPHA / DIGIT / "-" / "." / "_" / "~"
/// sub-delims = "!" / "$" / "&" / "'" / "(" / ")" / "*" / "+" / "," / ";" / "="
fn is_reg_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'-' | b'.'
                | b'_'
                | b'~'
                | b'!'
                | b'$'
                | b'&'
                | b'\''
                | b'('
                | b')'
                | b'*'
                | b'+'
                | b','
                | b';'
                | b'='
                | b'%'
        )
}

/// URI-host (reg-name) として妥当かを検証する (RFC 3986 Section 3.2.2)
///
/// pct-encoded の完全な検証 (%XX の 16 進数チェック) も行う。
fn is_valid_reg_name(host: &[u8]) -> bool {
    let mut i = 0;
    while i < host.len() {
        if host[i] == b'%' {
            // pct-encoded = "%" HEXDIG HEXDIG
            if i + 2 >= host.len()
                || !host[i + 1].is_ascii_hexdigit()
                || !host[i + 2].is_ascii_hexdigit()
            {
                return false;
            }
            i += 3;
        } else if is_reg_name_char(host[i]) {
            i += 1;
        } else {
            return false;
        }
    }
    true
}

/// IPv6address の括弧内が妥当な文字で構成されているかを検証する (RFC 3986 Section 3.2.2)
///
/// IP-literal = "[" ( IPv6address / IPvFuture ) "]"
/// 厳密な IPv6 アドレスパースは行わず、許可文字セット (HEXDIG / ":" / ".") のみ検証する。
fn is_valid_ip_literal_content(content: &[u8]) -> bool {
    if content.is_empty() {
        return false;
    }
    // IPvFuture = "v" 1*HEXDIG "." 1*( unreserved / sub-delims / ":" )
    if content[0] == b'v' {
        return content.len() >= 4
            && content[1..].contains(&b'.')
            && content[1..].iter().all(|&b| {
                b.is_ascii_hexdigit()
                    || b == b'.'
                    || b == b':'
                    || b == b'-'
                    || b == b'_'
                    || b == b'~'
                    || matches!(
                        b,
                        b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'='
                    )
            });
    }
    // IPv6address: HEXDIG / ":" / "." (IPv4-mapped 用)
    content
        .iter()
        .all(|&b| b.is_ascii_hexdigit() || b == b':' || b == b'.')
}

/// 非 CONNECT リクエストの :authority が URI authority として妥当かを検証する
/// (RFC 9114 Section 4.3.1, RFC 3986 Section 3.2)
///
/// authority = host [ ":" port ]
/// host = IP-literal / IPv4address / reg-name
/// ポートはオプション。userinfo は別途チェック済みのためここでは検証しない。
fn is_valid_authority(value: &[u8]) -> bool {
    if value.is_empty() {
        return false;
    }

    // IP-literal: [IPv6address] または [IPv6address]:port
    if value[0] == b'[' {
        let Some(bracket_end) = value.iter().position(|&b| b == b']') else {
            return false;
        };
        let literal_content = &value[1..bracket_end];
        if !is_valid_ip_literal_content(literal_content) {
            return false;
        }
        let rest = &value[bracket_end + 1..];
        if rest.is_empty() {
            return true;
        }
        // ]:port の形式
        if rest[0] != b':' {
            return false;
        }
        let port = &rest[1..];
        return port.iter().all(|b| b.is_ascii_digit());
    }

    // host[:port] の形式
    // ':' で分割してポート部分を検出する
    if let Some(colon_pos) = value.iter().rposition(|&b| b == b':') {
        let host = &value[..colon_pos];
        let port = &value[colon_pos + 1..];
        // ':' の後がすべて数字ならポート付き、そうでなければ ':' はホスト名の一部ではない
        // (reg-name に ':' は許可されないので、ポートとして解釈する)
        if !port.is_empty() && port.iter().all(|b| b.is_ascii_digit()) {
            return !host.is_empty() && is_valid_reg_name(host);
        }
    }

    // ポートなし: 全体がホスト名
    is_valid_reg_name(value)
}

/// plain CONNECT の :authority が authority-form (host:port) かを検証する
/// (RFC 9114 Section 4.4, RFC 9110 Section 7.1)
///
/// authority-form = uri-host ":" port
/// IPv6 の場合は [host]:port の形式
fn is_valid_connect_authority(value: &[u8]) -> bool {
    if value.is_empty() {
        return false;
    }

    // IPv6: [host]:port
    if value[0] == b'[' {
        // ']' を探す
        let Some(bracket_end) = value.iter().position(|&b| b == b']') else {
            return false;
        };
        // ']:port' の形式を期待
        if bracket_end + 1 >= value.len() || value[bracket_end + 1] != b':' {
            return false;
        }
        // IP-literal の中身を検証 (RFC 3986 Section 3.2.2)
        let literal_content = &value[1..bracket_end];
        if !is_valid_ip_literal_content(literal_content) {
            return false;
        }
        let port = &value[bracket_end + 2..];
        return !port.is_empty() && port.iter().all(|b| b.is_ascii_digit());
    }

    // IPv4 / ドメイン名: host:port
    // 最後の ':' で分割 (ホスト部に ':' は含まれない)
    let Some(colon_pos) = value.iter().rposition(|&b| b == b':') else {
        return false;
    };

    let host = &value[..colon_pos];
    let port = &value[colon_pos + 1..];

    // host と port がそれぞれ空でないこと、host が URI-host として妥当であること
    !host.is_empty()
        && !port.is_empty()
        && is_valid_reg_name(host)
        && port.iter().all(|b| b.is_ascii_digit())
}

/// リクエストヘッダーを検証 (RFC 9114 Section 4.1.2, 4.3.1, 4.4)
pub fn validate_request_headers<H: HeaderField>(headers: &[H]) -> Result<(), Error> {
    let mut method: Option<&[u8]> = None;
    let mut scheme: Option<&[u8]> = None;
    let mut path: Option<&[u8]> = None;
    let mut authority: Option<&[u8]> = None;
    let mut protocol: Option<&[u8]> = None;
    let mut host: Option<&[u8]> = None;
    let mut pseudo_done = false;

    for header in headers {
        if header.name().starts_with(b":") {
            // 擬似ヘッダーが通常ヘッダーの後に出現 (RFC 9114 Section 4.3)
            if pseudo_done {
                return Err(Error::StreamError(ErrorCode::MessageError));
            }

            match header.name() {
                b":method" => {
                    if method.is_some() {
                        // 重複 (RFC 9114 Section 4.3.1)
                        return Err(Error::StreamError(ErrorCode::MessageError));
                    }
                    method = Some(header.value());
                }
                b":scheme" => {
                    if scheme.is_some() {
                        return Err(Error::StreamError(ErrorCode::MessageError));
                    }
                    scheme = Some(header.value());
                }
                b":path" => {
                    if path.is_some() {
                        return Err(Error::StreamError(ErrorCode::MessageError));
                    }
                    path = Some(header.value());
                }
                b":authority" => {
                    if authority.is_some() {
                        return Err(Error::StreamError(ErrorCode::MessageError));
                    }
                    authority = Some(header.value());
                }
                b":protocol" => {
                    // Extended CONNECT (RFC 8441, RFC 9220)
                    if protocol.is_some() {
                        return Err(Error::StreamError(ErrorCode::MessageError));
                    }
                    protocol = Some(header.value());
                }
                b":status" => {
                    // レスポンス専用の擬似ヘッダー
                    return Err(Error::StreamError(ErrorCode::MessageError));
                }
                _ => {
                    // 未定義の擬似ヘッダー (RFC 9114 Section 4.3)
                    return Err(Error::StreamError(ErrorCode::MessageError));
                }
            }
        } else {
            pseudo_done = true;

            // フィールド名の検証 (RFC 9110 Section 5.1, RFC 9114 Section 4.2, 10.3)
            if !is_valid_field_name(header.name()) {
                return Err(Error::StreamError(ErrorCode::MessageError));
            }

            // 接続固有フィールドの検出 (RFC 9114 Section 4.2)
            if CONNECTION_SPECIFIC_FIELDS.contains(&header.name()) {
                return Err(Error::StreamError(ErrorCode::MessageError));
            }

            // TE ヘッダーは "trailers" のみ許可 (RFC 9114 Section 4.2)
            if header.name() == b"te" && header.value() != b"trailers" {
                return Err(Error::StreamError(ErrorCode::MessageError));
            }

            // フィールド値の検証 (RFC 9110 Section 5.5, RFC 9114 Section 10.3)
            if !is_valid_field_value(header.value()) {
                return Err(Error::StreamError(ErrorCode::MessageError));
            }

            if header.name() == b"host" {
                host = Some(header.value());
            }
        }
    }

    // :method は必須かつ有効な token でなければならない (RFC 9114 Section 4.3.1, RFC 9110 Section 9.1)
    let method = method.ok_or(Error::StreamError(ErrorCode::MessageError))?;
    if !is_valid_method(method) {
        return Err(Error::StreamError(ErrorCode::MessageError));
    }

    if method == b"CONNECT" && protocol.is_some() {
        // Extended CONNECT (RFC 8441, RFC 9220)
        // :protocol が存在する場合は :scheme, :path が必須
        if scheme.is_none() || path.is_none() {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }
        // :scheme は有効な URI scheme でなければならない (RFC 3986 Section 3.1)
        if let Some(s) = scheme
            && !is_valid_scheme(s)
        {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }
        // :path は空であってはならない (RFC 9114 Section 4.3.1)
        if let Some(p) = path
            && p.is_empty()
        {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }
        // mandatory authority component を持つ scheme (http, https) では :authority が必須
        // (RFC 9114 Section 4.3.1, RFC 8441 Section 4)
        let ext_scheme_requires_authority =
            matches!(scheme, Some(s) if s == b"http" || s == b"https");
        if ext_scheme_requires_authority && authority.is_none() && host.is_none() {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }
        // 非 http/https スキームでは :authority / Host の有無を制限しない。
        // RFC 9114 Section 4.3.1 の MUST NOT は「scheme が mandatory authority を
        // 持たず、かつリクエストターゲットに authority がない」場合のみ適用される。
        // Sans I/O ライブラリとして任意のスキームの authority 要件を判断できないため、
        // スキーム固有の検証は呼び出し側の責務とする。
        // :authority と host の整合チェック (RFC 9114 Section 4.3.1)
        if let (Some(a), Some(h)) = (authority, host)
            && a != h
        {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }
        // :authority / host が存在する場合は空であってはならない (RFC 9114 Section 4.3.1)
        if let Some(a) = authority
            && a.is_empty()
        {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }
        if let Some(h) = host
            && h.is_empty()
        {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }
    } else if method == b"CONNECT" {
        // 通常の CONNECT リクエスト (RFC 9114 Section 4.4)
        // :scheme と :path は存在してはならない
        if scheme.is_some() || path.is_some() {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }
        // :protocol は通常 CONNECT では不正
        // (ここには到達しない: protocol.is_some() は上の分岐で処理済み)

        // :authority は必須かつ authority-form (host:port) でなければならない
        // (RFC 9114 Section 4.4, RFC 9110 Section 7.1)
        match authority {
            None => return Err(Error::StreamError(ErrorCode::MessageError)),
            Some([]) => return Err(Error::StreamError(ErrorCode::MessageError)),
            Some(val) => {
                if !is_valid_connect_authority(val) {
                    return Err(Error::StreamError(ErrorCode::MessageError));
                }
            }
        }
    } else {
        // :protocol は非 CONNECT では不正
        if protocol.is_some() {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }
        // 非 CONNECT リクエスト (RFC 9114 Section 4.3.1)
        // :method, :scheme, :path は必須
        if scheme.is_none() || path.is_none() {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }

        // :scheme は有効な URI scheme でなければならない (RFC 3986 Section 3.1)
        if let Some(s) = scheme
            && !is_valid_scheme(s)
        {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }

        // :path の検証 (RFC 9114 Section 4.3.1)
        // http/https では path-absolute ("/" 始まり) または "*" (OPTIONS) でなければならない
        let is_http_or_https = matches!(scheme, Some(s) if s == b"http" || s == b"https");
        if let Some(p) = path {
            if p.is_empty() {
                return Err(Error::StreamError(ErrorCode::MessageError));
            }
            if is_http_or_https && !is_valid_http_path(p, method) {
                return Err(Error::StreamError(ErrorCode::MessageError));
            }
        }

        // mandatory authority component を持つ scheme (http, https) では
        // :authority または Host のいずれかが必須 (RFC 9114 Section 4.3.1)
        if is_http_or_https && authority.is_none() && host.is_none() {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }

        // 非 http/https スキームでは :authority / Host の有無を制限しない。
        // RFC 9114 Section 4.3.1 の MUST NOT は「scheme が mandatory authority を
        // 持たず、かつリクエストターゲットに authority がない」場合のみ適用される。
        // Sans I/O ライブラリとして任意のスキームの authority 要件を判断できないため、
        // スキーム固有の検証は呼び出し側の責務とする。
    }

    // :authority と host の整合 (RFC 9114 Section 4.3.1)
    if let (Some(a), Some(h)) = (authority, host)
        && a != h
    {
        return Err(Error::StreamError(ErrorCode::MessageError));
    }

    // :authority / host が存在する場合は空であってはならない (RFC 9114 Section 4.3.1)
    if let Some(a) = authority
        && a.is_empty()
        && method != b"CONNECT"
    {
        return Err(Error::StreamError(ErrorCode::MessageError));
    }
    if let Some(h) = host
        && h.is_empty()
    {
        return Err(Error::StreamError(ErrorCode::MessageError));
    }

    // http / https スキームでは :authority に userinfo を含めてはならない (RFC 9114 Section 4.3.1)
    // 通常の CONNECT は :scheme を持たないため、このチェックは Extended CONNECT と非 CONNECT に適用される
    let is_http_scheme = matches!(scheme, Some(s) if s == b"http" || s == b"https");
    if is_http_scheme
        && let Some(a) = authority
        && a.contains(&b'@')
    {
        return Err(Error::StreamError(ErrorCode::MessageError));
    }

    // :authority が URI authority として有効な形式かを検証する
    // (RFC 9114 Section 4.3.1, RFC 3986 Section 3.2)
    // plain CONNECT は is_valid_connect_authority で別途検証済みなのでここでは除外する
    if (method != b"CONNECT" || protocol.is_some())
        && let Some(a) = authority
        && !is_valid_authority(a)
    {
        return Err(Error::StreamError(ErrorCode::MessageError));
    }

    Ok(())
}

/// レスポンスヘッダーを検証 (RFC 9114 Section 4.1.2, 4.3.2)
pub fn validate_response_headers<H: HeaderField>(headers: &[H]) -> Result<(), Error> {
    let mut status: Option<&[u8]> = None;
    let mut pseudo_done = false;

    for header in headers {
        if header.name().starts_with(b":") {
            // 擬似ヘッダーが通常ヘッダーの後に出現
            if pseudo_done {
                return Err(Error::StreamError(ErrorCode::MessageError));
            }

            match header.name() {
                b":status" => {
                    if status.is_some() {
                        return Err(Error::StreamError(ErrorCode::MessageError));
                    }
                    // :status は 3 桁の ASCII 数字でなければならない (RFC 9110 status-code = 3DIGIT)
                    if header.value().len() != 3
                        || !header.value().iter().all(|b| b.is_ascii_digit())
                    {
                        return Err(Error::StreamError(ErrorCode::MessageError));
                    }
                    // HTTP/3 は 101 (Switching Protocols) をサポートしない (RFC 9114 Section 4.5)
                    if header.value() == b"101" {
                        return Err(Error::StreamError(ErrorCode::MessageError));
                    }
                    status = Some(header.value());
                }
                _ => {
                    // レスポンスではリクエスト擬似ヘッダーも未定義擬似ヘッダーも malformed
                    return Err(Error::StreamError(ErrorCode::MessageError));
                }
            }
        } else {
            pseudo_done = true;

            // フィールド名の検証 (RFC 9110 Section 5.1, RFC 9114 Section 4.2, 10.3)
            if !is_valid_field_name(header.name()) {
                return Err(Error::StreamError(ErrorCode::MessageError));
            }

            // 接続固有フィールドの検出
            if CONNECTION_SPECIFIC_FIELDS.contains(&header.name()) {
                return Err(Error::StreamError(ErrorCode::MessageError));
            }

            // TE ヘッダーはリクエストのみ許可。レスポンスには存在してはならない (RFC 9114 Section 4.2)
            if header.name() == b"te" {
                return Err(Error::StreamError(ErrorCode::MessageError));
            }

            // フィールド値の検証 (RFC 9110 Section 5.5, RFC 9114 Section 10.3)
            if !is_valid_field_value(header.value()) {
                return Err(Error::StreamError(ErrorCode::MessageError));
            }
        }
    }

    // :status は必須 (RFC 9114 Section 4.3.2)
    status.ok_or(Error::StreamError(ErrorCode::MessageError))?;

    Ok(())
}

/// ヘッダーを role に応じて検証
pub fn validate_headers<H: HeaderField>(headers: &[H], role: Role) -> Result<(), Error> {
    match role {
        // サーバーが受信するのはリクエスト
        Role::Server => validate_request_headers(headers),
        // クライアントが受信するのはレスポンス
        Role::Client => validate_response_headers(headers),
    }
}

/// content-length と受信済み DATA フレームの整合性を検証する (RFC 9114 Section 4.1.2)
///
/// - content-length ヘッダーが存在しない場合: 検証不要 (Ok)
/// - content-length ヘッダーが複数ある場合: malformed
/// - content-length 値が非負整数でない場合: malformed
/// - skip_body_check == false かつ値 != received_body_size の場合: malformed
///
/// HEAD レスポンスや 1xx/204/304 レスポンスは skip_body_check = true を渡すこと。
pub fn validate_content_length<H: HeaderField>(
    headers: &[H],
    received_body_size: u64,
    skip_body_check: bool,
) -> Result<(), Error> {
    let mut content_length: Option<u64> = None;

    for header in headers {
        if header.name() != b"content-length" {
            continue;
        }

        // 複数の content-length は malformed (RFC 9114 Section 4.1.2)
        if content_length.is_some() {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }

        // 値が非負整数でなければ malformed
        let value_str = std::str::from_utf8(header.value())
            .map_err(|_| Error::StreamError(ErrorCode::MessageError))?;
        let value = value_str
            .parse::<u64>()
            .map_err(|_| Error::StreamError(ErrorCode::MessageError))?;
        content_length = Some(value);
    }

    let Some(expected) = content_length else {
        // content-length ヘッダーなし: 検証不要
        return Ok(());
    };

    if skip_body_check {
        // HEAD レスポンス・1xx/204/304 レスポンス: DATA なしでも正当
        return Ok(());
    }

    // content-length と受信済み DATA の合計バイト数が一致しなければ malformed
    if expected != received_body_size {
        return Err(Error::StreamError(ErrorCode::MessageError));
    }

    Ok(())
}

/// トレーラーヘッダーを検証 (RFC 9114 Section 4.1.2, 4.3)
///
/// トレーラーセクションには疑似ヘッダーを含めてはならない (RFC 9114 Section 4.3)。
/// ロールに関わらず同じルールが適用される。
pub fn validate_trailer_headers<H: HeaderField>(headers: &[H]) -> Result<(), Error> {
    for header in headers {
        // トレーラーに疑似ヘッダーは禁止 (RFC 9114 Section 4.3)
        if header.name().starts_with(b":") {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }

        // フィールド名の検証 (RFC 9110 Section 5.1, RFC 9114 Section 4.2, 10.3)
        if !is_valid_field_name(header.name()) {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }

        // 接続固有フィールドの検出 (RFC 9114 Section 4.2)
        if CONNECTION_SPECIFIC_FIELDS.contains(&header.name()) {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }

        // TE ヘッダーはリクエストのみ許可。トレーラーには存在してはならない (RFC 9114 Section 4.2)
        if header.name() == b"te" {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }

        // フィールド値の検証 (RFC 9110 Section 5.5, RFC 9114 Section 10.3)
        if !is_valid_field_value(header.value()) {
            return Err(Error::StreamError(ErrorCode::MessageError));
        }
    }

    Ok(())
}

/// フィールドセクションサイズを計算する (RFC 9114 Section 4.2.2)
///
/// サイズは各フィールドの名前長 + 値長 + 32 バイトのオーバーヘッドの合計。
pub fn calculate_field_section_size<H: HeaderField>(headers: &[H]) -> u64 {
    headers
        .iter()
        .map(|h| h.name().len() as u64 + h.value().len() as u64 + 32)
        .sum()
}

/// peer の SETTINGS_MAX_FIELD_SECTION_SIZE を超えていないかチェックする (RFC 9114 Section 4.2.2)
///
/// peer がこの設定を送信していない場合 (None) はチェックしない。
pub fn check_field_section_size<H: HeaderField>(
    headers: &[H],
    peer_max: Option<u64>,
) -> Result<(), Error> {
    if let Some(max_size) = peer_max {
        let size = calculate_field_section_size(headers);
        if size > max_size {
            return Err(Error::ConnectionError(ErrorCode::InternalError));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(name: &[u8], value: &[u8]) -> DecodedHeader {
        DecodedHeader {
            name: name.to_vec(),
            value: value.to_vec(),
        }
    }

    // =========================================================================
    // リクエスト検証
    // =========================================================================

    #[test]
    fn test_valid_get_request() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
        ];
        assert!(validate_request_headers(&headers).is_ok());
    }

    #[test]
    fn test_valid_connect_request() {
        let headers = vec![
            h(b":method", b"CONNECT"),
            h(b":authority", b"example.com:443"),
        ];
        assert!(validate_request_headers(&headers).is_ok());
    }

    #[test]
    fn test_valid_extended_connect_request() {
        // WebTransport 等の Extended CONNECT (RFC 8441, RFC 9220)
        let headers = vec![
            h(b":method", b"CONNECT"),
            h(b":protocol", b"webtransport-h3"),
            h(b":scheme", b"https"),
            h(b":path", b"/webtransport"),
            h(b":authority", b"example.com"),
        ];
        assert!(validate_request_headers(&headers).is_ok());
    }

    #[test]
    fn test_extended_connect_missing_scheme_is_malformed() {
        let headers = vec![
            h(b":method", b"CONNECT"),
            h(b":protocol", b"webtransport-h3"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_extended_connect_missing_path_is_malformed() {
        let headers = vec![
            h(b":method", b"CONNECT"),
            h(b":protocol", b"webtransport-h3"),
            h(b":scheme", b"https"),
            h(b":authority", b"example.com"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_protocol_on_non_connect_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":protocol", b"webtransport-h3"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_connect_with_scheme_is_malformed() {
        let headers = vec![
            h(b":method", b"CONNECT"),
            h(b":scheme", b"https"),
            h(b":authority", b"example.com:443"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_connect_with_path_is_malformed() {
        let headers = vec![
            h(b":method", b"CONNECT"),
            h(b":path", b"/"),
            h(b":authority", b"example.com:443"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_connect_without_authority_is_malformed() {
        let headers = vec![h(b":method", b"CONNECT")];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_missing_method_is_malformed() {
        let headers = vec![h(b":scheme", b"https"), h(b":path", b"/")];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_missing_scheme_is_malformed() {
        let headers = vec![h(b":method", b"GET"), h(b":path", b"/")];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_missing_path_is_malformed() {
        let headers = vec![h(b":method", b"GET"), h(b":scheme", b"https")];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_empty_path_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b""),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_pseudo_after_regular_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b"accept", b"*/*"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_uppercase_field_name_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b"Content-Type", b"text/html"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_connection_field_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b"connection", b"keep-alive"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_te_trailers_is_allowed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
            h(b"te", b"trailers"),
        ];
        assert!(validate_request_headers(&headers).is_ok());
    }

    #[test]
    fn test_te_non_trailers_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
            h(b"te", b"gzip"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_https_without_authority_or_host_is_malformed() {
        // https scheme では :authority または Host が必須 (RFC 9114 Section 4.3.1)
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_http_without_authority_or_host_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"http"),
            h(b":path", b"/"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_https_with_host_only_is_valid() {
        // :authority がなくても Host があれば有効
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b"host", b"example.com"),
        ];
        assert!(validate_request_headers(&headers).is_ok());
    }

    #[test]
    fn test_authority_host_mismatch_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
            h(b"host", b"other.com"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_duplicate_method_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":method", b"POST"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_status_in_request_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":status", b"200"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_unknown_pseudo_header_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":unknown", b"value"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    // =========================================================================
    // レスポンス検証
    // =========================================================================

    #[test]
    fn test_valid_response() {
        let headers = vec![h(b":status", b"200"), h(b"content-type", b"text/html")];
        assert!(validate_response_headers(&headers).is_ok());
    }

    #[test]
    fn test_status_101_is_rejected() {
        // HTTP/3 は 101 (Switching Protocols) をサポートしない (RFC 9114 Section 4.5)
        let headers = vec![h(b":status", b"101")];
        assert!(validate_response_headers(&headers).is_err());
    }

    #[test]
    fn test_status_100_is_valid() {
        // 100 (Continue) は有効な中間レスポンス
        let headers = vec![h(b":status", b"100")];
        assert!(validate_response_headers(&headers).is_ok());
    }

    #[test]
    fn test_status_non_digit_is_malformed() {
        // :status の値が非数字は malformed (RFC 9114 Section 4.1.2, 4.3.2)
        let headers = vec![h(b":status", b"abc")];
        assert!(validate_response_headers(&headers).is_err());
    }

    #[test]
    fn test_status_two_digits_is_malformed() {
        let headers = vec![h(b":status", b"20")];
        assert!(validate_response_headers(&headers).is_err());
    }

    #[test]
    fn test_status_four_digits_is_malformed() {
        let headers = vec![h(b":status", b"2000")];
        assert!(validate_response_headers(&headers).is_err());
    }

    #[test]
    fn test_missing_status_is_malformed() {
        let headers = vec![h(b"content-type", b"text/html")];
        assert!(validate_response_headers(&headers).is_err());
    }

    #[test]
    fn test_method_in_response_is_malformed() {
        let headers = vec![h(b":status", b"200"), h(b":method", b"GET")];
        assert!(validate_response_headers(&headers).is_err());
    }

    #[test]
    fn test_response_uppercase_field_name_is_malformed() {
        let headers = vec![h(b":status", b"200"), h(b"Content-Type", b"text/html")];
        assert!(validate_response_headers(&headers).is_err());
    }

    #[test]
    fn test_response_connection_field_is_malformed() {
        let headers = vec![h(b":status", b"200"), h(b"transfer-encoding", b"chunked")];
        assert!(validate_response_headers(&headers).is_err());
    }

    // =========================================================================
    // トレーラー検証
    // =========================================================================

    #[test]
    fn test_valid_trailer() {
        // 疑似ヘッダーなし・通常フィールドのみのトレーラーは正当
        let headers = vec![h(b"x-checksum", b"abc123")];
        assert!(validate_trailer_headers(&headers).is_ok());
    }

    #[test]
    fn test_empty_trailer_is_valid() {
        // フィールドなしのトレーラーも正当
        let headers: Vec<DecodedHeader> = vec![];
        assert!(validate_trailer_headers(&headers).is_ok());
    }

    #[test]
    fn test_trailer_with_status_is_malformed() {
        // トレーラーに :status は禁止 (RFC 9114 Section 4.3)
        let headers = vec![h(b":status", b"200")];
        assert!(validate_trailer_headers(&headers).is_err());
    }

    #[test]
    fn test_trailer_with_method_is_malformed() {
        // トレーラーにリクエスト疑似ヘッダーも禁止 (RFC 9114 Section 4.3)
        let headers = vec![h(b":method", b"GET")];
        assert!(validate_trailer_headers(&headers).is_err());
    }

    #[test]
    fn test_trailer_uppercase_field_name_is_malformed() {
        let headers = vec![h(b"X-Checksum", b"abc123")];
        assert!(validate_trailer_headers(&headers).is_err());
    }

    #[test]
    fn test_trailer_connection_field_is_malformed() {
        let headers = vec![h(b"transfer-encoding", b"chunked")];
        assert!(validate_trailer_headers(&headers).is_err());
    }

    // =========================================================================
    // 0024: TE ヘッダーのレスポンス/トレーラー拒否 (RFC 9114 Section 4.2)
    // =========================================================================

    #[test]
    fn test_te_in_response_is_malformed() {
        // TE はリクエストのみ許可 (RFC 9114 Section 4.2)
        let headers = vec![h(b":status", b"200"), h(b"te", b"trailers")];
        assert!(validate_response_headers(&headers).is_err());
    }

    #[test]
    fn test_te_in_trailer_is_malformed() {
        // TE はリクエストのみ許可 (RFC 9114 Section 4.2)
        let headers = vec![h(b"te", b"trailers")];
        assert!(validate_trailer_headers(&headers).is_err());
    }

    // =========================================================================
    // 0025: field-value の NUL / CR / LF 拒否 (RFC 9114 Section 10.3)
    // =========================================================================

    #[test]
    fn test_request_field_value_with_nul_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
            h(b"x-header", b"val\x00ue"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_request_field_value_with_cr_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
            h(b"x-header", b"val\x0due"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_request_field_value_with_lf_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
            h(b"x-header", b"val\x0aue"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_response_field_value_with_nul_is_malformed() {
        let headers = vec![h(b":status", b"200"), h(b"x-header", b"val\x00ue")];
        assert!(validate_response_headers(&headers).is_err());
    }

    #[test]
    fn test_response_field_value_with_cr_is_malformed() {
        let headers = vec![h(b":status", b"200"), h(b"x-header", b"val\x0due")];
        assert!(validate_response_headers(&headers).is_err());
    }

    #[test]
    fn test_response_field_value_with_lf_is_malformed() {
        let headers = vec![h(b":status", b"200"), h(b"x-header", b"val\x0aue")];
        assert!(validate_response_headers(&headers).is_err());
    }

    #[test]
    fn test_trailer_field_value_with_nul_is_malformed() {
        let headers = vec![h(b"x-checksum", b"abc\x00def")];
        assert!(validate_trailer_headers(&headers).is_err());
    }

    #[test]
    fn test_trailer_field_value_with_cr_is_malformed() {
        let headers = vec![h(b"x-checksum", b"abc\x0ddef")];
        assert!(validate_trailer_headers(&headers).is_err());
    }

    #[test]
    fn test_trailer_field_value_with_lf_is_malformed() {
        let headers = vec![h(b"x-checksum", b"abc\x0adef")];
        assert!(validate_trailer_headers(&headers).is_err());
    }

    // =========================================================================
    // 0027: content-length と DATA フレームの整合性検証 (RFC 9114 Section 4.1.2)
    // =========================================================================

    #[test]
    fn test_content_length_absent_is_ok() {
        // content-length なし: 検証不要
        let headers = vec![h(b":status", b"200")];
        assert!(validate_content_length(&headers, 0, false).is_ok());
        assert!(validate_content_length(&headers, 100, false).is_ok());
    }

    #[test]
    fn test_content_length_zero_with_empty_body_is_ok() {
        // content-length: 0 かつ body 空は正当
        let headers = vec![h(b"content-length", b"0")];
        assert!(validate_content_length(&headers, 0, false).is_ok());
    }

    #[test]
    fn test_content_length_matches_body_size_is_ok() {
        // content-length の値と body サイズが一致
        let headers = vec![h(b"content-length", b"10")];
        assert!(validate_content_length(&headers, 10, false).is_ok());
    }

    #[test]
    fn test_duplicate_content_length_is_malformed() {
        // content-length が 2 個: malformed
        let headers = vec![h(b"content-length", b"10"), h(b"content-length", b"10")];
        assert!(validate_content_length(&headers, 10, false).is_err());
    }

    #[test]
    fn test_content_length_too_large_is_malformed() {
        // content-length の値が body サイズより大きい
        let headers = vec![h(b"content-length", b"10")];
        assert!(validate_content_length(&headers, 5, false).is_err());
    }

    #[test]
    fn test_content_length_too_small_is_malformed() {
        // content-length の値が body サイズより小さい
        let headers = vec![h(b"content-length", b"5")];
        assert!(validate_content_length(&headers, 10, false).is_err());
    }

    #[test]
    fn test_content_length_non_numeric_is_malformed() {
        // content-length が非数値: malformed
        let headers = vec![h(b"content-length", b"abc")];
        assert!(validate_content_length(&headers, 0, false).is_err());
    }

    #[test]
    fn test_content_length_skip_body_check_is_ok() {
        // skip_body_check = true の場合は body サイズ不一致でも Ok (HEAD/1xx/204/304)
        let headers = vec![h(b"content-length", b"100")];
        assert!(validate_content_length(&headers, 0, true).is_ok());
    }

    // =========================================================================
    // 0026: Extended CONNECT の :authority 検証 (RFC 9114 Section 4.3.1, RFC 8441)
    // =========================================================================

    #[test]
    fn test_extended_connect_https_without_authority_is_malformed() {
        // https scheme では :authority が必須 (RFC 9114 Section 4.3.1, RFC 8441 Section 4)
        let headers = vec![
            h(b":method", b"CONNECT"),
            h(b":protocol", b"webtransport-h3"),
            h(b":scheme", b"https"),
            h(b":path", b"/webtransport"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_extended_connect_https_with_empty_authority_is_malformed() {
        let headers = vec![
            h(b":method", b"CONNECT"),
            h(b":protocol", b"webtransport-h3"),
            h(b":scheme", b"https"),
            h(b":path", b"/webtransport"),
            h(b":authority", b""),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_extended_connect_https_with_host_is_valid() {
        // Host ヘッダーで代替可能 (RFC 9114 Section 4.3.1)
        let headers = vec![
            h(b":method", b"CONNECT"),
            h(b":protocol", b"webtransport-h3"),
            h(b":scheme", b"https"),
            h(b":path", b"/webtransport"),
            h(b"host", b"example.com"),
        ];
        assert!(validate_request_headers(&headers).is_ok());
    }

    #[test]
    fn test_extended_connect_authority_host_mismatch_is_malformed() {
        let headers = vec![
            h(b":method", b"CONNECT"),
            h(b":protocol", b"webtransport-h3"),
            h(b":scheme", b"https"),
            h(b":path", b"/webtransport"),
            h(b":authority", b"example.com"),
            h(b"host", b"other.com"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    // =========================================================================
    // 0045: フィールド名の不正文字検証 (RFC 9110 Section 5.1, RFC 9114 Section 10.3)
    // =========================================================================

    #[test]
    fn test_field_name_with_space_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
            h(b"invalid name", b"value"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_field_name_with_control_char_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
            h(b"x-hdr\x01", b"value"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_field_name_with_slash_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
            h(b"x/hdr", b"value"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_field_name_with_colon_is_malformed() {
        // 通常ヘッダーでコロンを含む名前は不正 (token に含まれない)
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
            h(b"x:hdr", b"value"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_field_name_with_at_sign_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
            h(b"x@hdr", b"value"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_field_name_with_del_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
            h(b"x-hdr\x7f", b"value"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_valid_field_name_with_tchar() {
        // tchar に含まれる記号を含むフィールド名は正当
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
            h(b"x-hdr_name.v1+2", b"value"),
        ];
        assert!(validate_request_headers(&headers).is_ok());
    }

    #[test]
    fn test_response_field_name_with_space_is_malformed() {
        let headers = vec![h(b":status", b"200"), h(b"invalid name", b"value")];
        assert!(validate_response_headers(&headers).is_err());
    }

    #[test]
    fn test_trailer_field_name_with_space_is_malformed() {
        let headers = vec![h(b"invalid name", b"value")];
        assert!(validate_trailer_headers(&headers).is_err());
    }

    // =========================================================================
    // 0046: フィールド値の不正文字検証 (RFC 9110 Section 5.5, RFC 9114 Section 10.3)
    // =========================================================================

    #[test]
    fn test_field_value_with_del_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
            h(b"x-hdr", b"val\x7fue"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_field_value_with_control_0x01_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
            h(b"x-hdr", b"val\x01ue"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_field_value_with_control_0x1f_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
            h(b"x-hdr", b"val\x1fue"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_field_value_with_leading_space_is_malformed() {
        // field-content ABNF: 先頭は field-vchar でなければならない
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
            h(b"x-hdr", b" value"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_field_value_with_trailing_space_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
            h(b"x-hdr", b"value "),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_field_value_with_leading_htab_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
            h(b"x-hdr", b"\tvalue"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_field_value_with_middle_space_is_valid() {
        // 途中の SP は field-content で許可
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
            h(b"x-hdr", b"val ue"),
        ];
        assert!(validate_request_headers(&headers).is_ok());
    }

    #[test]
    fn test_field_value_with_middle_htab_is_valid() {
        // 途中の HTAB は field-content で許可
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
            h(b"x-hdr", b"val\tue"),
        ];
        assert!(validate_request_headers(&headers).is_ok());
    }

    #[test]
    fn test_field_value_with_obs_text_is_valid() {
        // obs-text (0x80-0xFF) は許可
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
            h(b"x-hdr", b"\x80\xff"),
        ];
        assert!(validate_request_headers(&headers).is_ok());
    }

    #[test]
    fn test_empty_field_value_is_valid() {
        // 空のフィールド値は field-value = *field-content で許可
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
            h(b"x-hdr", b""),
        ];
        assert!(validate_request_headers(&headers).is_ok());
    }

    #[test]
    fn test_response_field_value_with_del_is_malformed() {
        let headers = vec![h(b":status", b"200"), h(b"x-hdr", b"val\x7fue")];
        assert!(validate_response_headers(&headers).is_err());
    }

    #[test]
    fn test_trailer_field_value_with_del_is_malformed() {
        let headers = vec![h(b"x-checksum", b"abc\x7fdef")];
        assert!(validate_trailer_headers(&headers).is_err());
    }

    // =========================================================================
    // 0051: :authority の userinfo 拒否 (RFC 9114 Section 4.3.1)
    // =========================================================================

    #[test]
    fn test_authority_with_userinfo_is_malformed() {
        // http/https scheme で :authority に userinfo を含むのは不正
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"user:pass@example.com"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_authority_with_userinfo_http_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"http"),
            h(b":path", b"/"),
            h(b":authority", b"user@example.com"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_authority_without_userinfo_is_valid() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com:443"),
        ];
        assert!(validate_request_headers(&headers).is_ok());
    }

    #[test]
    fn test_connect_authority_with_at_sign_is_malformed() {
        // authority-form は uri-host ":" port であり userinfo を含まない
        // (RFC 9110 Section 7.1) ため '@' は不正
        let headers = vec![
            h(b":method", b"CONNECT"),
            h(b":authority", b"user@example.com:443"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_extended_connect_authority_with_userinfo_is_malformed() {
        // Extended CONNECT で https scheme の場合は userinfo チェック適用
        let headers = vec![
            h(b":method", b"CONNECT"),
            h(b":protocol", b"webtransport-h3"),
            h(b":scheme", b"https"),
            h(b":path", b"/webtransport"),
            h(b":authority", b"user@example.com"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_non_http_scheme_with_authority_is_valid() {
        // 非 http/https スキームでも :authority を許可する
        // RFC 9114 Section 4.3.1 の MUST NOT は「scheme が mandatory authority を
        // 持たず、かつリクエストターゲットに authority がない」場合のみ適用される。
        // ライブラリはスキーム固有の authority 要件を判断しない。
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"ftp"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
        ];
        assert!(validate_request_headers(&headers).is_ok());
    }

    #[test]
    fn test_non_http_scheme_with_host_is_valid() {
        // 非 http/https スキームでも Host を許可する
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"ftp"),
            h(b":path", b"/"),
            h(b"host", b"example.com"),
        ];
        assert!(validate_request_headers(&headers).is_ok());
    }

    #[test]
    fn test_non_http_scheme_without_authority_is_valid() {
        // 非 http/https スキームで :authority も Host もない場合も正当
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"ftp"),
            h(b":path", b"/files"),
        ];
        assert!(validate_request_headers(&headers).is_ok());
    }

    // P1: :method の token 検証
    #[test]
    fn test_method_with_space_is_malformed() {
        // method は token (RFC 9110 Section 9.1) なので空白を含めない
        let headers = vec![
            h(b":method", b"GE T"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_method_with_control_char_is_malformed() {
        let headers = vec![
            h(b":method", b"GET\x01"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_method_with_slash_is_malformed() {
        // '/' は tchar ではない
        let headers = vec![
            h(b":method", b"GE/T"),
            h(b":scheme", b"https"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    // P1: :scheme の文法検証
    #[test]
    fn test_scheme_starting_with_digit_is_malformed() {
        // scheme は ALPHA で始まる (RFC 3986 Section 3.1)
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"1bad"),
            h(b":path", b"/"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_scheme_with_space_is_malformed() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"ht tps"),
            h(b":path", b"/"),
            h(b":authority", b"example.com"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_scheme_with_valid_special_chars() {
        // scheme = ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )
        // "coap+tcp" は妥当
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"coap+tcp"),
            h(b":path", b"/resource"),
        ];
        assert!(validate_request_headers(&headers).is_ok());
    }

    // P1: :path の検証
    #[test]
    fn test_path_not_starting_with_slash_is_malformed() {
        // http/https では path-absolute ("/" 始まり) が必須
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"abc"),
            h(b":authority", b"example.com"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_options_asterisk_path_is_valid() {
        // OPTIONS の場合は "*" が許可される
        let headers = vec![
            h(b":method", b"OPTIONS"),
            h(b":scheme", b"https"),
            h(b":path", b"*"),
            h(b":authority", b"example.com"),
        ];
        assert!(validate_request_headers(&headers).is_ok());
    }

    #[test]
    fn test_non_options_asterisk_path_is_malformed() {
        // OPTIONS 以外で "*" は不正
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"*"),
            h(b":authority", b"example.com"),
        ];
        assert!(validate_request_headers(&headers).is_err());
    }

    #[test]
    fn test_path_with_query_is_valid() {
        let headers = vec![
            h(b":method", b"GET"),
            h(b":scheme", b"https"),
            h(b":path", b"/search?q=hello"),
            h(b":authority", b"example.com"),
        ];
        assert!(validate_request_headers(&headers).is_ok());
    }

    // P2: Extended CONNECT でも authority 不要 scheme のチェック
    #[test]
    fn test_extended_connect_non_http_scheme_with_authority_is_valid() {
        // 非 http/https スキームでも :authority を許可する
        let headers = vec![
            h(b":method", b"CONNECT"),
            h(b":protocol", b"websocket"),
            h(b":scheme", b"ftp"),
            h(b":path", b"/ws"),
            h(b":authority", b"example.com"),
        ];
        assert!(validate_request_headers(&headers).is_ok());
    }

    #[test]
    fn test_extended_connect_non_http_scheme_without_authority_is_valid() {
        let headers = vec![
            h(b":method", b"CONNECT"),
            h(b":protocol", b"websocket"),
            h(b":scheme", b"ftp"),
            h(b":path", b"/ws"),
        ];
        assert!(validate_request_headers(&headers).is_ok());
    }
}
