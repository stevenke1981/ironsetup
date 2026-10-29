# Inno Setup 對齊清單 — v0.1.0

基準：Inno Setup 7.1.0 官方公開文件。狀態「已實作」描述程式能力；不同平台執行證據另見 `evidence/VERIFICATION.md`。這不是相容率百分比，也不是既有 Inno EXE 格式的實作。

| Inno 類別 | IronSetup v0.1 | 明確界線 |
| --- | --- | --- |
| `.iss` 與 section 語法 | 已實作子集 | UTF-8／BOM、全行註解、引號、分號、重複鍵檢查；未知語法報錯 |
| ISPP | 部分 | 僅 literal define/include/替換；無函式、條件編譯、expression evaluator |
| `[Setup]` | 部分 | AppId／Name／Version／Publisher、DefaultDir／Group、權限、解除安裝開關、輸出／來源、文字資訊、x64compatible |
| `[Files]`／`[Dirs]` | 已實作子集 | 檔案只安裝到 `{app}`，沒有系統目錄 DLL／字型寫入 |
| 檔案版本比較 | 拒絕隱含行為 | 必須明確給 `ignoreversion` 或 `onlyifdoesntexist` |
| 遞迴／排除／空目錄 | 已實作 | wildcard 是記錄在文件中的子集；防循環、symlink、reparse |
| `[Types]`／`[Components]` | 已實作子集 | 固定元件、自訂類型；無階層繼承／exclusive groups |
| `[Tasks]` | 已實作子集 | unchecked、元件條件；無 task-to-task 條件 |
| Components／Tasks 表達式 | 已實作 | not → and → or 優先序、括號、空白 OR 清單；檢查名稱存在 |
| `[Icons]` | 已實作子集 | COM ShellLink；群組、使用者桌面／程式選單；目標限應用程式內／解除安裝器 |
| `[Registry]` | 已實作子集 | HKCU／HKLM／HKA；string、expandsz、dword、qword、binary；uninsdeletevalue |
| `[Run]` | 已實作子集 | 應用內執行檔、skipifsilent／runhidden／nowait／waituntilterminated |
| `[INI]`／`[InstallDelete]`／`[UninstallDelete]` | 尚未實作 | 編譯時拒絕，避免廣泛刪除或未定義副作用 |
| `[UninstallRun]`／postinstall／shellexec | 尚未實作 | 編譯時拒絕 |
| Pascal `[Code]` | 尚未實作 | 無 Pascal VM／API bridge，編譯時拒絕 |
| 單一 EXE 安裝檔 | 已實作 | IronSetup stub＋manifest＋壓縮 payload＋trailer |
| Inno 二進位相容 | 尚未實作 | 無法讀取／轉換 Inno 封裝或 uninstall log |
| 壓縮 | 部分 | `zip` 對應 deflate；`none` 對應未壓縮；LZMA／solid 拒絕 |
| 加密／密碼 | 尚未實作 | 不提供無效的加密開關 |
| Authenticode／SignTool | 尚未實作 | 輸出未簽章；EOF trailer 尚未處理後追加的憑證表 |
| 安裝精靈 | 已實作 | 原生控制項、文字授權／資訊頁、選擇、進度與結果；英文 |
| 編譯 IDE | 部分 | New／Open／Save／Save As、UTF-8 編輯器、F9、背景編譯、log；沒有語法上色／Pascal debugger |
| 靜默安裝／解除安裝 | 已實作子集 | `/SILENT` 和 `/VERYSILENT` 都無 GUI；不執行重新開機 |
| 解除安裝登錄 | 已實作 | Windows Installed Apps／ARP、DisplayName／Version／Publisher／命令 |
| rollback／恢復 | 已實作 | 寫前 journal、備份、鎖、recover；保留未解決的備份 |
| 原地更新／降版 | 尚未實作 | 既有 journal 明確拒絕；先解除安裝 |
| 刪除使用者修改資料 | 不自動執行 | 使用者修改的 payload 與未擁有檔案保留 |
| 程式關閉／Restart Manager／重開機置換 | 尚未實作 | 檔案鎖定導致明確失敗／rollback，不強制關閉其他程式 |
| 網路下載／dependency bootstrapper | 尚未實作 | 本版只處理本地來源 |
| 多語系訊息／RTL／主題 | 尚未實作 | 介面英文，資料使用 Unicode；依 Windows 原生控制項呈現 |
| Windows x64 | 建置目標 | 提供實際 PE32+ EXE；原生 Windows UAT 狀態獨立標示 |
| Windows x86／ARM64 | 尚未實作 | 沒有對應原生 stub |
| Linux | CLI／檔案生命週期 | Windows 特有動作在變更前拒絕 |

## 預設值與命令差異

`PrivilegesRequired` 預設是 `lowest`、`Compression` 預設是 `zip`。未明寫時編譯器會提醒；與 Inno 預設不同。`/COMPONENTS` 在本版會明確覆寫選擇，即使同時指定非 custom `/TYPE`；不提供 Inno 所有階層 `*`／`!` 選擇語法。`/LOG` 不覆寫既有檔案。CLI 結束碼與完全相容的 ISCC 回傳碼不是同一個承諾。

本版的 `[Registry]` 沒有 `uninsdeletevalue` 就在解除安裝時保留該值；有此 flag 時還原原值或刪除本次新增值，且保留安裝後的外部修改。metadata 的一致性檢查不是發行者身分或惡意使用者權限隔離。

## 後續擴充順序

1. 真正 Windows 10／11 的安裝器生命週期、UAC、非管理員、多螢幕 DPI 與鎖定檔驗收。
2. 原地更新／降版、版本比較、跨版本 journal 與備份所有權。
3. 簽章相容的 PE certificate table 與已簽章 uninstaller／暫存副本。
4. 串流封裝／更大型檔案、LZMA2、多語系訊息。
5. 可測試的擴充 API／hook；Pascal VM 是獨立大項，不用簡易字串替換假裝相容。

參考：[官方功能](https://jrsoftware.org/isinfo.php)、[Files](https://jrsoftware.org/ishelp/topic_filessection.htm)、[Components](https://jrsoftware.org/ishelp/topic_componentssection.htm)、[Run](https://jrsoftware.org/ishelp/topic_runsection.htm)、[命令列](https://jrsoftware.org/ishelp/topic_setupcmdline.htm)。查閱：2026-10-25。
