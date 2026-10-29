# IronSetup v0.1.0

**以 Rust 獨立實作、參考 Inno Setup 工作方式的 Windows 安裝工具。**

這份便攜包包含 Windows x64 編譯器、原生編譯工作室、安裝精靈 stub、可直接執行的示範安裝 EXE、Linux x64 CLI、完整 Rust 原始碼、測試、範例與驗證紀錄。

定位是可運作的第一版相容子集。它能把支援的 `.iss` 腳本編譯成自己的單一 EXE 安裝程式；**不能宣稱完整取代 Inno Setup 7.1.0，也不能解包／執行既有 Inno Setup EXE 或 Pascal `[Code]`。** 未支援的 section、directive、parameter、flag 會報錯，而不是假裝完成。

## 先使用：不需要安裝 Rust

1. 將整個 ZIP 解壓縮至可寫入的目錄，例如 `D:\Tools\IronSetup`。
2. 雙擊 **`Start-IronSetup.cmd`**，開啟原生 Windows 編譯工作室與完整示範腳本。
3. 按 **Build / F9**。編譯結果與檔案路徑會出現在下方記錄區。
4. 雙擊產生的 EXE，依序完成歡迎、資訊、授權、目的地、元件／工作、確認、進度、完成頁。

也可以直接雙擊 **`Build-Demo.cmd`**，產生 `samples\IronSetup-Demo-Setup.exe`。包內已附一份預先建置的示範安裝程式。

**編譯工作室與精靈目前是英文介面**；腳本、檔案內容與路徑支援 UTF-8／Unicode，本文件使用繁體中文。Windows 預編譯檔的目標為 Windows 10／11 x64；驗證平台的實際狀態請看 `evidence/VERIFICATION.md`，Wine 執行不等於原生 Windows 驗收。

## 本版已實作

| 能力 | 行為 |
| --- | --- |
| ISS 編譯 | 支援 9 個常用 section；錯誤附來源檔案／行號 |
| 基本預處理 | `#define`、`{#NAME}`、`#include`、命令列定義覆寫；防循環及大小限制 |
| 檔案收集 | `*`／`?`、遞迴子目錄、排除規則、重新命名、空目錄 |
| 單一 EXE | Rust stub + 自有格式封裝 + SHA-256 trailer；目標電腦不用 Rust |
| 壓縮與完整性 | deflate 或不壓縮；封裝、每一份 payload 與 SFX trailer 驗證 |
| 原生介面 | Windows 原生控制項、檔案對話框、文字編輯器、背景編譯／安裝工作 |
| 安裝選項 | 安裝位置、類型、元件、固定元件、工作與布林條件 |
| Windows 整合 | 原生 COM `.lnk`、Registry、Installed Apps／ARP、獨立解除安裝程式 |
| 還原能力 | 寫入前記錄 journal、原始檔備份、失敗回復、`recover` 中斷交易 |
| 保留使用者資料 | 解除安裝保留已修改的檔案與不屬於安裝程式的檔案 |
| 命令列 | 編譯、檢查、預處理、inspect、安裝、verify、解除安裝、recover |

精確支援範圍在 `docs/COMPATIBILITY.md` 與 `crates/compiler/README.md`。

## 最小腳本

```ini
[Setup]
AppId=MyCompany.MyApp
AppName=My App
AppVersion=1.0.0
DefaultDirName={localappdata}\Programs\My App
PrivilegesRequired=lowest
Compression=zip
OutputBaseFilename=My-App-Setup

[Files]
Source: "dist\MyApp.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "dist\settings.toml"; DestDir: "{app}"; Flags: onlyifdoesntexist

[Icons]
Name: "{group}\My App"; Filename: "{app}\MyApp.exe"
```

把腳本放在專案資料夾；相對 Source 路徑以腳本所在目錄／`SourceDir` 為基準。

```powershell
.\bin\windows-x64\ironsetup.exe compile .\my-app.iss `
  --stub .\bin\windows-x64\ironsetup-stub.exe `
  --output .\My-App-Setup.exe
```

`ironsetup.exe` 會優先尋找同目錄的 `ironsetup-stub.exe`，因此平常可以省略 `--stub`。

### 必須明確指定覆寫政策

本版尚未實作 Inno 的 PE 檔案版本比較。每個 `[Files]` 項目必須有以下其中一個 flag：

- `ignoreversion`：明確允許以封裝內的內容取代目標；安裝前備份原始檔。
- `onlyifdoesntexist`：目標已存在就略過，適合設定檔。

未指定時編譯失敗。這可避免既有腳本被默默改成「一律覆寫」。

## 常用命令

```powershell
# 只檢查，不產生安裝檔
.\bin\windows-x64\ironsetup.exe check .\examples\windows-demo.iss

# 產生自有 .ispkg 封裝
.\bin\windows-x64\ironsetup.exe compile .\examples\hello.iss --package-only --output .\hello.ispkg

# 檢視封裝 manifest
.\bin\windows-x64\ironsetup.exe inspect .\samples\IronSetup-Demo-Setup.exe

# 靜默安裝；含空白的參數需整個加引號
.\samples\IronSetup-Demo-Setup.exe /VERYSILENT '/DIR=C:\Demo Apps\IronSetup' /TASKS= /NORESTART

# 驗證安裝內容
.\bin\windows-x64\ironsetup.exe verify --dir 'C:\Demo Apps\IronSetup'

# 同步命令列解除安裝，結束碼對應完整動作
.\bin\windows-x64\ironsetup.exe uninstall --dir 'C:\Demo Apps\IronSetup'

# 程序中斷、journal 顯示尚未完成時，回復已記錄交易
.\bin\windows-x64\ironsetup.exe recover --dir 'C:\Demo Apps\IronSetup'
```

支援 ISCC 常見短參數形狀：`/Q`、`/O目錄`、`/F名稱`、`/DName=value`；並非所有 ISCC 參數都相容。`/SILENT` 與 `/VERYSILENT` 在 v0.1 都完全隱藏精靈。`/LOG=完整路徑` 建立新記錄檔；如果已存在則拒絕覆寫。安裝程式不會自動重開機。

CLI 結束碼：`0` 成功、`2` 命令或操作失敗、`3` verify 偵測到遺失／修改的檔案。獨立安裝精靈：`0` 成功、`1` 失敗、`2` 使用者取消。Windows 自身解除安裝會先啟動暫存副本；原始 launcher 的 `0` 表示成功啟動副本，需要精準同步結束碼時使用外部 `ironsetup uninstall`。

## 安裝／解除安裝資料

安裝資料夾會包含 `.ironsetup` 狀態與備份，以及 `unins000.exe`。請勿把 `.ironsetup` 當快取刪除，否則無法可靠解除安裝或回復中斷交易。

解除安裝會比較目前內容與安裝時的 SHA-256：沒有修改的已安裝檔案可移除；先前被取代的原始檔會恢復；使用者修改後的檔案保留。若需要保留修改內容以及原始備份，報告會標示 `metadata_retained`，備份留在 `.ironsetup/backups`。不會遞迴清空整個應用程式目錄。

`verify` 檢查檔案及捷徑；Registry 不包含在 `intact` 判斷中，報告會明示。SHA-256 是完整性檢查，不代表發行者身分驗證。這版執行檔尚未數位簽章，也未實作 Authenticode 發行流程；不要對輸出 EXE 追加簽章資料後假設 trailer 仍可讀取。

## 已知界線

- **原地升級／重複安裝尚未實作**：同一安裝資料夾有有效 journal 時會拒絕；先解除安裝，再安裝新版本。
- Pascal `[Code]`、完整 ISPP、LZMA/LZMA2、solid compression、加密、密碼、網路下載、服務／驅動程式安裝、重開機置換、DLL／字型註冊、多語系訊息檔、32-bit／ARM64 原生 stub 尚未實作。
- 輸出 EXE／`.ispkg` 是 IronSetup 格式，不與 Inno Setup 二進位格式互通；`.ispkg` 也不是標準 ZIP 檔。
- payload 上限 **512 MiB、100,000 個檔案**；目前整包處理於記憶體，適合中小型應用程式。
- `[Run]` 在安裝交易提交後執行。外部程式的副作用無法回復；失敗會列入 warnings。`nowait` 只確認啟動。
- 暫存解除安裝副本可能留在 Windows Temp；這不影響已安裝應用的移除。
- 原生 Windows 實機、UAC 互動、高 DPI 多螢幕、SmartScreen、權限受限帳號與斷電狀況仍需目標機驗收。

## 開發、測試與重建

原始碼位於根目錄 `crates/`，不是另一個僅供展示的範本。`Cargo.lock` 固定依賴。

```powershell
# Windows：Rust stable + Visual Studio C++ Build Tools
powershell -File .\scripts\Build-Windows.ps1

# 包含會打開原生介面的自動操作測試
powershell -File .\scripts\Build-Windows.ps1 -GuiSmoke

# 已有便攜包時，不需要 Rust 的 Windows 生命週期驗收
powershell -File .\scripts\Test-Windows.ps1
```

Linux 可以執行 `bash scripts/ci.sh`。Linux 僅支援 CLI／檔案安裝；Registry、捷徑與 Windows GUI 不會假裝在 Linux 完成。包內 Linux 執行檔在 Ubuntu 24.04 建置，需要 glibc 2.39 或更新版本；較舊發行版請從原始碼重建。

首次從原始碼建置需要下載 Cargo 相依；預編譯便攜版執行時不需要 Cargo、Node.js、Python、WebView2 或網路。包裝腳本本身使用 Python 標準函式庫。

## Agent Team Workflow 證據

實際採用四個獨立 agent 實例：整合者、編譯器工程師、安裝引擎工程師、桌面介面工程師。每位作者在不同 Git worktree 寫自己的 crate；編譯器 agent 再以獨立角色審查引擎，重現並驗證修復兩個缺陷。

流程與狀態在 `docs/dispatch-plan.md`、`evidence/VERIFICATION.md`、`evidence/handoff.json`。本次未推送 GitHub、未建立線上 PR、未使用付費 CI 額度、未正式發行至外部服務；本地檢查記錄不冒稱 hosted CI。

## 參考與授權

以 [Inno Setup 官方功能](https://jrsoftware.org/isinfo.php)、[官方編譯器參數](https://jrsoftware.org/ishelp/topic_compilercmdline.htm)、[安裝參數](https://jrsoftware.org/ishelp/topic_setupcmdline.htm) 為公開行為參考。參考基準為官方 2026-08-12 發行的 7.1.0；查閱日期 2026-10-25（Asia/Taipei）。本專案未複製 Inno Setup 原始碼、品牌圖示或安裝引擎，也未呼叫 ISCC。

IronSetup 自有原始碼採 MIT 授權，見 `LICENSE`。相依與工具鏈授權見 `THIRD_PARTY_NOTICES.md` 以及發行包中的 `third-party/`。
