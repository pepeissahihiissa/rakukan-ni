# 1. rakukan 関連プロセス停止
Stop-Process -Name "rakukan-tray" -Force -ErrorAction SilentlyContinue
Stop-Process -Name "rakukan-engine-host" -Force -ErrorAction SilentlyContinue

# 2. ロック中の DLL をリネーム（ロック中でもリネームは可能）
Rename-Item "$env:LOCALAPPDATA\rakukan\rakukan_tsf.dll" `
           "$env:LOCALAPPDATA\rakukan\rakukan_tsf.dll.bak" -Force

# 3. 新しい DLL をコピー（パスが空いたので成功する）
Copy-Item "C:\Users\a\Documents\src\rakukan-main\target\release\rakukan_tsf.dll" `
          "$env:LOCALAPPDATA\rakukan\rakukan_tsf.dll"

# 4. 新しい DLL を登録（既存の COM エントリを上書き）
regsvr32 /s "$env:LOCALAPPDATA\rakukan\rakukan_tsf.dll"

# 5. バックアップ削除（次回起動時に自動解放される）
Remove-Item "$env:LOCALAPPDATA\rakukan\rakukan_tsf.dll.bak" -Force -ErrorAction SilentlyContinue