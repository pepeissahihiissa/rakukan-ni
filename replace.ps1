# 1. 強制停止（ctfmon と TextInputHost は IME 関連のホストプロセス）
Stop-Process -Name "rakukan-tray" -Force -ErrorAction SilentlyContinue
Stop-Process -Name "rakukan-engine-host" -Force -ErrorAction SilentlyContinue
Stop-Process -Name "ctfmon" -Force -ErrorAction SilentlyContinue
Stop-Process -Name "TextInputHost" -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 1500

# 2. 旧 DLL を登録解除
regsvr32 /s /u "$env:LOCALAPPDATA\rakukan\rakukan_tsf.dll"

# 3. コピー
Copy-Item -LiteralPath "C:\Users\a\Documents\src\rakukan-main\target\release\rakukan_tsf.dll" `
          -Destination "$env:LOCALAPPDATA\rakukan\rakukan_tsf.dll" -Force

# 4. 新 DLL を登録
regsvr32 /s "$env:LOCALAPPDATA\rakukan\rakukan_tsf.dll"

# 5. トレイ起動
Start-Process "$env:LOCALAPPDATA\rakukan\rakukan-tray.exe"