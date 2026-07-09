# AGENTS.md

このファイルは Codex がこのプロジェクトで作業するときに毎回読み込むメモ。

## ビルド / インストール（決定版）

```sh
# 1. エンジンDLLをビルド（variant は config.toml の gpu_backend に依存）
cargo make build-engine

# 2. TSF 側をビルド
cargo make build-tsf

# 3. インストール（DLL を %LOCALAPPDATA%\rakukan にコピー）
sudo cargo make install
```

### ビルド後の確認

```sh
# どのバックエンド DLL が使われるか確認（config.toml の gpu_backend）
Get-Content "$env:APPDATA\rakukan\config.toml" | Select-String "gpu_backend"

# DLL が正しく配置されたか確認
Get-ChildItem "$env:LOCALAPPDATA\rakukan\rakukan_engine_*.dll"
```

### よくある失敗と対策

| 症状 | 原因 | 対策 |
|------|------|------|
| 変更が反映されない | 違う variant の DLL が読まれている | `gpu_backend` を確認。`"auto"` → `"cpu"` に固定すると確実 |
| ビルドが通らない | MSVC/CUDA ツールチェーン不一致 | `cargo make check` で型チェックだけ先に通す |
| 変換がフリーズする | ワーカースレッドが panic | `engine-host` のログを確認（`%APPDATA%\rakukan\logs\`） |
| `cargo make install` で DLL ロックエラー | 他プロセスが TSF DLL を読み込み中 | 下記「TSF DLL の手動差し替え」を参照 |

### TSF DLL の手動差し替え（ロック回避）

`cargo make install` が DLL ロックで失敗した場合。**既存プロセスは旧 DLL を使い続け、新規プロセスに新 DLL が適用される。**

```powershell
# 1. 旧DLLのCOM登録解除
regsvr32 /s /u "$env:LOCALAPPDATA\rakukan\rakukan_tsf.dll"

# 2. リネーム（ロック中でも可能）
Rename-Item "$env:LOCALAPPDATA\rakukan\rakukan_tsf.dll" "$env:LOCALAPPDATA\rakukan\rakukan_tsf.dll.old"

# 3. 新DLLコピー
Copy-Item target\release\rakukan_tsf.dll "$env:LOCALAPPDATA\rakukan\rakukan_tsf.dll" -Force

# 4. 新DLL登録
regsvr32 /s "$env:LOCALAPPDATA\rakukan\rakukan_tsf.dll"
```

**ロックが解除できない場合の最終手段**: `explorer.exe` が DLL を保持しているためコピーに失敗する。以下の手順で回避する。

```powershell
# 1. TSF のアンロード
regsvr32 /s /u "$env:LOCALAPPDATA\rakukan\rakukan_tsf.dll"

# 2. explorer を強制終了（タスクバーが消えるが自動復帰する）
Stop-Process -Name explorer -Force

# 3. PowerShell ウィンドウが残っていればここで DLL を差し替え
Move-Item "$env:LOCALAPPDATA\rakukan\rakukan_tsf.dll" "$env:LOCALAPPDATA\rakukan\rakukan_tsf.dll.old" -Force
Copy-Item target\release\rakukan_tsf.dll "$env:LOCALAPPDATA\rakukan\rakukan_tsf.dll" -Force
regsvr32 /s "$env:LOCALAPPDATA\rakukan\rakukan_tsf.dll"

# 4. explorer 再起動（タスクマネージャーを開いて「ファイル→新しいタスクの実行→explorer」）
Start-Process explorer
```

### 補助コマンド

- `cargo make build-engine-full` — エンジンをフルクリーンビルド（llama CUDA キャッシュも削除、低速）
- `cargo make uninstall` — アンインストール
- `cargo make check` — 型チェックのみ（高速）
- `cargo make test` — エンジン単体テスト
- `cargo test --workspace --lib` — workspace 全体のテスト
- `cargo test -p rakukan-dict --lib` — rakukan-dict のテストのみ

### 評価モード（モデル直接テスト）

```sh
# 読みを直接モデルに渡して変換結果を確認（IME不要）
cargo run -p rakukan-engine-cli -- --eval --reading パソコンパソコンパソコン

# 文脈あり
cargo run -p rakukan-engine-cli -- --eval --reading パソコン --context "前に変換した文章"

# プロンプトに指示を追加
cargo run -p rakukan-engine-cli -- --eval --reading パソコンパソコン --instr "長さを保って変換:"

# 生のbeam候補も表示（--raw = RUST_LOG=debug）
cargo run -p rakukan-engine-cli -- --eval --reading パソコン --raw

# モデル指定
cargo run -p rakukan-engine-cli -- --model jinen-v1-small-q5 --eval --reading パソコン
```

### エンジン変更時の確実な更新手順

エンジン（`rakukan-engine` / `rakukan-engine-abi`）の変更は、以下の手順で確実に反映する。

```powershell
# ── 準備 ──

# 1. エンジン DLL をビルド（CPU variant が最速）
cargo build -p rakukan-engine --release

# 2. engine-host をビルド（engine-abi 変更時は必須）
cargo build -p rakukan-engine-host --release

# 3. TSF DLL をビルド
cargo build -p rakukan-tsf --release

# 4. 全バイナリをインストール先にコピー
#    ※ engine DLL のファイル名規則: rakukan_engine_{cpu,vulkan,cuda}.dll
Copy-Item target/release/rakukan-engine-host.exe "$env:LOCALAPPDATA\rakukan\rakukan-engine-host.exe" -Force
Copy-Item target/release/rakukan_engine.dll "$env:LOCALAPPDATA\rakukan\rakukan_engine_cpu.dll" -Force

# ── エンジンの再起動（重要） ──

# 5. 実行中の engine-host を強制停止（TSF が自動再起動する）
Get-Process -Name "rakukan-engine-host" -ErrorAction SilentlyContinue | Stop-Process -Force

# 6. config の gpu_backend を確認（cpu 固定推奨）
Get-Content "$env:APPDATA\rakukan\config.toml" | Select-String "gpu_backend"

# 7. engine-host が起動するまで待機（TSF 経由で起動）
#    この時点で Notepad 等でキー入力すると engine-host が起動する
#    → トグル入力で / を入力し、Space で変換を試す

# ── 確認 ──

# 8. ログで backned とバージョンを確認
Get-Content "$env:APPDATA\rakukan\logs\rakukan_tsf.log" | Select-String "engine connected via RPC|deserialize via"
#    「backend=CPU」＋バージョンが最新であることを確認

# 9. （参考）DLL のタイムスタンプ確認
Get-ChildItem "$env:LOCALAPPDATA\rakukan\rakukan_engine_cpu.dll" | Select-Object Name, Length, LastWriteTime
```

### エンジン DLL のみ単体でビルドする場合

CUDA variant のフルビルドは非常に時間がかかる（~20分以上）。通常は CPU variant のみビルドして `gpu_backend = "cpu"` でテストする。

```powershell
# CPU variant（最速、~1-2分）
cargo build -p rakukan-engine --release

# CUDA variant が必要な場合は cargo make 経由（C:\rb のビルドディレクトリ使用）
cargo make build-engine
# ※ タイムアウトに注意。CUDA ビルドは 30 分超える場合あり
```

## プロジェクト構成の注意点

- **out-of-process 化済み**: TSF DLL ↔ engine-host (RPC)。GPU リソースは engine-host が管理し、TSF 側は触らない。engine-host が複数起動していても GPU メモリは変換時のみ確保される。
- **エンジン DLL は 3 variant (`cpu` / `vulkan` / `cuda`)**: `config.toml` の `gpu_backend` で選択。`"auto"` だと順に検出。**CPU 固定が最も安定。**
- **ユーザー辞書は WinUI 設定が直接管理**: `user_dict.toml` は手動登録専用。engine は読み取りのみ。
- **学習履歴は別ファイル**: `%APPDATA%\rakukan\learn_history.bin` (bincode)。user_dict とは分離。
- **設定ファイル**:
  - `%APPDATA%\rakukan\config.toml` — 一般設定
  - `%APPDATA%\rakukan\keymap.toml` — キーバインド
  - `%APPDATA%\rakukan\user_dict.toml` — ユーザー辞書
  - `%APPDATA%\rakukan\learn_history.bin` — 学習履歴
  - `%LOCALAPPDATA%\rakukan\dict\rakukan.dict` — MOZC バイナリ辞書
