# SETTINGS フレームに GREASE (予約設定) を追加する

- Priority: Low
- Created: 2026-06-07
- Model: DeepSeek v4-pro
- Branch: feature/add-grease-settings
- Polished: 2026-06-07

## 目的

RFC 9114 Section 7.2.4.1 で推奨されている GREASE (予約設定) を SETTINGS フレームに含めることで、HTTP/3 実装の相互運用性を高める。

## 優先度根拠

RFC 9114 では SHOULD レベルの推奨であり、MUST ではない。GREASE が無くても既存実装との相互運用に支障はないが、プロトコルとして将来の拡張ポイントの堅牢性を高めるために追加すべき。

## 現状

`SettingsPayload::from_settings()` が生成する SETTINGS フレームには、アプリケーションが明示的に設定したパラメータ (QPACK 最大テーブル容量、最大フィールドセクションサイズ等) のみが含まれ、RFC 9114 Section 7.2.4.1 で SHOULD とされる予約設定 (GREASE: `0x1f * N + 0x21`) を 1 つも含めていない。

## 設計方針

- `SettingsPayload::from_settings()` で、予約設定 (GREASE) を SETTINGS フレームに最低 1 つ含める
- GREASE 値は `0x1f * N + 0x21` の形式で、`N` はランダムに選択する（実装依存）
- 値は任意でよいが、既存実装との互換性を考慮し VarInt 範囲内の小さな値を使用する
- 1 つ以上の予約設定を含めればよいので、とりあえず 1 つで十分

## 完了条件

- `SettingsPayload::from_settings()` が生成する SETTINGS フレームに GREASE 設定が最低 1 つ含まれる
- PBT (`pbt/tests/prop_settings.rs`) で GREASE 設定の存在またはラウンドトリップを検証する
- 既存のテストが全て通ること

## 解決方法

`SettingsPayload::from_settings()` 内で、アプリケーション設定の追加後に GREASE 設定を 1 つ追加する。GREASE 値は固定的な `0x21` (N=0) で十分だが、接続ごとに異なる値を選ぶ方がより効果的。
