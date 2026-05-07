//! QPACK エンコーダー (RFC 9204)
//!
//! 静的テーブルと動的テーブルを使用する QPACK エンコーダー。
//!
//! ## 機能
//!
//! - `Encoder`: 静的テーブルのみを使用するシンプルなエンコーダー
//! - `DynamicEncoder`: 動的テーブルも使用する拡張エンコーダー

use std::collections::{HashMap, VecDeque};

use super::dynamic_table::DynamicTable;
use super::huffman;
use super::table::{STATIC_TABLE, find_static_entry};

/// ヘッダーフィールド
///
/// name/value は `Bytes` で保持する (issue 0059 Phase 3)。
/// 動的テーブルへの insert は cheap clone (refcount のみ) で済む。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    /// ヘッダー名
    pub name: bytes::Bytes,
    /// ヘッダー値
    pub value: bytes::Bytes,
}

impl Header {
    /// 新しいヘッダーを作成 (内部で `copy_from_slice` で `Bytes` 化する)
    ///
    /// バイトリテラル (`b":method"` 等) や `&[u8]` を素直に受け入れる API。
    /// すでに `Bytes` を持っているなら [`Self::from_bytes`] を使うと
    /// refcount だけで構築でき、コピーを避けられる。
    pub fn new(name: impl AsRef<[u8]>, value: impl AsRef<[u8]>) -> Self {
        Self {
            name: bytes::Bytes::copy_from_slice(name.as_ref()),
            value: bytes::Bytes::copy_from_slice(value.as_ref()),
        }
    }

    /// 既存の `Bytes` から zero-copy でヘッダーを作成
    pub fn from_bytes(name: bytes::Bytes, value: bytes::Bytes) -> Self {
        Self { name, value }
    }
}

/// QPACK エンコーダー
#[derive(Debug, Default)]
pub struct Encoder {
    /// ハフマン符号化を使用するか
    use_huffman: bool,
}

impl Encoder {
    /// 新しいエンコーダーを作成
    pub fn new() -> Self {
        Self { use_huffman: true }
    }

    /// ハフマン符号化の使用を設定
    pub fn use_huffman(mut self, use_huffman: bool) -> Self {
        self.use_huffman = use_huffman;
        self
    }

    /// ヘッダーリストをエンコード
    ///
    /// 成功時はエンコードしたバイト数を返す
    pub fn encode(&self, buf: &mut [u8], headers: &[Header]) -> Option<usize> {
        let mut offset = 0;

        // Required Insert Count = 0 (静的テーブルのみ使用)
        if buf.is_empty() {
            return None;
        }
        buf[offset] = 0x00;
        offset += 1;

        // Delta Base = 0
        if offset >= buf.len() {
            return None;
        }
        buf[offset] = 0x00;
        offset += 1;

        // ヘッダーをエンコード
        for header in headers {
            let encoded = self.encode_header(&mut buf[offset..], header)?;
            offset += encoded;
        }

        Some(offset)
    }

    /// 単一のヘッダーをエンコード
    fn encode_header(&self, buf: &mut [u8], header: &Header) -> Option<usize> {
        let (exact_match, name_match) = find_static_entry(&header.name, &header.value);

        if let Some(index) = exact_match {
            // Indexed Field Line (静的テーブルの完全一致)
            self.encode_indexed_field(buf, index)
        } else if let Some(index) = name_match {
            // Literal Field Line with Name Reference (名前のみ一致)
            self.encode_literal_with_name_ref(buf, index, &header.value)
        } else {
            // Literal Field Line with Literal Name
            self.encode_literal_with_literal_name(buf, &header.name, &header.value)
        }
    }

    /// Indexed Field Line (静的テーブル) をエンコード
    ///
    /// Format: 1TNNNNNN
    /// T=1 for static table
    fn encode_indexed_field(&self, buf: &mut [u8], index: usize) -> Option<usize> {
        if index < 64 {
            // 6-bit prefix で収まる
            if buf.is_empty() {
                return None;
            }
            buf[0] = 0xc0 | (index as u8);
            Some(1)
        } else {
            // 6-bit prefix を超える場合
            self.encode_integer(buf, index as u64, 6, 0xc0)
        }
    }

    /// Literal Field Line with Name Reference (静的テーブル) をエンコード
    ///
    /// Format: 01NTNNNN (N=never index, T=static)
    fn encode_literal_with_name_ref(
        &self,
        buf: &mut [u8],
        index: usize,
        value: &[u8],
    ) -> Option<usize> {
        // Name Reference: 0101NNNN (T=1 for static, N=0)
        let mut offset = if index < 16 {
            if buf.is_empty() {
                return None;
            }
            buf[0] = 0x50 | (index as u8);
            1
        } else {
            self.encode_integer(buf, index as u64, 4, 0x50)?
        };

        // Value
        let value_len = self.encode_string(&mut buf[offset..], value)?;
        offset += value_len;

        Some(offset)
    }

    /// Literal Field Line with Literal Name をエンコード
    ///
    /// Format (RFC 9204 Section 4.5.4):
    /// ```text
    ///      0   1   2   3   4   5   6   7
    ///    +---+---+---+---+---+---+---+---+
    ///    | 0 | 0 | 1 | N |H|    Name     |
    ///    +---+---+---+---+---+-----------+
    ///    |  Name String (Length octets)  |
    ///    +-------------------------------+
    ///    |H|     Value Length (7+)       |
    ///    +---+---------------------------+
    ///    |  Value String (Length octets) |
    ///    +-------------------------------+
    /// ```
    fn encode_literal_with_literal_name(
        &self,
        buf: &mut [u8],
        name: &[u8],
        value: &[u8],
    ) -> Option<usize> {
        let mut offset = 0;

        // 名前のエンコード (3-bit prefix)
        // Prefix: 001N (N=0 for not "never indexed")
        // H ビット (bit 3) と Name Length (bits 0-2)
        let name_len = self.encode_string_with_prefix(buf, name, 3, 0x20)?;
        offset += name_len;

        // Value (string literal, 7-bit prefix)
        let value_len = self.encode_string(&mut buf[offset..], value)?;
        offset += value_len;

        Some(offset)
    }

    /// 文字列を指定された prefix bits でエンコード
    fn encode_string_with_prefix(
        &self,
        buf: &mut [u8],
        data: &[u8],
        prefix_bits: u8,
        prefix: u8,
    ) -> Option<usize> {
        if self.use_huffman {
            let huffman_len = huffman::encoded_len(data);
            if huffman_len < data.len() {
                // ハフマン符号化を使用
                // H ビットを設定: prefix に 0x08 を OR (3-bit prefix の場合)
                let h_bit = 1u8 << prefix_bits;
                let offset =
                    self.encode_integer(buf, huffman_len as u64, prefix_bits, prefix | h_bit)?;
                huffman::encode(&mut buf[offset..], data)?;
                return Some(offset + huffman_len);
            }
        }

        // リテラル文字列 (H=0)
        let offset = self.encode_integer(buf, data.len() as u64, prefix_bits, prefix)?;
        if buf.len() < offset + data.len() {
            return None;
        }
        buf[offset..offset + data.len()].copy_from_slice(data);
        Some(offset + data.len())
    }

    /// 文字列をエンコード (ハフマン対応)
    fn encode_string(&self, buf: &mut [u8], data: &[u8]) -> Option<usize> {
        if self.use_huffman {
            let huffman_len = huffman::encoded_len(data);
            if huffman_len < data.len() {
                // ハフマン符号化を使用
                let offset = self.encode_integer(buf, huffman_len as u64, 7, 0x80)?;
                huffman::encode(&mut buf[offset..], data)?;
                return Some(offset + huffman_len);
            }
        }

        // リテラル文字列
        let offset = self.encode_integer(buf, data.len() as u64, 7, 0x00)?;
        if buf.len() < offset + data.len() {
            return None;
        }
        buf[offset..offset + data.len()].copy_from_slice(data);
        Some(offset + data.len())
    }

    /// 整数をエンコード (RFC 7541 Section 5.1)
    fn encode_integer(
        &self,
        buf: &mut [u8],
        value: u64,
        prefix_bits: u8,
        prefix: u8,
    ) -> Option<usize> {
        let max_prefix = (1u64 << prefix_bits) - 1;

        if value < max_prefix {
            if buf.is_empty() {
                return None;
            }
            buf[0] = prefix | (value as u8);
            Some(1)
        } else {
            if buf.is_empty() {
                return None;
            }
            buf[0] = prefix | (max_prefix as u8);
            let mut offset = 1;
            let mut remaining = value - max_prefix;

            while remaining >= 128 {
                if offset >= buf.len() {
                    return None;
                }
                buf[offset] = 0x80 | ((remaining & 0x7f) as u8);
                remaining >>= 7;
                offset += 1;
            }

            if offset >= buf.len() {
                return None;
            }
            buf[offset] = remaining as u8;
            Some(offset + 1)
        }
    }
}

/// ヘッダーリストをエンコードするのに必要なバッファサイズを推定
pub fn estimate_encoded_size(headers: &[Header]) -> usize {
    // 2 bytes for Required Insert Count and Delta Base
    let mut size = 2;

    for header in headers {
        let (exact_match, name_match) = find_static_entry(&header.name, &header.value);

        if exact_match.is_some() {
            // Indexed Field Line: 1-2 bytes
            size += 2;
        } else if name_match.is_some() {
            // Literal with Name Reference
            size += 2 + 1 + header.value.len();
        } else {
            // Literal with Literal Name
            size += 2 + header.name.len() + 1 + header.value.len();
        }
    }

    size
}

/// 動的テーブル対応 QPACK エンコーダー (RFC 9204)
///
/// 静的テーブルと動的テーブルの両方を使用してヘッダーを圧縮する。
#[derive(Debug)]
pub struct DynamicEncoder {
    /// 動的テーブル
    table: DynamicTable,
    /// ハフマン符号化を使用するか
    use_huffman: bool,
    /// ピアの最大テーブル容量
    max_table_capacity: u64,
    /// Known Received Count (デコーダーが受信したと確認された挿入カウント)
    known_received_count: u64,
    /// 直前のエンコードで使用された Required Insert Count
    last_required_insert_count: u64,
    /// ピアの SETTINGS_QPACK_BLOCKED_STREAMS (RFC 9204 Section 2.1.2)
    peer_max_blocked_streams: u64,
    /// 未 ack フィールドセクションの Required Insert Count (stream_id → RIC の FIFO)
    /// (RFC 9204 Section 2.1.1, 4.4.1)
    unacked_section_rics: HashMap<u64, VecDeque<u64>>,
}

impl Default for DynamicEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl DynamicEncoder {
    /// 新しい動的エンコーダーを作成
    pub fn new() -> Self {
        Self {
            table: DynamicTable::new(),
            use_huffman: true,
            max_table_capacity: 0,
            known_received_count: 0,
            last_required_insert_count: 0,
            peer_max_blocked_streams: 0,
            unacked_section_rics: HashMap::new(),
        }
    }

    /// ハフマン符号化の使用を設定
    pub fn use_huffman(mut self, use_huffman: bool) -> Self {
        self.use_huffman = use_huffman;
        self
    }

    /// 最大テーブル容量を設定 (ピアの SETTINGS から)
    pub fn set_max_table_capacity(&mut self, capacity: u64) {
        self.max_table_capacity = capacity;
    }

    /// ピアの SETTINGS_QPACK_BLOCKED_STREAMS を設定 (RFC 9204 Section 2.1.2)
    ///
    /// エンコーダーはブロックし得るストリーム数をこの値以下に制限しなければならない (MUST)。
    pub fn set_peer_max_blocked_streams(&mut self, max: u64) {
        self.peer_max_blocked_streams = max;
    }

    /// ピアの SETTINGS_QPACK_BLOCKED_STREAMS を取得
    pub fn peer_max_blocked_streams(&self) -> u64 {
        self.peer_max_blocked_streams
    }

    /// 動的テーブル容量を設定
    pub fn set_table_capacity(&mut self, capacity: u64) {
        self.table.set_capacity(capacity);
    }

    /// 動的テーブルへの参照を取得
    pub fn table(&self) -> &DynamicTable {
        &self.table
    }

    /// 動的テーブルへの可変参照を取得
    pub fn table_mut(&mut self) -> &mut DynamicTable {
        &mut self.table
    }

    /// Known Received Count を取得
    pub fn known_received_count(&self) -> u64 {
        self.known_received_count
    }

    /// 動的テーブルの挿入カウント (エンコーダーが送信した数) を取得
    pub fn insert_count(&self) -> u64 {
        self.table.insert_count()
    }

    /// Known Received Count を更新 (デコーダーからの確認応答)
    pub fn acknowledge(&mut self, count: u64) {
        if count > self.known_received_count {
            self.known_received_count = count;
        }
        self.update_eviction_limit();
    }

    /// フィールドセクションの送信を記録する (RFC 9204 Section 2.1.1)
    ///
    /// Required Insert Count > 0 のフィールドセクションを送信した際に呼び出す。
    /// Section Acknowledgment でこの RIC が ack されるまで、
    /// 関連する動的テーブルエントリの eviction を防ぐ。
    pub fn track_section(&mut self, stream_id: u64, required_insert_count: u64) {
        if required_insert_count > 0 {
            self.unacked_section_rics
                .entry(stream_id)
                .or_default()
                .push_back(required_insert_count);
            self.update_eviction_limit();
        }
    }

    /// Section Acknowledgment を処理する (RFC 9204 Section 4.4.1)
    ///
    /// 指定ストリームの最も古い未 ack フィールドセクションを解放し、
    /// ack された Required Insert Count で Known Received Count を更新する
    /// (RFC 9204 Section 2.1.4)。
    /// 全て ack 済みの場合は false を返す。
    pub fn ack_section(&mut self, stream_id: u64) -> bool {
        let Some(rics) = self.unacked_section_rics.get_mut(&stream_id) else {
            return false;
        };
        let Some(ric) = rics.pop_front() else {
            return false;
        };
        if rics.is_empty() {
            self.unacked_section_rics.remove(&stream_id);
        }
        // Section Acknowledgment は、デコーダーがそのフィールドセクションのデコードに
        // 必要な全ての動的テーブル状態を受信したことを意味する (RFC 9204 Section 2.1.4)
        if ric > self.known_received_count {
            self.known_received_count = ric;
        }
        self.update_eviction_limit();
        true
    }

    /// Stream Cancellation を処理する (RFC 9204 Section 4.4.2)
    ///
    /// 指定ストリームの全ての未 ack フィールドセクションを解放する。
    pub fn cancel_stream(&mut self, stream_id: u64) {
        self.unacked_section_rics.remove(&stream_id);
        self.update_eviction_limit();
    }

    /// 指定ストリームの未 ack フィールドセクション数を取得
    pub fn unacked_section_count(&self, stream_id: u64) -> u64 {
        self.unacked_section_rics
            .get(&stream_id)
            .map_or(0, |rics| rics.len() as u64)
    }

    /// 未 ack セクションを持つストリーム数を取得
    pub fn blocked_streams_count(&self) -> usize {
        self.unacked_section_rics.len()
    }

    /// eviction_limit を再計算して動的テーブルに設定する
    ///
    /// eviction_limit = 未 ack フィールドセクションの最大 RIC
    /// (保守的: RIC 未満の全エントリを参照している可能性があるとみなす)
    fn update_eviction_limit(&mut self) {
        let max_ric = self
            .unacked_section_rics
            .values()
            .flat_map(|rics| rics.iter())
            .copied()
            .max()
            .unwrap_or(0);
        self.table.set_eviction_limit(max_ric);
    }

    /// ヘッダーリストをエンコード
    ///
    /// 動的テーブルを使用しない場合は静的テーブルのみで圧縮。
    /// 動的テーブルを使用する場合は Required Insert Count と Base を適切に設定。
    ///
    /// `blocked_streams_count` はブロックし得るストリーム数 (RIC > 0 の未 ack ストリーム数)。
    /// ピアの SETTINGS_QPACK_BLOCKED_STREAMS に達している場合は
    /// 動的テーブル参照を使用せず静的テーブルのみでエンコードする (RFC 9204 Section 2.1.2)。
    pub fn encode(
        &mut self,
        buf: &mut [u8],
        headers: &[Header],
        blocked_streams_count: usize,
    ) -> Option<usize> {
        // 動的テーブルが空の場合は静的テーブルのみを使用
        if self.table.is_empty() {
            self.last_required_insert_count = 0;
            return self.encode_static_only(buf, headers);
        }

        // ブロック可能ストリーム数が上限に達している場合は動的テーブル参照を抑止
        // (RFC 9204 Section 2.1.2)
        if blocked_streams_count as u64 >= self.peer_max_blocked_streams {
            self.last_required_insert_count = 0;
            return self.encode_static_only(buf, headers);
        }

        self.encode_with_dynamic(buf, headers)
    }

    /// 直前のエンコードで使用された Required Insert Count を取得
    pub fn last_required_insert_count(&self) -> u64 {
        self.last_required_insert_count
    }

    /// 静的テーブルのみを使用してエンコード
    fn encode_static_only(&self, buf: &mut [u8], headers: &[Header]) -> Option<usize> {
        let mut offset = 0;

        // Required Insert Count = 0
        if buf.is_empty() {
            return None;
        }
        buf[offset] = 0x00;
        offset += 1;

        // Delta Base = 0, Sign = 0
        if offset >= buf.len() {
            return None;
        }
        buf[offset] = 0x00;
        offset += 1;

        // ヘッダーをエンコード
        for header in headers {
            let encoded = self.encode_header_static_only(&mut buf[offset..], header)?;
            offset += encoded;
        }

        Some(offset)
    }

    /// 動的テーブルを使用してエンコード
    fn encode_with_dynamic(&mut self, buf: &mut [u8], headers: &[Header]) -> Option<usize> {
        // 参照する動的テーブルエントリの最大絶対インデックスを計算
        let mut required_insert_count = 0u64;

        for header in headers {
            // 動的テーブルで検索
            let (exact, name_only) = self.table.find_entry(&header.name, &header.value);
            if let Some(idx) = exact {
                if idx + 1 > required_insert_count {
                    required_insert_count = idx + 1;
                }
            } else if let Some(idx) = name_only
                && idx + 1 > required_insert_count
            {
                required_insert_count = idx + 1;
            }
        }

        self.last_required_insert_count = required_insert_count;

        let mut offset = 0;

        // Required Insert Count をエンコード
        let enc_insert_count = self.encode_required_insert_count(required_insert_count);
        offset += encode_integer_to_buf(&mut buf[offset..], enc_insert_count, 8, 0x00)?;

        // Base = Required Insert Count (シンプルな実装)
        // Sign = 0, Delta Base = 0
        if offset >= buf.len() {
            return None;
        }
        buf[offset] = 0x00;
        offset += 1;

        let base = required_insert_count;

        // ヘッダーをエンコード
        for header in headers {
            let encoded = self.encode_header_with_dynamic(&mut buf[offset..], header, base)?;
            offset += encoded;
        }

        Some(offset)
    }

    /// Required Insert Count をエンコード (RFC 9204 Section 4.5.1.1)
    fn encode_required_insert_count(&self, req_insert_count: u64) -> u64 {
        if req_insert_count == 0 {
            return 0;
        }

        let max_entries = self.max_table_capacity / 32;
        if max_entries == 0 {
            return 1;
        }

        let full_range = 2 * max_entries;
        (req_insert_count % full_range) + 1
    }

    /// 静的テーブルのみを使用してヘッダーをエンコード
    fn encode_header_static_only(&self, buf: &mut [u8], header: &Header) -> Option<usize> {
        let (exact_match, name_match) = find_static_entry(&header.name, &header.value);

        if let Some(index) = exact_match {
            // Indexed Field Line (静的テーブル)
            self.encode_indexed_field_static(buf, index)
        } else if let Some(index) = name_match {
            // Literal with Name Reference (静的テーブル)
            self.encode_literal_with_name_ref_static(buf, index, &header.value)
        } else {
            // Literal with Literal Name
            self.encode_literal_with_literal_name(buf, &header.name, &header.value)
        }
    }

    /// 動的テーブルを使用してヘッダーをエンコード
    fn encode_header_with_dynamic(
        &self,
        buf: &mut [u8],
        header: &Header,
        base: u64,
    ) -> Option<usize> {
        // 動的テーブルで検索
        let (dyn_exact, dyn_name) = self.table.find_entry(&header.name, &header.value);

        // 静的テーブルで検索
        let (static_exact, static_name) = find_static_entry(&header.name, &header.value);

        // 優先順位: 動的完全一致 > 静的完全一致 > 動的名前一致 > 静的名前一致 > リテラル
        if let Some(abs_index) = dyn_exact {
            // Indexed Field Line (動的テーブル)
            self.encode_indexed_field_dynamic(buf, abs_index, base)
        } else if let Some(index) = static_exact {
            // Indexed Field Line (静的テーブル)
            self.encode_indexed_field_static(buf, index)
        } else if let Some(abs_index) = dyn_name {
            // Literal with Name Reference (動的テーブル)
            self.encode_literal_with_name_ref_dynamic(buf, abs_index, &header.value, base)
        } else if let Some(index) = static_name {
            // Literal with Name Reference (静的テーブル)
            self.encode_literal_with_name_ref_static(buf, index, &header.value)
        } else {
            // Literal with Literal Name
            self.encode_literal_with_literal_name(buf, &header.name, &header.value)
        }
    }

    /// Indexed Field Line (静的テーブル) をエンコード
    ///
    /// Format: 1TNNNNNN (T=1 for static)
    fn encode_indexed_field_static(&self, buf: &mut [u8], index: usize) -> Option<usize> {
        // 0xc0 = 11000000 (T=1)
        encode_integer_to_buf(buf, index as u64, 6, 0xc0)
    }

    /// Indexed Field Line (動的テーブル) をエンコード
    ///
    /// Format: 1TNNNNNN (T=0 for dynamic)
    /// 相対インデックスを使用: relative_index = base - absolute_index - 1
    fn encode_indexed_field_dynamic(
        &self,
        buf: &mut [u8],
        absolute_index: u64,
        base: u64,
    ) -> Option<usize> {
        if absolute_index >= base {
            return None; // Post-Base indexing は未サポート
        }
        let relative_index = base - absolute_index - 1;
        // 0x80 = 10000000 (T=0)
        encode_integer_to_buf(buf, relative_index, 6, 0x80)
    }

    /// Literal with Name Reference (静的テーブル) をエンコード
    ///
    /// Format: 01NTNNNN (N=never index, T=1 for static)
    fn encode_literal_with_name_ref_static(
        &self,
        buf: &mut [u8],
        index: usize,
        value: &[u8],
    ) -> Option<usize> {
        // 0x50 = 01010000 (N=0, T=1)
        let mut offset = encode_integer_to_buf(buf, index as u64, 4, 0x50)?;

        // Value
        let value_len = self.encode_string(&mut buf[offset..], value)?;
        offset += value_len;

        Some(offset)
    }

    /// Literal with Name Reference (動的テーブル) をエンコード
    ///
    /// Format: 01NTNNNN (N=never index, T=0 for dynamic)
    /// 相対インデックスを使用
    fn encode_literal_with_name_ref_dynamic(
        &self,
        buf: &mut [u8],
        absolute_index: u64,
        value: &[u8],
        base: u64,
    ) -> Option<usize> {
        if absolute_index >= base {
            return None; // Post-Base indexing は未サポート
        }
        let relative_index = base - absolute_index - 1;
        // 0x40 = 01000000 (N=0, T=0)
        let mut offset = encode_integer_to_buf(buf, relative_index, 4, 0x40)?;

        // Value
        let value_len = self.encode_string(&mut buf[offset..], value)?;
        offset += value_len;

        Some(offset)
    }

    /// Literal with Literal Name をエンコード
    ///
    /// Format (RFC 9204 Section 4.5.4):
    /// ```text
    ///      0   1   2   3   4   5   6   7
    ///    +---+---+---+---+---+---+---+---+
    ///    | 0 | 0 | 1 | N |H|    Name     |
    ///    +---+---+---+---+---+-----------+
    ///    |  Name String (Length octets)  |
    ///    +-------------------------------+
    ///    |H|     Value Length (7+)       |
    ///    +---+---------------------------+
    ///    |  Value String (Length octets) |
    ///    +-------------------------------+
    /// ```
    fn encode_literal_with_literal_name(
        &self,
        buf: &mut [u8],
        name: &[u8],
        value: &[u8],
    ) -> Option<usize> {
        let mut offset = 0;

        // 名前のエンコード (3-bit prefix)
        // Prefix: 001N (N=0 for not "never indexed")
        // H ビット (bit 3) と Name Length (bits 0-2)
        let name_len = self.encode_string_with_prefix(buf, name, 3, 0x20)?;
        offset += name_len;

        // Value (string literal, 7-bit prefix)
        let value_len = self.encode_string(&mut buf[offset..], value)?;
        offset += value_len;

        Some(offset)
    }

    /// 文字列を指定された prefix bits でエンコード
    fn encode_string_with_prefix(
        &self,
        buf: &mut [u8],
        data: &[u8],
        prefix_bits: u8,
        prefix: u8,
    ) -> Option<usize> {
        if self.use_huffman {
            let huffman_len = huffman::encoded_len(data);
            if huffman_len < data.len() {
                // ハフマン符号化を使用
                // H ビットを設定: prefix に 0x08 を OR (3-bit prefix の場合)
                let h_bit = 1u8 << prefix_bits;
                let offset =
                    encode_integer_to_buf(buf, huffman_len as u64, prefix_bits, prefix | h_bit)?;
                huffman::encode(&mut buf[offset..], data)?;
                return Some(offset + huffman_len);
            }
        }

        // リテラル文字列 (H=0)
        let offset = encode_integer_to_buf(buf, data.len() as u64, prefix_bits, prefix)?;
        if buf.len() < offset + data.len() {
            return None;
        }
        buf[offset..offset + data.len()].copy_from_slice(data);
        Some(offset + data.len())
    }

    /// 文字列をエンコード (7-bit prefix)
    fn encode_string(&self, buf: &mut [u8], data: &[u8]) -> Option<usize> {
        if self.use_huffman {
            let huffman_len = huffman::encoded_len(data);
            if huffman_len < data.len() {
                let offset = encode_integer_to_buf(buf, huffman_len as u64, 7, 0x80)?;
                huffman::encode(&mut buf[offset..], data)?;
                return Some(offset + huffman_len);
            }
        }

        let offset = encode_integer_to_buf(buf, data.len() as u64, 7, 0x00)?;
        if buf.len() < offset + data.len() {
            return None;
        }
        buf[offset..offset + data.len()].copy_from_slice(data);
        Some(offset + data.len())
    }

    /// エントリを動的テーブルに挿入
    pub fn insert(&mut self, name: bytes::Bytes, value: bytes::Bytes) -> Option<u64> {
        self.table.insert(name, value)
    }

    /// 静的テーブル名参照でエントリを挿入
    pub fn insert_with_static_name_ref(
        &mut self,
        name_index: usize,
        value: bytes::Bytes,
    ) -> Option<u64> {
        let name = bytes::Bytes::from_static(STATIC_TABLE.get(name_index)?.name);
        self.table.insert(name, value)
    }

    /// 動的テーブル名参照でエントリを挿入
    pub fn insert_with_dynamic_name_ref(
        &mut self,
        relative_index: u64,
        value: bytes::Bytes,
    ) -> Option<u64> {
        let name = self
            .table
            .get_by_relative_index_encoder(relative_index)?
            .name
            .clone();
        self.table.insert(name, value)
    }
}

/// 整数をバッファにエンコード
fn encode_integer_to_buf(buf: &mut [u8], value: u64, prefix_bits: u8, prefix: u8) -> Option<usize> {
    let max_prefix = (1u64 << prefix_bits) - 1;

    if value < max_prefix {
        if buf.is_empty() {
            return None;
        }
        buf[0] = prefix | (value as u8);
        Some(1)
    } else {
        if buf.is_empty() {
            return None;
        }
        buf[0] = prefix | (max_prefix as u8);
        let mut offset = 1;
        let mut remaining = value - max_prefix;

        while remaining >= 128 {
            if offset >= buf.len() {
                return None;
            }
            buf[offset] = 0x80 | ((remaining & 0x7f) as u8);
            remaining >>= 7;
            offset += 1;
        }

        if offset >= buf.len() {
            return None;
        }
        buf[offset] = remaining as u8;
        Some(offset + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_indexed_field() {
        let encoder = Encoder::new().use_huffman(false);
        let headers = vec![Header::new(b":method", b"GET")];

        let mut buf = vec![0u8; 64];
        let len = encoder.encode(&mut buf, &headers).unwrap();

        // Required Insert Count (0) + Delta Base (0) + Indexed Field (17)
        assert!(len >= 3);
        assert_eq!(buf[0], 0x00); // Required Insert Count
        assert_eq!(buf[1], 0x00); // Delta Base
        assert_eq!(buf[2], 0xc0 | 17); // Indexed static table entry 17 (:method GET)
    }

    #[test]
    fn test_encode_literal_with_name_ref() {
        let encoder = Encoder::new().use_huffman(false);
        let headers = vec![Header::new(b":status", b"201")];

        let mut buf = vec![0u8; 64];
        let len = encoder.encode(&mut buf, &headers).unwrap();

        assert!(len >= 3);
        assert_eq!(buf[0], 0x00); // Required Insert Count
        assert_eq!(buf[1], 0x00); // Delta Base
        // Name reference to static table (24 is first :status)
    }

    #[test]
    fn test_encode_literal_with_literal_name() {
        let encoder = Encoder::new().use_huffman(false);
        let headers = vec![Header::new(b"x-custom", b"value")];

        let mut buf = vec![0u8; 64];
        let len = encoder.encode(&mut buf, &headers).unwrap();

        assert!(len > 4);
        assert_eq!(buf[0], 0x00); // Required Insert Count
        assert_eq!(buf[1], 0x00); // Delta Base
        // Literal with Literal Name: 001N|H|LLL
        // N=0, H=0, LLL=7 (max for 3-bit), 続きに 1 (8-7=1)
        // "x-custom" は 8 バイトなので 3-bit prefix を超える
        assert_eq!(buf[2], 0x27); // 0x20 | 7 = 0x27
        assert_eq!(buf[3], 0x01); // 8 - 7 = 1
    }

    #[test]
    fn test_encode_multiple_headers() {
        let encoder = Encoder::new();
        let headers = vec![
            Header::new(b":method", b"GET"),
            Header::new(b":scheme", b"https"),
            Header::new(b":path", b"/"),
            Header::new(b":authority", b"example.com"),
        ];

        let mut buf = vec![0u8; 128];
        let len = encoder.encode(&mut buf, &headers).unwrap();
        assert!(len > 0);
    }

    #[test]
    fn test_static_table_boundary() {
        // Test encoding index >= 64 (needs multi-byte encoding)
        let encoder = Encoder::new();

        // Index 98 (x-frame-options: sameorigin)
        let headers = vec![Header::new(b"x-frame-options", b"sameorigin")];
        let mut buf = vec![0u8; 64];
        let len = encoder.encode(&mut buf, &headers).unwrap();
        assert!(len > 0);
    }

    #[test]
    fn test_estimate_encoded_size() {
        let headers = vec![
            Header::new(b":method", b"GET"),
            Header::new(b"x-custom", b"value"),
        ];
        let estimate = estimate_encoded_size(&headers);
        assert!(estimate > 0);
    }

    #[test]
    fn test_dynamic_encoder_static_only() {
        let mut encoder = DynamicEncoder::new().use_huffman(false);
        let headers = vec![Header::new(b":method", b"GET")];

        let mut buf = vec![0u8; 64];
        let len = encoder.encode(&mut buf, &headers, 0).unwrap();

        assert!(len >= 3);
        assert_eq!(buf[0], 0x00); // Required Insert Count
        assert_eq!(buf[1], 0x00); // Delta Base
        assert_eq!(buf[2], 0xc0 | 17); // Indexed static table entry 17
    }

    #[test]
    fn test_dynamic_encoder_with_dynamic_table() {
        let mut encoder = DynamicEncoder::new().use_huffman(false);
        encoder.set_max_table_capacity(4096);
        encoder.set_table_capacity(1024);
        encoder.set_peer_max_blocked_streams(100);

        // 動的テーブルにエントリを挿入
        encoder.insert(
            bytes::Bytes::from_static(b":authority"),
            bytes::Bytes::from_static(b"www.example.com"),
        );
        assert_eq!(encoder.table().len(), 1);

        // エンコード
        let headers = vec![Header::new(b":authority", b"www.example.com")];
        let mut buf = vec![0u8; 64];
        let len = encoder.encode(&mut buf, &headers, 0).unwrap();

        // Required Insert Count > 0 (動的テーブルを参照)
        assert!(len >= 3);
        // Encoded Insert Count = (1 % (2 * (4096/32))) + 1 = 2
        assert!(buf[0] > 0);
    }

    #[test]
    fn test_dynamic_encoder_blocked_streams_limit() {
        let mut encoder = DynamicEncoder::new().use_huffman(false);
        encoder.set_max_table_capacity(4096);
        encoder.set_table_capacity(1024);
        encoder.set_peer_max_blocked_streams(2);

        // 動的テーブルにエントリを挿入
        encoder.insert(
            bytes::Bytes::from_static(b":authority"),
            bytes::Bytes::from_static(b"www.example.com"),
        );

        // blocked_streams_count < peer_max_blocked_streams: 動的テーブルを使用
        let headers = vec![Header::new(b":authority", b"www.example.com")];
        let mut buf = vec![0u8; 64];
        let _len = encoder.encode(&mut buf, &headers, 1).unwrap();
        assert!(buf[0] > 0); // RIC > 0

        // blocked_streams_count >= peer_max_blocked_streams: 静的テーブルのみ
        let mut buf2 = vec![0u8; 64];
        let len2 = encoder.encode(&mut buf2, &headers, 2).unwrap();
        assert_eq!(buf2[0], 0x00); // RIC = 0
        assert!(len2 > 0);
    }

    #[test]
    fn test_dynamic_encoder_prefer_dynamic() {
        let mut encoder = DynamicEncoder::new().use_huffman(false);
        encoder.set_max_table_capacity(4096);
        encoder.set_table_capacity(1024);
        encoder.set_peer_max_blocked_streams(100);

        // :method = GET は静的テーブルにある (インデックス 17)
        // しかし、動的テーブルに追加すると動的テーブルが優先される
        encoder.insert(
            bytes::Bytes::from_static(b":method"),
            bytes::Bytes::from_static(b"GET"),
        );

        let headers = vec![Header::new(b":method", b"GET")];
        let mut buf = vec![0u8; 64];
        let len = encoder.encode(&mut buf, &headers, 0).unwrap();

        assert!(len >= 3);
        // Required Insert Count が設定されている
        assert!(buf[0] > 0);
    }

    #[test]
    fn test_dynamic_encoder_insert_with_static_ref() {
        let mut encoder = DynamicEncoder::new();
        encoder.set_table_capacity(1024);

        // :authority (静的テーブルインデックス 0) を参照して挿入
        let idx = encoder.insert_with_static_name_ref(0, bytes::Bytes::from_static(b"example.com"));
        assert_eq!(idx, Some(0));

        let entry = encoder.table().get_by_absolute_index(0).unwrap();
        assert_eq!(entry.name, &b":authority"[..]);
        assert_eq!(entry.value, &b"example.com"[..]);
    }
}
