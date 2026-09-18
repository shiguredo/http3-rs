.PHONY: test doc-test cover pbt pbt-cover fuzz fuzzing fuzzing-list interop-test-h3 interop-test-wt interop-test-browser interop-test check clippy fmt clean

# 全テストを実行する
test:
	cargo test --workspace --tests

# doctest を実行する (compile_fail ブロックを含む)
doc-test:
	cargo test --doc --workspace

# 全テストカバレッジ付きで実行する
cover:
	cargo llvm-cov --tests --workspace

# PBT をカバレッジ付きで実行する
pbt-with-cover:
	cargo llvm-cov -p pbt --tests

# Fuzzing を全ターゲットで 30 秒ずつ実行する
fuzzing:
	@for target in $$(cargo fuzz list); do \
		echo "=== Fuzzing $$target ==="; \
		cargo +nightly fuzz run $$target -- -max_total_time=30 || exit 1; \
	done

# Fuzzing ターゲット一覧を表示する
fuzzing-list:
	cargo fuzz list

# interop テスト (H3) を実行する
interop-test-h3:
	cd interop/h3 && cargo test

# interop テスト (WebTransport) を実行する
interop-test-wt:
	cd interop/wt && cargo test

# ブラウザ (Chromium / WebKit) との WebTransport 相互運用テストを実行する
#
# npm ci とブラウザのダウンロードに時間がかかるため interop-test には含めない。
# CI から個別に実行する。
interop-test-browser:
	cargo build --manifest-path examples/wt_server/Cargo.toml
	cd interop/browser && npm ci && npx playwright install chromium webkit && node run.mjs

# interop テストを全て実行する (ブラウザは含めない)
interop-test: interop-test-h3 interop-test-wt

# cargo check を実行する
check:
	cargo check --workspace

# cargo clippy を実行する
clippy:
	cargo clippy --workspace -- -D warnings

# cargo fmt を実行する
fmt:
	cargo fmt --all

# ビルド成果物を削除する
clean:
	cargo clean
