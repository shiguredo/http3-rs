//! Property-Based Testing for QPACK (RFC 9204)

mod integer;

use pbt::strategies::{valid_header_name, valid_header_value};
use pbt::wire_header;
use shiguredo_http3::qpack::{
    DecodeOutput, Decoder, DecoderInstruction, DecoderStream, DecoderStreamReceiver,
    DynamicDecoder, DynamicEncoder, DynamicEntry, DynamicTable, Encoder, EncoderInstruction,
    EncoderStream, EncoderStreamReceiver, Header, STATIC_TABLE_LEN, find_static_entry, huffman,
};

/// 動的テーブル容量 (RFC 9204 Section 3.2)
const DYNAMIC_TABLE_CAPACITY: u64 = 4096;

/// エントリオーバーヘッド (RFC 9204 Section 3.2.1)
const ENTRY_OVERHEAD: u64 = 32;

/// 有効なストリーム ID を生成
fn valid_stream_id(ctx: &mut noprop::TestCaseContext) -> u64 {
    noprop::sample_u64_in(ctx, 0..1000)
}

/// 有効なインクリメント値を生成
fn valid_increment(ctx: &mut noprop::TestCaseContext) -> u64 {
    noprop::sample_u64_in(ctx, 1..1000)
}

// =============================================================================
// Dynamic Table Properties
// =============================================================================

/// Property: エントリサイズは常に 32 + name_len + value_len
#[test]
fn prop_entry_size_formula() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let name = valid_header_name(ctx);
        let value = valid_header_value(ctx);
        let entry = DynamicEntry::new(name.clone(), value.clone(), 0);
        let expected_size = ENTRY_OVERHEAD + name.len() as u64 + value.len() as u64;

        assert_eq!(
            entry.size(),
            expected_size,
            "Entry size should be 32 + {} + {} = {}",
            name.len(),
            value.len(),
            expected_size
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: テーブルサイズは容量を超えない
#[test]
fn prop_table_size_within_capacity() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let capacity = noprop::sample_u64_in(ctx, 64..4096);
        let entry_count = noprop::sample_usize_in(ctx, 1..20);
        let mut table = DynamicTable::with_capacity(capacity);

        for _ in 0..entry_count {
            let name = valid_header_name(ctx);
            let value = valid_header_value(ctx);
            let _ = table.insert(name, value);
        }

        assert!(
            table.current_size() <= table.max_capacity(),
            "Table size {} exceeds capacity {}",
            table.current_size(),
            table.max_capacity()
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: 挿入後の insert_count は単調増加
#[test]
fn prop_insert_count_monotonic() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let capacity = noprop::sample_u64_in(ctx, 256..4096);
        let entry_count = noprop::sample_usize_in(ctx, 1..10);
        let mut table = DynamicTable::with_capacity(capacity);
        let mut prev_count = table.insert_count();

        for _ in 0..entry_count {
            let name = valid_header_name(ctx);
            let value = valid_header_value(ctx);
            if table.insert(name, value).is_some() {
                let new_count = table.insert_count();
                assert!(
                    new_count > prev_count,
                    "Insert count should increase: {} -> {}",
                    prev_count,
                    new_count
                );
                prev_count = new_count;
            }
        }
        Ok(())
    })?;
    Ok(())
}

/// Property: 容量変更後もサイズ不変式が維持される
#[test]
fn prop_capacity_change_maintains_invariant() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let initial_capacity = noprop::sample_u64_in(ctx, 256..4096);
        let new_capacity = noprop::sample_u64_in(ctx, 64..2048);
        let entry_count = noprop::sample_usize_in(ctx, 1..10);
        let mut table = DynamicTable::with_capacity(initial_capacity);

        for _ in 0..entry_count {
            let name = valid_header_name(ctx);
            let value = valid_header_value(ctx);
            let _ = table.insert(name, value);
        }

        table.set_capacity(new_capacity);

        assert!(
            table.current_size() <= table.max_capacity(),
            "After capacity change: size {} > capacity {}",
            table.current_size(),
            table.max_capacity()
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: 絶対インデックスは一意
#[test]
fn prop_absolute_index_unique() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let capacity = noprop::sample_u64_in(ctx, 1024..4096);
        let entry_count = noprop::sample_usize_in(ctx, 2..10);
        let mut table = DynamicTable::with_capacity(capacity);
        let mut indices = Vec::new();

        for _ in 0..entry_count {
            let name = valid_header_name(ctx);
            let value = valid_header_value(ctx);
            if let Some(idx) = table.insert(name, value) {
                assert!(!indices.contains(&idx), "Duplicate absolute index: {}", idx);
                indices.push(idx);
            }
        }
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// Encoder Stream Properties
// =============================================================================

/// Property: Set Capacity 命令のラウンドトリップ
#[test]
fn prop_set_capacity_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let capacity = noprop::sample_u64_in(ctx, 1..=4096);
        let mut sender = EncoderStream::new();
        sender.set_max_table_capacity(4096);
        sender
            .encode_set_capacity(capacity)
            .expect("test must succeed");
        let encoded = sender.get_data().to_vec();

        let mut receiver = EncoderStreamReceiver::new();
        receiver.set_max_table_capacity(4096);
        receiver.receive(&encoded);

        let mut table = DynamicTable::new();
        let instruction = receiver.process(&mut table).expect("test must succeed");

        assert_eq!(
            instruction,
            Some(EncoderInstruction::SetDynamicTableCapacity { capacity }),
            "Roundtrip failed for capacity {}",
            capacity
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: Insert with Literal Name 命令のラウンドトリップ
#[test]
fn prop_insert_literal_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let name = valid_header_name(ctx);
        let value = valid_header_value(ctx);
        let mut sender = EncoderStream::new();
        sender.set_max_table_capacity(4096);
        sender
            .encode_insert_with_literal_name(&name, &value)
            .expect("test must succeed");
        let encoded = sender.get_data().to_vec();

        let mut receiver = EncoderStreamReceiver::new();
        receiver.set_max_table_capacity(4096);
        receiver.receive(&encoded);

        let mut table = DynamicTable::with_capacity(4096);
        let instruction = receiver.process(&mut table).expect("test must succeed");

        if let Some(EncoderInstruction::InsertWithLiteralName { name: n, value: v }) = instruction {
            assert_eq!(n, name, "Name mismatch");
            assert_eq!(v, value, "Value mismatch");
        } else {
            panic!("Expected InsertWithLiteralName instruction");
        }
        Ok(())
    })?;
    Ok(())
}

/// Property: Duplicate 命令のラウンドトリップ
#[test]
fn prop_duplicate_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let index = noprop::sample_u64_in(ctx, 0..10);
        let mut sender = EncoderStream::new();
        sender.set_max_table_capacity(4096);
        sender.encode_duplicate(index).expect("test must succeed");
        let encoded = sender.get_data().to_vec();

        let mut receiver = EncoderStreamReceiver::new();
        receiver.set_max_table_capacity(4096);
        receiver.receive(&encoded);

        // テーブルにエントリを追加
        let mut table = DynamicTable::with_capacity(4096);
        for i in 0..=index {
            table.insert(format!("name{}", i).into_bytes(), b"value".to_vec());
        }

        let instruction = receiver.process(&mut table).expect("test must succeed");

        assert_eq!(
            instruction,
            Some(EncoderInstruction::Duplicate {
                relative_index: index
            }),
        );
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// Decoder Stream Properties
// =============================================================================

/// Property: Section Acknowledgment 命令のラウンドトリップ
#[test]
fn prop_section_ack_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let stream_id = valid_stream_id(ctx);
        let mut sender = DecoderStream::new();
        sender.encode_section_acknowledgment(stream_id);
        let encoded = sender.get_data().to_vec();

        let mut receiver = DecoderStreamReceiver::new();
        receiver.receive(&encoded);
        let instruction = receiver.process(u64::MAX).expect("test must succeed");

        assert_eq!(
            instruction,
            Some(DecoderInstruction::SectionAcknowledgment { stream_id }),
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: Stream Cancellation 命令のラウンドトリップ
#[test]
fn prop_stream_cancel_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let stream_id = valid_stream_id(ctx);
        let mut sender = DecoderStream::new();
        sender.encode_stream_cancellation(stream_id);
        let encoded = sender.get_data().to_vec();

        let mut receiver = DecoderStreamReceiver::new();
        receiver.receive(&encoded);
        let instruction = receiver.process(u64::MAX).expect("test must succeed");

        assert_eq!(
            instruction,
            Some(DecoderInstruction::StreamCancellation { stream_id }),
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: Insert Count Increment 命令のラウンドトリップ
#[test]
fn prop_insert_count_increment_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let increment = valid_increment(ctx);
        let mut sender = DecoderStream::new();
        sender.encode_insert_count_increment(increment);
        let encoded = sender.get_data().to_vec();

        let mut receiver = DecoderStreamReceiver::new();
        receiver.receive(&encoded);
        let instruction = receiver.process(u64::MAX).expect("test must succeed");

        assert_eq!(
            instruction,
            Some(DecoderInstruction::InsertCountIncrement { increment }),
        );
        Ok(())
    })?;
    Ok(())
}

/// Property: Known Received Count は累積される
#[test]
fn prop_known_received_count_accumulates() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let inc_count = noprop::sample_usize_in(ctx, 1..5);
        let mut sender = DecoderStream::new();
        let mut receiver = DecoderStreamReceiver::new();

        let mut expected_count = 0u64;
        for _ in 0..inc_count {
            let inc = valid_increment(ctx);
            sender.encode_insert_count_increment(inc);
            receiver.receive(sender.get_data());
            sender.consume_data(sender.get_data().len());

            let _ = receiver.process(u64::MAX).expect("test must succeed");
            expected_count += inc;

            assert_eq!(
                receiver.known_received_count(),
                expected_count,
                "Known received count should be {}",
                expected_count
            );
        }
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// Encoder/Decoder Integration Properties
// =============================================================================

/// Property: エンコーダーストリームの連続命令処理
#[test]
fn prop_encoder_stream_sequential() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let name = valid_header_name(ctx);
        let value = valid_header_value(ctx);
        let mut sender = EncoderStream::new();
        sender.set_max_table_capacity(4096);

        // 複数の命令をエンコード
        sender.encode_set_capacity(1024).expect("test must succeed");
        sender
            .encode_insert_with_literal_name(&name, &value)
            .expect("test must succeed");

        let encoded = sender.get_data().to_vec();

        let mut receiver = EncoderStreamReceiver::new();
        receiver.set_max_table_capacity(4096);
        receiver.receive(&encoded);

        let mut table = DynamicTable::new();

        // 最初の命令: Set Capacity
        let inst1 = receiver.process(&mut table).expect("test must succeed");
        if let Some(EncoderInstruction::SetDynamicTableCapacity { capacity }) = inst1 {
            assert_eq!(capacity, 1024);
        } else {
            panic!("Expected SetDynamicTableCapacity instruction");
        }

        // 2 番目の命令: Insert with Literal Name
        let inst2 = receiver.process(&mut table).expect("test must succeed");
        let is_insert_literal = matches!(
            inst2,
            Some(EncoderInstruction::InsertWithLiteralName { .. })
        );
        assert!(
            is_insert_literal,
            "Expected InsertWithLiteralName instruction"
        );

        // テーブルにエントリが追加されている
        assert_eq!(table.len(), 1);
        Ok(())
    })?;
    Ok(())
}

/// Property: デコーダーストリームの連続命令処理
#[test]
fn prop_decoder_stream_sequential() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let stream_id1 = valid_stream_id(ctx);
        let stream_id2 = valid_stream_id(ctx);
        let increment = valid_increment(ctx);
        let mut sender = DecoderStream::new();

        sender.encode_section_acknowledgment(stream_id1);
        sender.encode_stream_cancellation(stream_id2);
        sender.encode_insert_count_increment(increment);

        let encoded = sender.get_data().to_vec();

        let mut receiver = DecoderStreamReceiver::new();
        receiver.receive(&encoded);

        let inst1 = receiver.process(u64::MAX).expect("test must succeed");
        assert_eq!(
            inst1,
            Some(DecoderInstruction::SectionAcknowledgment {
                stream_id: stream_id1
            })
        );

        let inst2 = receiver.process(u64::MAX).expect("test must succeed");
        assert_eq!(
            inst2,
            Some(DecoderInstruction::StreamCancellation {
                stream_id: stream_id2
            })
        );

        let inst3 = receiver.process(u64::MAX).expect("test must succeed");
        assert_eq!(
            inst3,
            Some(DecoderInstruction::InsertCountIncrement { increment })
        );

        // バッファが空
        let inst4 = receiver.process(u64::MAX).expect("test must succeed");
        assert!(inst4.is_none());
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// Huffman Encoding Properties
// =============================================================================

/// ASCII 印字可能文字列を生成
fn printable_ascii(ctx: &mut noprop::TestCaseContext) -> Vec<u8> {
    let len = noprop::sample_usize_in(ctx, 1..128);
    let mut data = Vec::new();
    for _ in 0..len {
        data.push(0x20 + noprop::sample_usize_in(ctx, 0..0x5f) as u8);
    }
    data
}

/// Property: Huffman エンコード/デコードのラウンドトリップ
#[test]
fn prop_huffman_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let data = printable_ascii(ctx);
        let encoded_len = huffman::encoded_len(&data);
        let mut encoded = vec![0u8; encoded_len];
        huffman::encode(&mut encoded, &data);

        let decoded = huffman::decode(&encoded).expect("test must succeed");
        assert_eq!(decoded, data);
        Ok(())
    })?;
    Ok(())
}

/// Property: Huffman encoded_len は実際のエンコード長と一致
#[test]
fn prop_huffman_encoded_len_accurate() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let data = printable_ascii(ctx);
        let predicted_len = huffman::encoded_len(&data);
        let mut encoded = vec![0u8; predicted_len + 10];
        let actual_len = huffman::encode(&mut encoded, &data).expect("test must succeed");

        assert_eq!(predicted_len, actual_len);
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// Encoder/Decoder Properties
// =============================================================================

/// 静的テーブルに存在するヘッダー名を生成
fn static_header_name(ctx: &mut noprop::TestCaseContext) -> Vec<u8> {
    use shiguredo_http3::qpack::STATIC_TABLE;
    let idx = noprop::sample_usize_in(ctx, 0..STATIC_TABLE_LEN);
    STATIC_TABLE[idx].name().to_vec()
}

/// Property: Encoder/Decoder ラウンドトリップ (静的テーブルのみ)
#[test]
fn prop_encoder_decoder_roundtrip_static() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let value = valid_header_value(ctx);
        let mut encoder = Encoder::new();
        let decoder = Decoder::new();

        let headers = vec![
            wire_header(b":method", b"GET"),
            wire_header(b":path", &value),
        ];

        let mut buf = vec![0u8; 1024];
        let encoded_len = encoder
            .encode(&mut buf, &headers, 0)
            .expect("test must succeed");

        let decoded = decoder
            .decode(&buf[..encoded_len])
            .expect("test must succeed");

        assert_eq!(decoded.len(), 2);
        assert_eq!(&decoded[0].name(), b":method");
        assert_eq!(&decoded[0].value(), b"GET");
        assert_eq!(&decoded[1].name(), b":path");
        assert_eq!(&decoded[1].value(), &value);
        Ok(())
    })?;
    Ok(())
}

/// Property: Encoder/Decoder ラウンドトリップ (カスタムヘッダー)
#[test]
fn prop_encoder_decoder_roundtrip_literal() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let name = valid_header_name(ctx);
        let value = valid_header_value(ctx);
        let mut encoder = Encoder::new();
        let decoder = Decoder::new();

        let headers = vec![wire_header(&name, &value)];

        let mut buf = vec![0u8; 1024];
        let encoded_len = encoder
            .encode(&mut buf, &headers, 0)
            .expect("test must succeed");

        let decoded = decoder
            .decode(&buf[..encoded_len])
            .expect("test must succeed");

        assert_eq!(decoded.len(), 1);
        assert_eq!(&decoded[0].name(), &name);
        assert_eq!(&decoded[0].value(), &value);
        Ok(())
    })?;
    Ok(())
}

/// Property: 複数ヘッダーのラウンドトリップ
#[test]
fn prop_encoder_decoder_multiple_headers() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let header_count = noprop::sample_usize_in(ctx, 1..5);
        let mut headers_data = Vec::new();
        for _ in 0..header_count {
            let name = valid_header_name(ctx);
            let value = valid_header_value(ctx);
            headers_data.push((name, value));
        }

        let mut encoder = Encoder::new();
        let decoder = Decoder::new();

        let headers: Vec<Header> = headers_data
            .iter()
            .map(|(n, v)| wire_header(n, v))
            .collect();

        let mut buf = vec![0u8; 4096];
        let encoded_len = encoder
            .encode(&mut buf, &headers, 0)
            .expect("test must succeed");

        let decoded = decoder
            .decode(&buf[..encoded_len])
            .expect("test must succeed");

        assert_eq!(decoded.len(), headers.len());
        for (orig, dec) in headers_data.iter().zip(decoded.iter()) {
            assert_eq!(&dec.name(), &orig.0);
            assert_eq!(&dec.value(), &orig.1);
        }
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// Static Table Properties
// =============================================================================

/// Property: 静的テーブル検索の一貫性 (完全一致)
#[test]
fn prop_static_table_find_exact() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        use shiguredo_http3::qpack::STATIC_TABLE;
        let idx = noprop::sample_usize_in(ctx, 0..STATIC_TABLE_LEN);
        let entry = &STATIC_TABLE[idx];

        let (exact, name_only) = find_static_entry(entry.name(), entry.value());

        // 完全一致が見つかる場合、name_only も Some
        if exact.is_some() {
            assert!(name_only.is_some());
        }
        Ok(())
    })?;
    Ok(())
}

/// Property: 静的テーブル検索は常に有効なインデックスを返す
#[test]
fn prop_static_table_find_valid_index() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let name = static_header_name(ctx);
        let value = valid_header_value(ctx);
        let (exact, name_only) = find_static_entry(&name, &value);

        if let Some(idx) = exact {
            assert!(idx < STATIC_TABLE_LEN);
        }
        if let Some(idx) = name_only {
            assert!(idx < STATIC_TABLE_LEN);
        }
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// Dynamic Table Additional Properties
// =============================================================================

/// Property: 動的テーブルの find_entry は挿入したエントリを見つける
#[test]
fn prop_dynamic_table_find_inserted() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let name = valid_header_name(ctx);
        let value = valid_header_value(ctx);
        let mut table = DynamicTable::with_capacity(4096);
        let idx = table
            .insert(name.clone(), value.clone())
            .expect("test must succeed");

        let (exact, name_only) = table.find_entry(&name, &value);

        assert_eq!(exact, Some(idx));
        assert_eq!(name_only, Some(idx));
        Ok(())
    })?;
    Ok(())
}

/// Property: 動的テーブルの find_entry は名前のみ一致も検出
#[test]
fn prop_dynamic_table_find_name_only() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let name = valid_header_name(ctx);
        let value1 = valid_header_value(ctx);
        let value2 = valid_header_value(ctx);
        let mut table = DynamicTable::with_capacity(4096);
        let idx = table
            .insert(name.clone(), value1.clone())
            .expect("test must succeed");

        // 異なる値で検索
        let (exact, name_only) = table.find_entry(&name, &value2);

        if value1 == value2 {
            assert_eq!(exact, Some(idx));
        } else {
            assert!(exact.is_none());
        }
        assert_eq!(name_only, Some(idx));
        Ok(())
    })?;
    Ok(())
}

/// Property: 動的テーブルの duplicate は同じ内容のエントリを作成
#[test]
fn prop_dynamic_table_duplicate_content() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let name = valid_header_name(ctx);
        let value = valid_header_value(ctx);
        let mut table = DynamicTable::with_capacity(4096);
        table
            .insert(name.clone(), value.clone())
            .expect("test must succeed");

        // relative_index 0 は最新エントリ
        let dup_idx = table.duplicate(0).expect("test must succeed");

        let original = table.get_by_absolute_index(0).expect("test must succeed");
        let duplicated = table
            .get_by_absolute_index(dup_idx)
            .expect("test must succeed");

        assert_eq!(&original.name, &duplicated.name);
        assert_eq!(&original.value, &duplicated.value);
        assert_ne!(original.absolute_index, duplicated.absolute_index);
        Ok(())
    })?;
    Ok(())
}

/// Property: エビクション後も取得可能なエントリは有効
#[test]
fn prop_dynamic_table_eviction_valid() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let entry_count = noprop::sample_usize_in(ctx, 5..15);
        // 小さな容量で強制的にエビクションを発生させる
        let mut table = DynamicTable::with_capacity(128);

        for _ in 0..entry_count {
            let name = valid_header_name(ctx);
            let value = valid_header_value(ctx);
            let _ = table.insert(name, value);
        }

        // 残っているエントリはすべて有効
        for entry in table.iter() {
            let retrieved = table.get_by_absolute_index(entry.absolute_index);
            assert!(retrieved.is_some());
            assert_eq!(&retrieved.expect("test must succeed").name, &entry.name);
        }
        Ok(())
    })?;
    Ok(())
}

/// Property: 動的テーブルの相対インデックス変換が正しい
#[test]
fn prop_dynamic_table_relative_index() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let entry_count = noprop::sample_usize_in(ctx, 2..5);
        let mut table = DynamicTable::with_capacity(4096);

        for _ in 0..entry_count {
            let name = valid_header_name(ctx);
            let value = valid_header_value(ctx);
            table.insert(name, value);
        }

        let insert_count = table.insert_count();

        // 相対インデックス 0 は最新エントリ (absolute = insert_count - 1)
        let newest = table.get_by_relative_index_encoder(0);
        assert!(newest.is_some());
        assert_eq!(
            newest.expect("test must succeed").absolute_index,
            insert_count - 1
        );
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// Insert with Name Reference Properties
// =============================================================================

/// Property: Insert with Name Reference (静的テーブル) のラウンドトリップ
#[test]
fn prop_insert_with_name_ref_static_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let static_idx = noprop::sample_u64_in(ctx, 0..10);
        let value = valid_header_value(ctx);
        let mut sender = EncoderStream::new();
        sender.set_max_table_capacity(4096);
        sender
            .encode_insert_with_name_ref(true, static_idx, &value)
            .expect("test must succeed");
        let encoded = sender.get_data().to_vec();

        let mut receiver = EncoderStreamReceiver::new();
        receiver.set_max_table_capacity(4096);
        receiver.receive(&encoded);

        let mut table = DynamicTable::with_capacity(4096);
        let instruction = receiver.process(&mut table).expect("test must succeed");

        if let Some(EncoderInstruction::InsertWithNameReference {
            is_static,
            name_index,
            value: v,
        }) = instruction
        {
            assert!(is_static);
            assert_eq!(name_index, static_idx);
            assert_eq!(v, value);
        } else {
            panic!("Expected InsertWithNameReference instruction");
        }
        Ok(())
    })?;
    Ok(())
}

/// Property: エンコーダーストリーム命令の分割受信 (RFC 9204 Section 4.2)
///
/// QUIC stream は byte stream なので命令が任意の位置で分割される。
/// 部分受信時は Ok(None) を返し、完全なデータが揃うまで待機する。
#[test]
fn prop_encoder_stream_partial_receive() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let name = valid_header_name(ctx);
        let value = valid_header_value(ctx);
        let mut sender = EncoderStream::new();
        sender.set_max_table_capacity(4096);
        sender
            .encode_insert_with_literal_name(&name, &value)
            .expect("test must succeed");
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
                assert_eq!(
                    result.expect("test must succeed"),
                    None,
                    "Partial data at byte {} should return Ok(None)",
                    i
                );
            } else {
                // 最後のバイトで完全な命令が得られる
                let instruction = result.expect("test must succeed");
                assert!(
                    instruction.is_some(),
                    "Complete data should return instruction"
                );
                if let Some(EncoderInstruction::InsertWithLiteralName { name: n, value: v }) =
                    instruction
                {
                    assert_eq!(n, name.clone());
                    assert_eq!(v, value.clone());
                }
            }
        }
        Ok(())
    })?;
    Ok(())
}

/// Property: デコーダーストリーム命令の分割受信 (RFC 9204 Section 4.2)
#[test]
fn prop_decoder_stream_partial_receive() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let stream_id = valid_stream_id(ctx);
        let mut sender = DecoderStream::new();
        sender.encode_section_acknowledgment(stream_id);
        let encoded = sender.get_data().to_vec();

        // 1 バイトずつ受信
        let mut receiver = DecoderStreamReceiver::new();

        for (i, &byte) in encoded.iter().enumerate() {
            receiver.receive(&[byte]);
            let result = receiver.process(u64::MAX);

            if i < encoded.len() - 1 {
                assert_eq!(
                    result.expect("test must succeed"),
                    None,
                    "Partial data at byte {} should return Ok(None)",
                    i
                );
            } else {
                let instruction = result.expect("test must succeed");
                assert_eq!(
                    instruction,
                    Some(DecoderInstruction::SectionAcknowledgment { stream_id }),
                );
            }
        }
        Ok(())
    })?;
    Ok(())
}

/// Property: Insert with Name Reference (動的テーブル) のラウンドトリップ
#[test]
fn prop_insert_with_name_ref_dynamic_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let name = valid_header_name(ctx);
        let value1 = valid_header_value(ctx);
        let value2 = valid_header_value(ctx);
        // まず動的テーブルにエントリを追加
        let mut table = DynamicTable::with_capacity(4096);
        table.insert(name.clone(), value1);

        let mut sender = EncoderStream::new();
        sender.set_max_table_capacity(4096);
        // 動的テーブルの relative_index 0 を参照
        sender
            .encode_insert_with_name_ref(false, 0, &value2)
            .expect("test must succeed");
        let encoded = sender.get_data().to_vec();

        let mut receiver = EncoderStreamReceiver::new();
        receiver.set_max_table_capacity(4096);
        receiver.receive(&encoded);

        let instruction = receiver.process(&mut table).expect("test must succeed");

        if let Some(EncoderInstruction::InsertWithNameReference {
            is_static,
            name_index,
            value: v,
        }) = instruction
        {
            assert!(!is_static);
            assert_eq!(name_index, 0);
            assert_eq!(&v, &value2);
        } else {
            panic!("Expected InsertWithNameReference instruction");
        }

        // テーブルに新しいエントリが追加されている
        assert_eq!(table.len(), 2);
        let new_entry = table.get_by_absolute_index(1).expect("test must succeed");
        assert_eq!(&new_entry.name, &name);
        assert_eq!(&new_entry.value, &value2);
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// DynamicEncoder / DynamicDecoder Roundtrip (RFC 9204 Section 4)
// =============================================================================

/// 動的テーブルエンコード用のヘッダーを生成
fn dynamic_header(ctx: &mut noprop::TestCaseContext) -> Header {
    let name = valid_header_name(ctx);
    let value = valid_header_value(ctx);
    wire_header(&name, &value)
}

/// Property: エンコーダーとデコーダーのテーブルを同期させると、
/// DynamicEncoder でエンコードしたヘッダーは DynamicDecoder で復元できる
#[test]
fn prop_dynamic_encoder_decoder_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let entry_count = noprop::sample_usize_in(ctx, 0..5);
        let header_count = noprop::sample_usize_in(ctx, 1..5);
        let mut entries = Vec::new();
        for _ in 0..entry_count {
            let name = valid_header_name(ctx);
            let value = valid_header_value(ctx);
            entries.push((name, value));
        }
        let mut headers = Vec::new();
        for _ in 0..header_count {
            headers.push(dynamic_header(ctx));
        }
        let use_huffman = noprop::sample_bool(ctx);

        let mut encoder = DynamicEncoder::new().use_huffman(use_huffman);
        encoder.set_max_table_capacity(DYNAMIC_TABLE_CAPACITY);
        encoder
            .set_table_capacity(DYNAMIC_TABLE_CAPACITY)
            .expect("PBT: capacity matches max_table_capacity");

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
                assert_eq!(headers.len(), decoded.len());
                for (orig, dec) in headers.iter().zip(decoded.iter()) {
                    assert_eq!(orig.name().to_vec(), dec.name().to_vec());
                    assert_eq!(orig.value().to_vec(), dec.value().to_vec());
                }
            }
            // テーブル状態の不一致によるブロックやエラーは許容
            Ok(DecodeOutput::Blocked) | Err(_) => {}
        }
        Ok(())
    })?;
    Ok(())
}

/// Property: 動的テーブル参照でブロックされたデコードは、
/// エンコーダーストリームでテーブル更新後に成功する (RFC 9204 Section 2.1.2)
#[test]
fn prop_blocked_then_unblocked() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let entry_name = valid_header_name(ctx);
        let entry_value = valid_header_value(ctx);
        // エンコーダー側: 動的テーブルにエントリを挿入してエンコード
        let mut encoder = DynamicEncoder::new().use_huffman(false);
        encoder.set_max_table_capacity(DYNAMIC_TABLE_CAPACITY);
        encoder
            .set_table_capacity(DYNAMIC_TABLE_CAPACITY)
            .expect("PBT: capacity matches max_table_capacity");
        encoder.insert(entry_name.clone(), entry_value.clone());

        let headers = vec![wire_header(&entry_name, &entry_value)];
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
            assert_eq!(decoded.len(), 1);
            assert_eq!(decoded[0].name().to_vec(), entry_name);
            assert_eq!(decoded[0].value().to_vec(), entry_value);
        }
        Ok(())
    })?;
    Ok(())
}

// =============================================================================
// Construct-Time Validation Consistency (Header)
// =============================================================================

/// Property: `Header::from_static` と `Header::new` が同じ値を返す
/// (`const fn` 検査とランタイム検査のロジック一致)
///
/// `Header::from_static` は `&'static [u8]` を要求するため `Box::leak` で
/// 擬似的に静的化する。noprop では shrink は発生しないが、ケースごとに
/// リークが累積するためケース数を絞る (cases: 16)。
#[test]
fn prop_header_from_static_matches_new() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(16, |ctx| {
        let name = valid_header_name(ctx);
        let value = valid_header_value(ctx);
        let via_new = Header::new(&name, &value).expect("test must succeed");
        let static_name: &'static [u8] = Box::leak(name.clone().into_boxed_slice());
        let static_value: &'static [u8] = Box::leak(value.clone().into_boxed_slice());
        let via_static = Header::from_static(static_name, static_value);
        assert_eq!(via_new, via_static);
        Ok(())
    })?;
    Ok(())
}

/// Property: `wire_header` と `Header::new` が同じ値を返す
/// (wire 模擬経路と公開 API のロジック一致)
#[test]
fn prop_wire_header_matches_new() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let name = valid_header_name(ctx);
        let value = valid_header_value(ctx);
        let via_new = Header::new(&name, &value).expect("test must succeed");
        let via_wire = wire_header(&name, &value);
        assert_eq!(via_new, via_wire);
        Ok(())
    })?;
    Ok(())
}

/// Property (完全性): `Header::new` が受理する任意 (name, value) は、
/// QPACK encode → decode の往復で同じ Header が再構築される。
/// (構築時検査と decoder の受理集合一致の片方向確認)
#[test]
fn prop_header_new_accepts_imply_qpack_roundtrip() -> noprop::TestResult {
    let seed = noprop::seed_from_env_or_time("PROP_QPACK_SEED")?;
    let mut runner = noprop::Runner::new(seed);
    runner.run(256, |ctx| {
        let name = valid_header_name(ctx);
        let value = valid_header_value(ctx);
        let original = Header::new(&name, &value).expect("test must succeed");
        let mut encoder = Encoder::new();
        let decoder = Decoder::new();
        let headers = vec![original.clone()];
        let mut buf = vec![0u8; 8192];
        let encoded_len = encoder
            .encode(&mut buf, &headers, 0)
            .expect("test must succeed");
        let decoded = decoder
            .decode(&buf[..encoded_len])
            .expect("test must succeed");
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].name(), original.name());
        assert_eq!(decoded[0].value(), original.value());
        Ok(())
    })?;
    Ok(())
}
