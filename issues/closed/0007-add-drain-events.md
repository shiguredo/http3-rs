# Connection::drain_events() を追加する

Created: 2026-03-29
Completed: 2026-03-29
Model: Opus 4.6

## 概要

`Connection::drain_events()` メソッドを追加し、イベントキューの全イベントを一括取得できるようにする。

## 根拠

moqt-rust-private の publisher / subscriber / relay の全てで、`poll_event()` を while ループで呼んで `Vec<Event>` に集めるラッパーを書いている:

```rust
fn poll_all_events(&mut self) -> Vec<Event> {
    let mut events = Vec::new();
    while let Some(ev) = self.h3_conn.poll_event() {
        events.push(ev);
    }
    events
}
```

このパターンは `feed_stream()` 後のイベント処理だけでなく、他のトリガー (Notify 経由等) でのイベント回収にも使われている。`poll_event()` を 1 つずつ処理するケースと、全回収するケースの両方が存在する。

`drain_events` は Rust 標準の `Vec::drain()` と同じ意味で「キューから全要素を取り出す」操作を表し、名前が明確。

## 対応方針

- `Connection::drain_events(&mut self) -> Vec<Event>` を追加する
  - 内部のイベントキュー (`VecDeque<Event>`) から全イベントを取り出して返す
  - キューが空なら空の Vec を返す
- `ClientConnection::drain_events()` と `ServerConnection::drain_events()` にも委譲メソッドを追加する
- 既存の `poll_event()` は維持する (1 イベントずつ処理したいケース向け)
- Sans I/O の範疇内 (内部キュー操作のみ、I/O なし)

## 解決方法

`Connection::drain_events()`, `ClientConnection::drain_events()`, `ServerConnection::drain_events()` を追加した。

- イベントキュー (`VecDeque<Event>`) から全イベントを `Vec<Event>` として返す
- キューが空なら空の `Vec` を返す
- 既存の `poll_event()` は維持 (1 イベントずつ処理したいケース向け)
