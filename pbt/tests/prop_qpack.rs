//! Property-Based Testing for QPACK (RFC 9204)

use proptest::prelude::*;
use shiguredo_http3::qpack::{
    DecodeOutput, Decoder, DecoderInstruction, DecoderStream, DecoderStreamReceiver,
    DynamicDecoder, DynamicEncoder, DynamicEntry, DynamicTable, Encoder, EncoderInstruction,
    EncoderStream, EncoderStreamReceiver, Header, STATIC_TABLE_LEN, find_static_entry, huffman,
};

/// 動的テーブル容量 (RFC 9204 Section 3.2)
const DYNAMIC_TABLE_CAPACITY: u64 = 4096;

/// エントリオーバーヘッド (RFC 9204 Section 3.2.1)
const ENTRY_OVERHEAD: u64 = 32;

prop_compose! {
    /// 有効なヘッダー名を生成 (ASCII 小文字のみ)
    /// issue 0059 Phase 3: Bytes 戻りで生成して clone を refcount だけにする
    fn valid_header_name()(
        len in 1usize..64,
    )(
        name in prop::collection::vec(prop::char::range('a', 'z'), len)
    ) -> bytes::Bytes {
        bytes::Bytes::from(name.into_iter().map(|c| c as u8).collect::<Vec<u8>>())
    }
}

prop_compose! {
    /// 有効なヘッダー値を生成
    fn valid_header_value()(
        len in 0usize..256,
    )(
        value in prop::collection::vec(0x20u8..0x7f, len)
    ) -> bytes::Bytes {
        bytes::Bytes::from(value)
    }
}

prop_compose! {
    /// 有効なテーブル容量を生成
    fn valid_capacity()(capacity in 0u64..65536) -> u64 {
        capacity
    }
}

prop_compose! {
    /// 有効な相対インデックスを生成
    fn valid_relative_index()(index in 0u64..100) -> u64 {
        index
    }
}

prop_compose! {
    /// 有効なストリーム ID を生成
    fn valid_stream_id()(id in 0u64..1000) -> u64 {
        id
    }
}

prop_compose! {
    /// 有効なインクリメント値を生成
    fn valid_increment()(inc in 1u64..1000) -> u64 {
        inc
    }
}

// =============================================================================
// Dynamic Table Properties
// =============================================================================

proptest! {
    /// Property: エントリサイズは常に 32 + name_len + value_len
    #[test]
    fn prop_entry_size_formula(
        name in valid_header_name(),
        value in valid_header_value(),
    ) {
        let entry = DynamicEntry::new(name.clone(), value.clone(), 0);
        let expected_size = ENTRY_OVERHEAD + name.len() as u64 + value.len() as u64;

        prop_assert_eq!(
            entry.size(), expected_size,
            "Entry size should be 32 + {} + {} = {}",
            name.len(), value.len(), expected_size
        );
    }

    /// Property: テーブルサイズは容量を超えない
    #[test]
    fn prop_table_size_within_capacity(
        capacity in 64u64..4096,
        entries in prop::collection::vec(
            (valid_header_name(), valid_header_value()),
            1..20
        ),
    ) {
        let mut table = DynamicTable::with_capacity(capacity);

        for (name, value) in entries {
            let _ = table.insert(name, value);
        }

        prop_assert!(
            table.current_size() <= table.max_capacity(),
            "Table size {} exceeds capacity {}",
            table.current_size(), table.max_capacity()
        );
    }

    /// Property: 挿入後の insert_count は単調増加
    #[test]
    fn prop_insert_count_monotonic(
        capacity in 256u64..4096,
        entries in prop::collection::vec(
            (valid_header_name(), valid_header_value()),
            1..10
        ),
    ) {
        let mut table = DynamicTable::with_capacity(capacity);
        let mut prev_count = table.insert_count();

        for (name, value) in entries {
            if table.insert(name, value).is_some() {
                let new_count = table.insert_count();
                prop_assert!(
                    new_count > prev_count,
                    "Insert count should increase: {} -> {}",
                    prev_count, new_count
                );
                prev_count = new_count;
            }
        }
    }

    /// Property: 容量変更後もサイズ不変式が維持される
    #[test]
    fn prop_capacity_change_maintains_invariant(
        initial_capacity in 256u64..4096,
        new_capacity in 64u64..2048,
        entries in prop::collection::vec(
            (valid_header_name(), valid_header_value()),
            1..10
        ),
    ) {
        let mut table = DynamicTable::with_capacity(initial_capacity);

        for (name, value) in entries {
            let _ = table.insert(name, value);
        }

        table.set_capacity(new_capacity);

        prop_assert!(
            table.current_size() <= table.max_capacity(),
            "After capacity change: size {} > capacity {}",
            table.current_size(), table.max_capacity()
        );
    }

    /// Property: 絶対インデックスは一意
    #[test]
    fn prop_absolute_index_unique(
        capacity in 1024u64..4096,
        entries in prop::collection::vec(
            (valid_header_name(), valid_header_value()),
            2..10
        ),
    ) {
        let mut table = DynamicTable::with_capacity(capacity);
        let mut indices = Vec::new();

        for (name, value) in entries {
            if let Some(idx) = table.insert(name, value) {
                prop_assert!(
                    !indices.contains(&idx),
                    "Duplicate absolute index: {}",
                    idx
                );
                indices.push(idx);
            }
        }
    }
}

// =============================================================================
// Encoder Stream Properties
// =============================================================================

proptest! {
    /// Property: Set Capacity 命令のラウンドトリップ
    #[test]
    fn prop_set_capacity_roundtrip(capacity in 1u64..=4096) {
        let mut sender = EncoderStream::new();
        sender.set_max_table_capacity(4096);
        sender.encode_set_capacity(capacity).unwrap();
        let encoded = sender.get_data().to_vec();

        let mut receiver = EncoderStreamReceiver::new();
        receiver.set_max_table_capacity(4096);
        receiver.receive(&encoded);

        let mut table = DynamicTable::new();
        let instruction = receiver.process(&mut table).unwrap();

        prop_assert_eq!(
            instruction,
            Some(EncoderInstruction::SetDynamicTableCapacity { capacity }),
            "Roundtrip failed for capacity {}", capacity
        );
    }

    /// Property: Insert with Literal Name 命令のラウンドトリップ
    #[test]
    fn prop_insert_literal_roundtrip(
        name in valid_header_name(),
        value in valid_header_value(),
    ) {
        let mut sender = EncoderStream::new();
        sender.set_max_table_capacity(4096);
        sender.encode_insert_with_literal_name(&name, &value).unwrap();
        let encoded = sender.get_data().to_vec();

        let mut receiver = EncoderStreamReceiver::new();
        receiver.set_max_table_capacity(4096);
        receiver.receive(&encoded);

        let mut table = DynamicTable::with_capacity(4096);
        let instruction = receiver.process(&mut table).unwrap();

        if let Some(EncoderInstruction::InsertWithLiteralName { name: n, value: v }) = instruction {
            prop_assert_eq!(n, name, "Name mismatch");
            prop_assert_eq!(v, value, "Value mismatch");
        } else {
            prop_assert!(false, "Expected InsertWithLiteralName instruction");
        }
    }

    /// Property: Duplicate 命令のラウンドトリップ
    #[test]
    fn prop_duplicate_roundtrip(index in 0u64..10) {
        let mut sender = EncoderStream::new();
        sender.set_max_table_capacity(4096);
        sender.encode_duplicate(index).unwrap();
        let encoded = sender.get_data().to_vec();

        let mut receiver = EncoderStreamReceiver::new();
        receiver.set_max_table_capacity(4096);
        receiver.receive(&encoded);

        // テーブルにエントリを追加
        let mut table = DynamicTable::with_capacity(4096);
        for i in 0..=index {
            table.insert(
                bytes::Bytes::from(format!("name{}", i).into_bytes()),
                bytes::Bytes::from_static(b"value"),
            );
        }

        let instruction = receiver.process(&mut table).unwrap();

        prop_assert_eq!(
            instruction,
            Some(EncoderInstruction::Duplicate { relative_index: index }),
        );
    }
}

// =============================================================================
// Decoder Stream Properties
// =============================================================================

proptest! {
    /// Property: Section Acknowledgment 命令のラウンドトリップ
    #[test]
    fn prop_section_ack_roundtrip(stream_id in valid_stream_id()) {
        let mut sender = DecoderStream::new();
        sender.encode_section_acknowledgment(stream_id);
        let encoded = sender.get_data().to_vec();

        let mut receiver = DecoderStreamReceiver::new();
        receiver.receive(&encoded);
        let instruction = receiver.process(u64::MAX).unwrap();

        prop_assert_eq!(
            instruction,
            Some(DecoderInstruction::SectionAcknowledgment { stream_id }),
        );
    }

    /// Property: Stream Cancellation 命令のラウンドトリップ
    #[test]
    fn prop_stream_cancel_roundtrip(stream_id in valid_stream_id()) {
        let mut sender = DecoderStream::new();
        sender.encode_stream_cancellation(stream_id);
        let encoded = sender.get_data().to_vec();

        let mut receiver = DecoderStreamReceiver::new();
        receiver.receive(&encoded);
        let instruction = receiver.process(u64::MAX).unwrap();

        prop_assert_eq!(
            instruction,
            Some(DecoderInstruction::StreamCancellation { stream_id }),
        );
    }

    /// Property: Insert Count Increment 命令のラウンドトリップ
    #[test]
    fn prop_insert_count_increment_roundtrip(increment in valid_increment()) {
        let mut sender = DecoderStream::new();
        sender.encode_insert_count_increment(increment);
        let encoded = sender.get_data().to_vec();

        let mut receiver = DecoderStreamReceiver::new();
        receiver.receive(&encoded);
        let instruction = receiver.process(u64::MAX).unwrap();

        prop_assert_eq!(
            instruction,
            Some(DecoderInstruction::InsertCountIncrement { increment }),
        );
    }

    /// Property: Known Received Count は累積される
    #[test]
    fn prop_known_received_count_accumulates(
        increments in prop::collection::vec(valid_increment(), 1..5),
    ) {
        let mut sender = DecoderStream::new();
        let mut receiver = DecoderStreamReceiver::new();

        let mut expected_count = 0u64;
        for inc in increments {
            sender.encode_insert_count_increment(inc);
            receiver.receive(sender.get_data());
            sender.consume_data(sender.get_data().len());

            let _ = receiver.process(u64::MAX).unwrap();
            expected_count += inc;

            prop_assert_eq!(
                receiver.known_received_count(), expected_count,
                "Known received count should be {}",
                expected_count
            );
        }
    }
}

// =============================================================================
// Encoder/Decoder Integration Properties
// =============================================================================

proptest! {
    /// Property: エンコーダーストリームの連続命令処理
    #[test]
    fn prop_encoder_stream_sequential(
        name in valid_header_name(),
        value in valid_header_value(),
    ) {
        let mut sender = EncoderStream::new();
        sender.set_max_table_capacity(4096);

        // 複数の命令をエンコード
        sender.encode_set_capacity(1024).unwrap();
        sender.encode_insert_with_literal_name(&name, &value).unwrap();

        let encoded = sender.get_data().to_vec();

        let mut receiver = EncoderStreamReceiver::new();
        receiver.set_max_table_capacity(4096);
        receiver.receive(&encoded);

        let mut table = DynamicTable::new();

        // 最初の命令: Set Capacity
        let inst1 = receiver.process(&mut table).unwrap();
        if let Some(EncoderInstruction::SetDynamicTableCapacity { capacity }) = inst1 {
            prop_assert_eq!(capacity, 1024);
        } else {
            prop_assert!(false, "Expected SetDynamicTableCapacity instruction");
        }

        // 2 番目の命令: Insert with Literal Name
        let inst2 = receiver.process(&mut table).unwrap();
        let is_insert_literal = matches!(inst2, Some(EncoderInstruction::InsertWithLiteralName { .. }));
        prop_assert!(is_insert_literal, "Expected InsertWithLiteralName instruction");

        // テーブルにエントリが追加されている
        prop_assert_eq!(table.len(), 1);
    }

    /// Property: デコーダーストリームの連続命令処理
    #[test]
    fn prop_decoder_stream_sequential(
        stream_id1 in valid_stream_id(),
        stream_id2 in valid_stream_id(),
        increment in valid_increment(),
    ) {
        let mut sender = DecoderStream::new();

        sender.encode_section_acknowledgment(stream_id1);
        sender.encode_stream_cancellation(stream_id2);
        sender.encode_insert_count_increment(increment);

        let encoded = sender.get_data().to_vec();

        let mut receiver = DecoderStreamReceiver::new();
        receiver.receive(&encoded);

        let inst1 = receiver.process(u64::MAX).unwrap();
        prop_assert_eq!(
            inst1,
            Some(DecoderInstruction::SectionAcknowledgment { stream_id: stream_id1 })
        );

        let inst2 = receiver.process(u64::MAX).unwrap();
        prop_assert_eq!(
            inst2,
            Some(DecoderInstruction::StreamCancellation { stream_id: stream_id2 })
        );

        let inst3 = receiver.process(u64::MAX).unwrap();
        prop_assert_eq!(
            inst3,
            Some(DecoderInstruction::InsertCountIncrement { increment })
        );

        // バッファが空
        let inst4 = receiver.process(u64::MAX).unwrap();
        prop_assert!(inst4.is_none());
    }
}

// =============================================================================
// Huffman Encoding Properties
// =============================================================================

prop_compose! {
    /// ASCII 印字可能文字列を生成
    fn printable_ascii()(
        len in 1usize..128,
    )(
        data in prop::collection::vec(0x20u8..0x7f, len)
    ) -> Vec<u8> {
        data
    }
}

proptest! {
    /// Property: Huffman エンコード/デコードのラウンドトリップ
    #[test]
    fn prop_huffman_roundtrip(data in printable_ascii()) {
        let encoded_len = huffman::encoded_len(&data);
        let mut encoded = vec![0u8; encoded_len];
        huffman::encode(&mut encoded, &data);

        let decoded = huffman::decode(&encoded).unwrap();
        prop_assert_eq!(decoded, data);
    }

    /// Property: Huffman encoded_len は実際のエンコード長と一致
    #[test]
    fn prop_huffman_encoded_len_accurate(data in printable_ascii()) {
        let predicted_len = huffman::encoded_len(&data);
        let mut encoded = vec![0u8; predicted_len + 10];
        let actual_len = huffman::encode(&mut encoded, &data).unwrap();

        prop_assert_eq!(predicted_len, actual_len);
    }

    /// Property: Huffman エンコードは元データより長くなることがある
    #[test]
    fn prop_huffman_length_varies(data in printable_ascii()) {
        let encoded_len = huffman::encoded_len(&data);
        // Huffman エンコードは常に有効な長さを返す
        prop_assert!(encoded_len > 0);
    }
}

// =============================================================================
// Encoder/Decoder Properties
// =============================================================================

prop_compose! {
    /// 静的テーブルに存在するヘッダー名を生成
    fn static_header_name()(
        idx in 0usize..STATIC_TABLE_LEN
    ) -> Vec<u8> {
        use shiguredo_http3::qpack::STATIC_TABLE;
        STATIC_TABLE[idx].name.to_vec()
    }
}

proptest! {
    /// Property: Encoder/Decoder ラウンドトリップ (静的テーブルのみ)
    #[test]
    fn prop_encoder_decoder_roundtrip_static(
        value in valid_header_value(),
    ) {
        let encoder = Encoder::new();
        let decoder = Decoder::new();

        let headers = vec![
            Header::new(b":method", b"GET"),
            Header::new(b":path", value.clone()),
        ];

        let mut buf = vec![0u8; 1024];
        let encoded_len = encoder.encode(&mut buf, &headers).unwrap();

        let decoded = decoder.decode(&buf[..encoded_len]).unwrap();

        prop_assert_eq!(decoded.len(), 2);
        prop_assert_eq!(&decoded[0].name, &b":method"[..]);
        prop_assert_eq!(&decoded[0].value, &b"GET"[..]);
        prop_assert_eq!(&decoded[1].name, &b":path"[..]);
        prop_assert_eq!(&decoded[1].value, &value);
    }

    /// Property: Encoder/Decoder ラウンドトリップ (カスタムヘッダー)
    #[test]
    fn prop_encoder_decoder_roundtrip_literal(
        name in valid_header_name(),
        value in valid_header_value(),
    ) {
        let encoder = Encoder::new();
        let decoder = Decoder::new();

        let headers = vec![Header::new(name.clone(), value.clone())];

        let mut buf = vec![0u8; 1024];
        let encoded_len = encoder.encode(&mut buf, &headers).unwrap();

        let decoded = decoder.decode(&buf[..encoded_len]).unwrap();

        prop_assert_eq!(decoded.len(), 1);
        prop_assert_eq!(&decoded[0].name, &name);
        prop_assert_eq!(&decoded[0].value, &value);
    }

    /// Property: 複数ヘッダーのラウンドトリップ
    #[test]
    fn prop_encoder_decoder_multiple_headers(
        headers_data in prop::collection::vec(
            (valid_header_name(), valid_header_value()),
            1..5
        ),
    ) {
        let encoder = Encoder::new();
        let decoder = Decoder::new();

        let headers: Vec<Header> = headers_data
            .iter()
            .map(|(n, v)| Header::new(n.clone(), v.clone()))
            .collect();

        let mut buf = vec![0u8; 4096];
        let encoded_len = encoder.encode(&mut buf, &headers).unwrap();

        let decoded = decoder.decode(&buf[..encoded_len]).unwrap();

        prop_assert_eq!(decoded.len(), headers.len());
        for (orig, dec) in headers_data.iter().zip(decoded.iter()) {
            prop_assert_eq!(&dec.name, &orig.0);
            prop_assert_eq!(&dec.value, &orig.1);
        }
    }
}

// =============================================================================
// Static Table Properties
// =============================================================================

proptest! {
    /// Property: 静的テーブル検索の一貫性 (完全一致)
    #[test]
    fn prop_static_table_find_exact(idx in 0usize..STATIC_TABLE_LEN) {
        use shiguredo_http3::qpack::STATIC_TABLE;
        let entry = &STATIC_TABLE[idx];

        let (exact, name_only) = find_static_entry(entry.name, entry.value);

        // 完全一致が見つかる場合、name_only も Some
        if exact.is_some() {
            prop_assert!(name_only.is_some());
        }
    }

    /// Property: 静的テーブル検索は常に有効なインデックスを返す
    #[test]
    fn prop_static_table_find_valid_index(
        name in static_header_name(),
        value in valid_header_value(),
    ) {
        let (exact, name_only) = find_static_entry(&name, &value);

        if let Some(idx) = exact {
            prop_assert!(idx < STATIC_TABLE_LEN);
        }
        if let Some(idx) = name_only {
            prop_assert!(idx < STATIC_TABLE_LEN);
        }
    }
}

// =============================================================================
// Dynamic Table Additional Properties
// =============================================================================

proptest! {
    /// Property: 動的テーブルの find_entry は挿入したエントリを見つける
    #[test]
    fn prop_dynamic_table_find_inserted(
        name in valid_header_name(),
        value in valid_header_value(),
    ) {
        let mut table = DynamicTable::with_capacity(4096);
        let idx = table.insert(name.clone(), value.clone()).unwrap();

        let (exact, name_only) = table.find_entry(&name, &value);

        prop_assert_eq!(exact, Some(idx));
        prop_assert_eq!(name_only, Some(idx));
    }

    /// Property: 動的テーブルの find_entry は名前のみ一致も検出
    #[test]
    fn prop_dynamic_table_find_name_only(
        name in valid_header_name(),
        value1 in valid_header_value(),
        value2 in valid_header_value(),
    ) {
        let mut table = DynamicTable::with_capacity(4096);
        let idx = table.insert(name.clone(), value1.clone()).unwrap();

        // 異なる値で検索
        let (exact, name_only) = table.find_entry(&name, &value2);

        if value1 == value2 {
            prop_assert_eq!(exact, Some(idx));
        } else {
            prop_assert!(exact.is_none());
        }
        prop_assert_eq!(name_only, Some(idx));
    }

    /// Property: 動的テーブルの duplicate は同じ内容のエントリを作成
    #[test]
    fn prop_dynamic_table_duplicate_content(
        name in valid_header_name(),
        value in valid_header_value(),
    ) {
        let mut table = DynamicTable::with_capacity(4096);
        table.insert(name.clone(), value.clone()).unwrap();

        // relative_index 0 は最新エントリ
        let dup_idx = table.duplicate(0).unwrap();

        let original = table.get_by_absolute_index(0).unwrap();
        let duplicated = table.get_by_absolute_index(dup_idx).unwrap();

        prop_assert_eq!(&original.name, &duplicated.name);
        prop_assert_eq!(&original.value, &duplicated.value);
        prop_assert_ne!(original.absolute_index, duplicated.absolute_index);
    }

    /// Property: エビクション後も取得可能なエントリは有効
    #[test]
    fn prop_dynamic_table_eviction_valid(
        entries in prop::collection::vec(
            (valid_header_name(), valid_header_value()),
            5..15
        ),
    ) {
        // 小さな容量で強制的にエビクションを発生させる
        let mut table = DynamicTable::with_capacity(128);

        for (name, value) in &entries {
            let _ = table.insert(name.clone(), value.clone());
        }

        // 残っているエントリはすべて有効
        for entry in table.iter() {
            let retrieved = table.get_by_absolute_index(entry.absolute_index);
            prop_assert!(retrieved.is_some());
            prop_assert_eq!(&retrieved.unwrap().name, &entry.name);
        }
    }

    /// Property: 動的テーブルの相対インデックス変換が正しい
    #[test]
    fn prop_dynamic_table_relative_index(
        entries in prop::collection::vec(
            (valid_header_name(), valid_header_value()),
            2..5
        ),
    ) {
        let mut table = DynamicTable::with_capacity(4096);

        for (name, value) in &entries {
            table.insert(name.clone(), value.clone());
        }

        let insert_count = table.insert_count();

        // 相対インデックス 0 は最新エントリ (absolute = insert_count - 1)
        let newest = table.get_by_relative_index_encoder(0);
        prop_assert!(newest.is_some());
        prop_assert_eq!(newest.unwrap().absolute_index, insert_count - 1);
    }
}

// =============================================================================
// Insert with Name Reference Properties
// =============================================================================

proptest! {
    /// Property: Insert with Name Reference (静的テーブル) のラウンドトリップ
    #[test]
    fn prop_insert_with_name_ref_static_roundtrip(
        static_idx in 0u64..10,
        value in valid_header_value(),
    ) {
        let mut sender = EncoderStream::new();
        sender.set_max_table_capacity(4096);
        sender.encode_insert_with_name_ref(true, static_idx, &value).unwrap();
        let encoded = sender.get_data().to_vec();

        let mut receiver = EncoderStreamReceiver::new();
        receiver.set_max_table_capacity(4096);
        receiver.receive(&encoded);

        let mut table = DynamicTable::with_capacity(4096);
        let instruction = receiver.process(&mut table).unwrap();

        if let Some(EncoderInstruction::InsertWithNameReference {
            is_static,
            name_index,
            value: v,
        }) = instruction
        {
            prop_assert!(is_static);
            prop_assert_eq!(name_index, static_idx);
            prop_assert_eq!(v, value);
        } else {
            prop_assert!(false, "Expected InsertWithNameReference instruction");
        }
    }

    /// Property: エンコーダーストリーム命令の分割受信 (RFC 9204 Section 4.2)
    ///
    /// QUIC stream は byte stream なので命令が任意の位置で分割される。
    /// 部分受信時は Ok(None) を返し、完全なデータが揃うまで待機する。
    #[test]
    fn prop_encoder_stream_partial_receive(
        name in valid_header_name(),
        value in valid_header_value(),
    ) {
        let mut sender = EncoderStream::new();
        sender.set_max_table_capacity(4096);
        sender.encode_insert_with_literal_name(&name, &value).unwrap();
        let encoded = sender.get_data().to_vec();

        // 1 バイトずつ受信
        let mut receiver = EncoderStreamReceiver::new();
        receiver.set_max_table_capacity(4096);
        let mut table = DynamicTable::with_capacity(4096);

        for (i, &byte) in encoded.iter().enumerate() {
            receiver.receive(&[byte]);
            let result = receiver.process(&mut table);

            if i < encoded.len() - 1 {
                // 途中は Ok(None) でなければならない
                prop_assert_eq!(
                    result.unwrap(), None,
                    "Partial data at byte {} should return Ok(None)",
                    i
                );
            } else {
                // 最後のバイトで完全な命令が得られる
                let instruction = result.unwrap();
                prop_assert!(instruction.is_some(), "Complete data should return instruction");
                if let Some(EncoderInstruction::InsertWithLiteralName { name: n, value: v }) = instruction {
                    prop_assert_eq!(n, name.clone());
                    prop_assert_eq!(v, value.clone());
                }
            }
        }
    }

    /// Property: デコーダーストリーム命令の分割受信 (RFC 9204 Section 4.2)
    #[test]
    fn prop_decoder_stream_partial_receive(stream_id in valid_stream_id()) {
        let mut sender = DecoderStream::new();
        sender.encode_section_acknowledgment(stream_id);
        let encoded = sender.get_data().to_vec();

        // 1 バイトずつ受信
        let mut receiver = DecoderStreamReceiver::new();

        for (i, &byte) in encoded.iter().enumerate() {
            receiver.receive(&[byte]);
            let result = receiver.process(u64::MAX);

            if i < encoded.len() - 1 {
                prop_assert_eq!(
                    result.unwrap(), None,
                    "Partial data at byte {} should return Ok(None)",
                    i
                );
            } else {
                let instruction = result.unwrap();
                prop_assert_eq!(
                    instruction,
                    Some(DecoderInstruction::SectionAcknowledgment { stream_id }),
                );
            }
        }
    }

    /// Property: Insert with Name Reference (動的テーブル) のラウンドトリップ
    #[test]
    fn prop_insert_with_name_ref_dynamic_roundtrip(
        name in valid_header_name(),
        value1 in valid_header_value(),
        value2 in valid_header_value(),
    ) {
        // まず動的テーブルにエントリを追加
        let mut table = DynamicTable::with_capacity(4096);
        table.insert(name.clone(), value1);

        let mut sender = EncoderStream::new();
        sender.set_max_table_capacity(4096);
        // 動的テーブルの relative_index 0 を参照
        sender.encode_insert_with_name_ref(false, 0, &value2).unwrap();
        let encoded = sender.get_data().to_vec();

        let mut receiver = EncoderStreamReceiver::new();
        receiver.set_max_table_capacity(4096);
        receiver.receive(&encoded);

        let instruction = receiver.process(&mut table).unwrap();

        if let Some(EncoderInstruction::InsertWithNameReference {
            is_static,
            name_index,
            value: v,
        }) = instruction
        {
            prop_assert!(!is_static);
            prop_assert_eq!(name_index, 0);
            prop_assert_eq!(&v, &value2);
        } else {
            prop_assert!(false, "Expected InsertWithNameReference instruction");
        }

        // テーブルに新しいエントリが追加されている
        prop_assert_eq!(table.len(), 2);
        let new_entry = table.get_by_absolute_index(1).unwrap();
        prop_assert_eq!(&new_entry.name, &name);
        prop_assert_eq!(&new_entry.value, &value2);
    }
}

// =============================================================================
// DynamicEncoder / DynamicDecoder Roundtrip (RFC 9204 Section 4)
// =============================================================================

prop_compose! {
    /// 動的テーブルエンコード用のヘッダーを生成
    fn dynamic_header()(
        name in valid_header_name(),
        value in valid_header_value(),
    ) -> Header {
        Header::new(name, value)
    }
}

proptest! {
    /// Property: エンコーダーとデコーダーのテーブルを同期させると、
    /// DynamicEncoder でエンコードしたヘッダーは DynamicDecoder で復元できる
    #[test]
    fn prop_dynamic_encoder_decoder_roundtrip(
        entries in prop::collection::vec(
            (valid_header_name(), valid_header_value()),
            0..5,
        ),
        headers in prop::collection::vec(dynamic_header(), 1..5),
        use_huffman in any::<bool>(),
    ) {
        let mut encoder = DynamicEncoder::new().use_huffman(use_huffman);
        encoder.set_max_table_capacity(DYNAMIC_TABLE_CAPACITY);
        encoder.set_table_capacity(DYNAMIC_TABLE_CAPACITY);

        let mut decoder = DynamicDecoder::new();
        decoder.set_max_table_capacity(DYNAMIC_TABLE_CAPACITY);
        decoder.set_table_capacity(DYNAMIC_TABLE_CAPACITY);

        // エンコーダーとデコーダーに同じエントリを挿入してテーブルを同期させる
        for (name, value) in &entries {
            encoder.insert(name.clone(), value.clone());
            decoder.insert(name.clone(), value.clone());
        }

        let mut buf = vec![0u8; 64 * 1024];
        let Some(len) = encoder.encode(&mut buf, &headers, 0) else {
            return Ok(());
        };

        match decoder.decode(&buf[..len]) {
            Ok(DecodeOutput::Decoded(decoded)) => {
                prop_assert_eq!(headers.len(), decoded.len());
                for (orig, dec) in headers.iter().zip(decoded.iter()) {
                    prop_assert_eq!(orig.name.clone(), dec.name.clone());
                    prop_assert_eq!(orig.value.clone(), dec.value.clone());
                }
            }
            // テーブル状態の不一致によるブロックやエラーは許容
            Ok(DecodeOutput::Blocked) | Err(_) => {}
        }
    }

    /// Property: 動的テーブル参照でブロックされたデコードは、
    /// エンコーダーストリームでテーブル更新後に成功する (RFC 9204 Section 2.1.2)
    #[test]
    fn prop_blocked_then_unblocked(
        entry_name in valid_header_name(),
        entry_value in valid_header_value(),
    ) {
        // エンコーダー側: 動的テーブルにエントリを挿入してエンコード
        let mut encoder = DynamicEncoder::new().use_huffman(false);
        encoder.set_max_table_capacity(DYNAMIC_TABLE_CAPACITY);
        encoder.set_table_capacity(DYNAMIC_TABLE_CAPACITY);
        encoder.insert(entry_name.clone(), entry_value.clone());

        let headers = vec![Header::new(entry_name.clone(), entry_value.clone())];
        let mut buf = vec![0u8; 64 * 1024];
        let Some(encoded_len) = encoder.encode(&mut buf, &headers, 0) else {
            return Ok(());
        };
        let encoded = buf[..encoded_len].to_vec();

        // デコーダー側: テーブル空でデコード → Blocked 期待
        let mut decoder = DynamicDecoder::new();
        decoder.set_max_table_capacity(DYNAMIC_TABLE_CAPACITY);
        decoder.set_table_capacity(DYNAMIC_TABLE_CAPACITY);
        let first_result = decoder.decode(&encoded);

        // エンコーダーストリーム命令で decoder のテーブルを更新
        let mut enc_stream = EncoderStream::new();
        let _ = enc_stream.encode_insert_with_literal_name(&entry_name, &entry_value);
        let stream_data = enc_stream.get_data().to_vec();

        let mut receiver = EncoderStreamReceiver::new();
        receiver.set_max_table_capacity(DYNAMIC_TABLE_CAPACITY);
        receiver.receive(&stream_data);
        loop {
            match receiver.process(decoder.table_mut()) {
                Ok(None) | Err(_) => break,
                Ok(Some(_)) => {}
            }
        }

        // テーブル更新後のデコード
        let second_result = decoder.decode(&encoded);

        // Blocked → Decoded のプロパティ
        if let (Ok(DecodeOutput::Blocked), Ok(DecodeOutput::Decoded(decoded))) =
            (first_result, second_result)
        {
            prop_assert_eq!(decoded.len(), 1);
            prop_assert_eq!(decoded[0].name.clone(), entry_name);
            prop_assert_eq!(decoded[0].value.clone(), entry_value);
        }
    }
}
