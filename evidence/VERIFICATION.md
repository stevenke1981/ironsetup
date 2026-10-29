# IronSetup v0.1.0 — 交付驗證

日期：2026-10-25（Asia/Taipei）。整合者：`/root`。產品程式碼驗證基準：`29f5a44`；此後只新增驗證 helper、文件與包裝資料。發行包的完整來源 revision 與檔案 SHA-256 見 `package-build.json` 及根目錄 `SHA256SUMS.txt`。

## 結論與環境

**相容子集已完成實作，Linux 與 Windows PE／Wine 的指定驗證通過，可以交付便攜 ZIP。原生 Windows 10／11 驗收仍為 NOT_RUN。**

本次環境是 Ubuntu 24.04 x86_64、Rust 1.90.0、MinGW-w64 Windows GNU 交叉編譯工具鏈、Wine 9.0 x64 與 Xvfb。Wine 使用本次工作專用的暫存 prefix。未連線到使用者的 Windows 電腦。所有產品程式均由這份 Rust 原始碼建置，沒有呼叫 Inno Setup／ISCC。

## 實際執行的檢查

| 檢查 | 結果 | 平台與證據 |
| --- | --- | --- |
| `cargo fmt --all -- --check` | PASS | Rust 原始碼格式 |
| workspace Clippy，全部 target、`-D warnings` | PASS | Linux：`clippy-linux.log`；Windows 交叉 target：`clippy-windows.log` |
| Linux workspace unit／integration tests | **67 PASS，0 FAIL** | `linux-unit-tests.log`；沒有忽略的 Linux 測試 |
| Windows workspace PE test executables | **64 PASS，0 FAIL** | 實際透過 Wine 執行，`windows-tests-wine.log`；1 個 opt-in GUI scenario 在這個預設測試命令中 ignored，下一列另行執行 |
| 原生控制項 GUI scenario | **1 PASS** | `wine-ui-smoke.log`、`ui.md`；Wine／Xvfb；此 scenario 使用測試 callback |
| 真正 Release Studio → 安裝精靈 → verify → uninstall | **PASS** | `real-studio-control-integration.log`、`real-studio-integration.md`；真實編譯與安裝 callback，無 mock |
| Linux Release CLI／SFX 生命週期 | **27 checks PASS** | `smoke-linux.json` |
| Windows Release CLI／SFX 生命週期 | **26 checks PASS** | `smoke-wine.json`；Windows EXE 實際在 Wine 執行 |
| Windows 原生 API 生命週期 | **1 PASS，已包含於上述 64 tests** | HKCU、COM `.lnk`、ARP、解除安裝器與清理；`windows-tests-wine.log`、`engine.md` |
| 獨立引擎缺陷審查 | **2 個重現缺陷均修復／獨立複驗 PASS** | `review-engine.md`、`review-engine-findings.json`；這 2 個 regression 已包含於 Linux 67 tests |
| Linux／Windows Release builds | PASS | `build-linux-release.log`、`build-windows-release.log` |
| 真實 Windows 10／11、UAC 互動、多螢幕 DPI、螢幕閱讀器 | **NOT_RUN** | 沒有原生 Windows 主機；不以 Wine 或交叉編譯取代此驗收 |
| GitHub PR、hosted CI、外部部署／發行 | **N/A** | 本次要求交付 ZIP；未執行這些動作 |

不同平台的 tests 數量不同，是因為 Unix 權限／symlink 與 Windows 路徑／原生 API 的條件編譯；不是漏跑相同測試。不同層的檢查可能重疊，不能把本表全部數字加總成「不同功能數量」。

## Release 整合測試做了什麼

`scripts/smoke.py` 對真實 Release CLI 執行腳本檢查、明確拒絕未支援的 Pascal、兩次編譯位元組一致、manifest 解析、元件子集安裝、完整性驗證、修改檔案後 verify 結束碼 3、保留使用者檔案、拒絕原地重複安裝、恢復被覆寫原始檔、拒絕損壞封裝且不先寫檔，以及產生／執行自解壓安裝程式。最後執行的是**安裝後產生的 `unins000.exe` 本身**，並確認安裝目錄清除。Linux 額外檢查原始檔權限，故為 27 checks；Windows 為 26。

`real_studio_smoke.rs` 的正常按鈕路徑開啟實際 Studio，按 F9 編譯獨立腳本，以實際 sibling stub 產生 SFX；接著開啟 SFX 的授權／目錄／安裝精靈。它確認 payload 逐位元組一致、解除安裝器存在、CLI `verify` 為 `intact: true`，再用 CLI 解除安裝並確認目錄移除。測試前後 Release EXE SHA-256 相同。

## 保留的失敗證據與修復

1. **引擎 journal 外部路徑與 Unix 原始權限**：獨立 reviewer 先重現 2 個失敗；作者修復後 reviewer 用新 target 目錄再次執行，2 PASS。完整經過見 `review-engine.md`。
2. **GUI 授權勾選框重繪**：實際畫面檢查發現背景遮住子控制項；修正 parent style 與換頁重繪後重新執行 GUI scenario 並目視確認。
3. **Windows canonical 路徑 `\\?\`**：Release 整合測試發現 Windows 路徑 prefix 的 `?` 被誤認為來源目錄 wildcard。修正為只檢查一般路徑 component，新增 regression；Windows 與 Linux tests、Release builds 及 smoke 均重新通過。原始失敗保留在 `smoke-wine-before-path-fix.json`。
4. **首次整合測試編譯失敗**：review regression 的 Unix 條件 attribute 放置錯誤，修正後完整測試通過；保留 `integration-initial-compile.log`。
5. **Wine 快速合成訊息 assertion**：第一次真實 SFX GUI driver 直接注入 Next 的 `WM_COMMAND`，在授權後發生 Wine `free(): invalid pointer`，當時尚未寫入安裝目錄。改為實際 Next 控制項 `BM_CLICK` 並等待 150 ms 換頁後，同一組 Release EXE 的完整流程通過，產品 source 未改。**首次 assertion 的根因尚未證明**，不可將其描述為已修復的產品缺陷；兩份 log 都保留於 `real-studio-*` 證據。

## 交付範圍與界線

發行包含 Windows x64 compiler／studio／stub、Windows 示範安裝器及 demo 應用、Linux x64 CLI／stub、完整 source／Cargo.lock、範例、build／test／package scripts、文件與授權。Windows PE 靜態 import 檢查只見 Windows 系統 DLL；沒有另需散發的 winpthread／GCC DLL。Linux 預編譯檔需要 glibc 2.39 或更新版本；可在較舊系統自行從 source 重建。

這版是 Inno 工作方式的獨立 Rust 子集，支援 9 個常用 ISS section。原地升級、Pascal、完整 ISPP、LZMA2、簽章、多語系精靈等尚未實作；UI 是英文。封裝為 IronSetup 自有格式，不可讀取既有 Inno EXE。SHA-256 是完整性校驗，不是發行者認證。完整差異見 `docs/COMPATIBILITY.md`。

包裝程式會在暫存 stage 重新編譯 3 個可用範例，產生 `sample-builds.json`、`package-build.json` 與每個檔案的 `SHA256SUMS.txt`，最後檢查 ZIP CRC。這些包裝步驟的結果應以實際產生的資料為準；本文件不代表原生 Windows 驗收或正式外部發行。

## 重跑入口

- Linux：`bash scripts/ci.sh`。
- Windows source build：`powershell -File scripts/Build-Windows.ps1`。
- 加入 opt-in GUI scenario：`powershell -File scripts/Build-Windows.ps1 -GuiSmoke`。
- 便攜 Windows 生命週期測試：`powershell -File scripts/Test-Windows.ps1`。
- Release CLI smoke：`python scripts/smoke.py --bin-dir bin/windows-x64 --report smoke.json`。

Windows PowerShell scripts 已提供供目標主機驗收；本次沒有在原生 Windows／PowerShell 中執行，不把 script 的存在視為測試通過。
