# rakukan-ni — 楽観改

> ⚠️ **注意：現在テスト動作中です**
>
> インストールによって **Windows の動作が不安定になる可能性があります**。
> TSF（Text Services Framework）DLL をシステムに登録するため、インストール・アンインストールの操作は
> **自己責任** で行ってください。重要な作業環境への適用は推奨しません。

[rakukan](https://github.com/fukuyori/rakukan) の有志による改変版です。
中核エンジンは [karukan](https://github.com/togatoga/karukan)、TSF 層の実装は [azooKey-Windows](https://github.com/fkunn1326/azooKey-Windows) を参考にしています。

オリジナルの rakukan に対し、いくつかのバグ修正を施したつもりですが、まだまだ作業中で安定動作は望めません。
逐次修正は行いますが、オリジナルに追従できるかはわかりません。今後の取り組みも気まぐれです。

---

Windows 向け日本語 IME。ローカルで動く小型 LLM と Mozc 系辞書を組み合わせ、従来のかな漢字変換とは少し違う候補の出し方を試すための実験的な IME です。入力中の読みから候補を先読みするライブ変換、数字やアルファベットを壊さない literal 保護、ユーザー辞書・学習履歴による候補の優先順位調整を中心にしています。

設計上の大きな特徴は、TSF DLL と変換エンジンを別プロセスに分けていることです。Windows の入力フレームワーク側には軽いクライアントだけを置き、LLM や GPU バックエンドは `rakukan-engine-host.exe` 側で管理します。これにより、CPU / Vulkan / CUDA の engine DLL を設定で切り替えながら、IME 側の安定性をできるだけ保つ構成にしています。

## 主な機能

- **ライブ変換**: ひらがな入力後、短い停止でトップ候補を自動表示
- **範囲指定変換**: `Shift+Right/Left` で先頭から変換範囲を指定 → `Space` で変換 → `Enter` で確定、残りで LiveConv 再開
- **区読点分割変換**: `、` `。` や全角記号 `（）～` 、和文記号 `「」・` など記号を含む読みを入力すると記号位置でブロックへ自動分割。`Space` で各ブロックを変換 → `Enter` でブロックを順番に確定。候補ウィンドウは確定のたびに次のブロック直下へ追従
- **数値保護**: LLM が数字を改変しない（`2024ねん → 2024年`）。数字・アルファベットは半角/全角の両方を候補として提示
- **LLM + 辞書変換**: jinen モデルと Mozc 系辞書を併用
- **ユーザー辞書学習**: 確定した変換結果を即時反映
- **文字種変換**: `F6`〜`F10` でひらがな・カタカナ・英数を往復
- **GPU アクセラレーション**: CUDA / Vulkan バックエンド対応
- **out-of-process 構成**: TSF DLL と engine-host を分離し、GPU リソースや LLM 実行をホストプロセス側で管理

## インストール

```powershell
# 初回: esaxx-rs パッチのセットアップ
cargo fetch
.\scripts\setup-esaxx-patch.ps1

# ① engine DLL をビルド (cpu/vulkan/cuda)
cargo make build-engine

# ② tsf + tray + host + dict-builder + WinUI settings をビルド
cargo make build-tsf

# ③ 電子署名 (任意; 配布用)
cargo make sign

# ④ %LOCALAPPDATA%\rakukan\ にコピー + TSF 登録 + tray 起動 (★管理者権限)
cargo make install
```

まとめ実行:

```powershell
cargo make full-install   # ①〜④ を一括 (リリース向け)
cargo make quick-install  # 開発時の高速再インストール (engine 使いまわし、署名なし)
```

インストール先: `%LOCALAPPDATA%\rakukan\`  
設定: `%APPDATA%\rakukan\config.toml`  
ログ:

- TSF 側: `%LOCALAPPDATA%\rakukan\rakukan.log`
- エンジンホスト側: `%LOCALAPPDATA%\rakukan\rakukan-engine-host.log`

## 設定の目安

`%APPDATA%\rakukan\config.toml` では `model_variant` と `n_gpu_layers` を調整できます。

- `jinen-v1-xsmall-q5` は比較的軽く、`n_gpu_layers = 16` 前後から試しやすい
- `jinen-v1-small-q5` は `n_gpu_layers = 8` か `16` くらいから始めるのが安全
- `n_gpu_layers = 0` は CPU のみ
- 未指定は全レイヤー GPU オフロード

## キー操作

| キー | 動作 |
| ---- | ---- |
| Space / 変換 | 変換開始 / 次候補 / 選択中分節の再変換 |
| Enter | 表示中の内容を確定（区読点分割変換中はブロックを順番に確定） |
| ESC | 変換キャンセル |
| Backspace | 1文字削除 |
| Left / Right | 分節選択の移動 |
| Shift+Left / Shift+Right | 分節選択の縮小 / 拡張 |
| ↑ / ↓ | 候補を前後に移動 |
| 1〜9 | 候補を番号で選択 |
| Tab / PageDown | 次ページ |
| Shift+Tab / PageUp | 前ページ |
| F6 | ひらがな |
| F7 | カタカナ |
| F8 | 半角カタカナ |
| F9 | 全角英数 |
| F10 | 半角英数 |

## ライセンス

rakukan-ni 本体のコードは **MIT ライセンス** です。
辞書・モデルなどの同梱物や取得物には、それぞれ個別のライセンス条件が適用されます。

This project is a fork of [rakukan](https://github.com/fukuyori/rakukan) by fukuyori, which in turn uses the LLM-based conversion engine from [karukan](https://github.com/togatoga/karukan) by togatoga (MIT/Apache-2.0) and references the TSF implementation of [azooKey-Windows](https://github.com/fkunn1326/azooKey-Windows) by fkunn1326.
