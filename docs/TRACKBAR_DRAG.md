# トラックバーへのドラッグ&ドロップ適用

> 使い方と、リバースエンジニアリングに詳しくない方向けの更新手順は
> [`DRAG_AND_SEGMENTS.md`](DRAG_AND_SEGMENTS.md) にまとめています。
> このファイルは実装寄りの技術メモです。

HL_Ezing の「適用」ボタンは、クリックすると従来どおり適用先ピッカーを開きます。
ボタンを 6px 以上ドラッグすると、AviUtl2 のオブジェクト設定にあるトラックバーへ
直接ドロップできます。

この機能は AviUtl2 の非公開 UI を実行時に解析しているため、AviUtl2 の
バージョンアップで動かなくなる可能性があります。以下は、壊れたときの調査と
更新のためのメモです。

## 仕組み

1. フロントエンドが「適用」ボタンのドラッグ開始を検出し、`start_drag` IPC を送る。
2. Rust 側のワーカースレッドが左ボタンの解放を監視する。
3. 解放位置のルートウィンドウを取得し、そのウィンドウのクライアント座標を
   基準にトラックバー矩形をスクリーン座標へ変換する。
4. 自プロセスの書き込み可能なプライベートメモリを走査し、
   `Component::EffectItemNumberControl` の vtable を持つオブジェクトを探す。
5. `EffectItemNumberControl` 内に埋め込まれた `Control::SliderInputControl`
   を vtable 走査で見つけ、その矩形・アイテム名・`EffectSetting` を読む。
   スライダーの埋め込みオフセットはバージョンによって変わるため、
   固定値ではなくオブジェクト内の vtable ポインタから動的に求めている。
6. `get_tracks` 相当の一覧と照合して `effect_name` / `effect_index` / `item_name`
   を決め、既存の `apply_to_item` でカーブを適用する。
7. ドラッグ中は、対象の `SliderInputControl` の矩形を
   クリック透過の枠ウィンドウ（`HighlightOverlay`）でハイライトする。

スキャン結果は短時間キャッシュする。最初のスキャンだけ時間がかかるが、
同じオブジェクト設定を開いている間の2回目以降はキャッシュを使う。
`prepare_drag` IPC でポインタ押下時からスキャンを温める。

vtable は絶対アドレスを埋め込まず、PE の RTTI からクラス名で解決します。
そのため、ASLR や vtable のアドレス変更には自動で追従します。
バージョン依存なのは `src/trackbar.rs` の `LayoutProfile` に集約した
オフセットだけです。

## 診断

- AviUtl2 のメニュー: `HL_Ezing` → `ドラッグ適用の診断情報をログ出力`
- HL_Ezing の設定画面: 「ドラッグ診断をログ出力」
- IPC: `{ "cmd": "debug_trackbars" }`

出力例:

```json
{
  "version": 2011400,
  "profile": "AviUtl2 2.1.x",
  "module_base": "0x7FF6...",
  "vtables": {
    "effect_item_control": "0x7FF6...",
    "trackbar": "0x7FF6...",
    "effect_setting": "0x7FF6..."
  },
  "host": "0x...",
  "root": "0x...",
  "targets": [
    {
      "control": "0x...",
      "trackbar": "0x...",
      "context": "0x...",
      "context_vtable": "0x...",
      "rect": { "x": 100, "y": 200, "w": 300, "h": 20 },
      "visible": true,
      "item": "X",
      "label": "X",
      "effect": "座標",
      "effect_rank": 0
    }
  ]
}
```

オブジェクト設定を開いた状態で実行してください。`targets` が空の場合は、
オブジェクトが選択されていないか、設定画面が閉じています。

## バージョンアップ時の確認順序

1. まず診断ログを取得する。
2. `vtables` が `null` または解決失敗になっている場合
   - クラス名が変わっていないか確認する。
   - `CLASS_EFFECT_ITEM_CONTROL` / `CLASS_SLIDER_INPUT` を更新する。
   - RTTI の型名は `.?AV<クラス名>` で `.rdata` に格納されている。
3. `vtables` は解決できるが `targets` が空の場合
   - `LayoutProfile` のオフセットを確認する。
   - 特に `item_control_item`、`item_control_context`、`slider_rect` を確認する。
4. `targets` は出るがドロップ位置がずれる場合
   - スキャン結果の `rect`（メインウィンドウのクライアント座標）と、
     実際のカーソル位置を比較する。
   - 判定時はドロップ先ルートウィンドウから `ClientToScreen` で変換する。
     ずれる場合は `slider_rect` の x/y を確認する。
   - プラグインウィンドウが分離（フローティング）されていても、
     ドロップ先が AviUtl2 メインウィンドウなら判定できる。
5. `targets` は出るが適用先が違う場合
   - `effect` が空なら `setting_effect_base` / `effect_base_name` を確認する。
   - 同名エフェクトの順序は `effect_rank` と `get_tracks` の順序で照合している。

## 現在のプロファイル

`LayoutProfile` (`src/trackbar.rs`):

| フィールド | 2.1.14 / 2.1.15 |
| --- | ---: |
| `item_control_item` | `0x278` |
| `item_control_size` | `0x1B48` |
| `item_control_context` | `0x0E8` |
| `setting_effect_base` | `0x0F8` |
| `effect_base_name` | `0x40` |
| `item_name_wstring` | `0x10` |
| `slider_visible` | `0x0C` |
| `slider_rect` | `0x50` |

`SliderInputControl` 自体の位置は動的に検出する。参考値:

- 2.1.14 / 2.1.15: `+0x568`, `+0x1058`
- 2.1.3a: `+0x540`, `+0x1000`

将来のバージョンでアイテム名・コンテキストのオフセットが変わった場合は、
`LayoutProfile` を追加し、`layout_profile(version)` のバージョン範囲を更新します。

## 既知の制限

- AviUtl2 が Direct2D で描画する設定画面の内部構造に依存している。
- オブジェクト設定が開かれていて、対象トラックバーが表示されている必要がある。
- 管理者権限や外部プロセスへの注入は不要。すべて自プロセス内の読み取りで完結する。
- 解決に失敗した場合はドラッグ適用だけを無効化し、クリック適用・右クリックメニューは
  そのまま利用できる。
