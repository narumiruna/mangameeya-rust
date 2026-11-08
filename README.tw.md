# MangaMeeya Rust

[English](README.md) | [正體中文](README.tw.md) | [日本語](README.ja.md)

以 Rust 與 [GPUI Kit](https://github.com/longbridge/gpui-kit) 完整改寫的離線漫畫閱讀器，同時支援 **Windows 與 macOS**。
介面直接由 GPU 繪製，不使用 Tauri、WebView、TypeScript 或 Node.js。

## 功能

- 開啟圖片、圖片資料夾、ZIP/CBZ 與 RAR/CBR；支援子資料夾與自然排序。
- English、日本語、正體中文介面，可在偏好設定即時切換並自動儲存。
- 單頁、雙頁跨頁、垂直連續頁面，以及右向左／左向右閱讀。
- 適合視窗、寬度、高度、原始尺寸，以及 10%–800% 縮放。
- EXIF 方向、90 度旋轉、亮度、對比、灰階與動畫圖片。
- 長圖片分割成 GPU textures，保留原始尺寸與解析度。
- 虛擬化縮圖列、檔案瀏覽器、最近開啟、每本書閱讀位置與書籤。
- 原生檔案選擇與儲存視窗、鍵盤、滾輪、畫面兩側點擊及全視窗拖放。
- 全螢幕、定時投影片及另存原始圖片。
- 單一主要閱讀視窗，另有獨立的偏好設定視窗。再次啟動會將新路徑交給現有閱讀視窗，即使偏好設定正在使用中也一樣；macOS 支援 Finder 的文件開啟事件。
- Windows 安裝程式與 macOS app bundle 均宣告圖片及漫畫檔案關聯。

來源圖片不會被修改；另存操作會拒絕覆寫本書的來源檔案，包括 symlink 與 hard link。
加密 RAR 會顯示錯誤，不會要求或保存密碼。
每頁原始資料限 128 MiB；解碼限 32 megapixels。動畫另限 256 frames 與合計 128 MiB 的解碼資料。
圖片資料由 Rust 直接載入，不經過 Base64 或 WebView IPC 傳輸。
載圖在背景執行。離開可見區的圖片會從 CPU cache 與 GPU atlas 釋放；連續頁面的尺寸資料會保留，以維持捲動位置。

## 執行

安裝 [Rust](https://rustup.rs/)。`rust-toolchain.toml` 固定 Rust 1.99.1；GPUI Kit 版本與其 GPUI snapshot 由 `Cargo.lock` 固定。

### Windows

需要 Windows 10／11、支援 DirectX 11 的 GPU，以及 Visual Studio Build Tools 的 **Desktop development with C++** 工作負載與 Windows SDK。

```powershell
cargo run --locked
cargo run --locked -- "C:\Comics\book.cbz"
```

不需要 WebView2 Runtime 或 Node.js。

### macOS

支援 macOS 12 以上的 Apple Silicon 與 Intel Mac。從原始碼建置需要完整 Xcode 與 Metal compiler，不只 Command Line Tools。
若 Xcode 尚未完成首次啟動或 Metal component 安裝，先在 Xcode 完成設定；以下命令不會代替使用者接受授權或開啟安裝提示。

```sh
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
xcrun --sdk macosx metal --version
cargo run --locked
cargo run --locked -- /path/to/book.cbz
```

## 操作

| 操作 | 按鍵 |
| --- | --- |
| 開啟漫畫／資料夾 | `Ctrl+O`／`Ctrl+Shift+O` |
| 另存原始圖片／關閉漫畫 | `Ctrl+S`／`Ctrl+W` |
| 檔案瀏覽／返回上一層 | `Backspace` |
| 開啟選取的檔案或資料夾 | `Enter`、雙擊 |
| 重新整理檔案瀏覽 | `F5` |
| 下一頁／上一頁 | `Space`、`PageDown`／`PageUp` |
| 依閱讀方向翻頁 | `←`、`→` |
| 捲動閱讀區 | `↑`、`↓`、滑鼠滾輪 |
| 第一頁／最後一頁 | `Home`／`End` |
| 單頁／雙頁、連續頁面、閱讀方向 | `S`、`C`、`D` |
| 適合視窗／寬度／高度／原尺寸 | `F`／`W`／`H`／`1` |
| 放大／縮小／100% | `+`／`-`／`0` |
| 旋轉 | `[`／`]` |
| 新增書籤／跳至書籤 | `B`／`Shift+B` |
| 投影片／縮圖／側欄 | `P`／`T`／`Tab` |
| 全螢幕／離開全螢幕 | `F11`／`Escape` |

macOS 同時支援 `Command+O`、`Command+Shift+O`、`Command+S`、`Command+W`、`Command+Q`、`Command+,` 與 `Control+Command+F`。
一般滾輪只捲動，不直接翻頁；按住 `Ctrl` 使用滾輪可縮放。單頁與雙頁可點擊閱讀區的左右邊緣翻頁。

### 中央檔案瀏覽

閱讀中按 `Backspace`，只把中央畫布改成黑底封面網格；上方工具列、左右面板與狀態列維持原位。選取檔案後按 `Enter` 或雙擊開書。資料夾會進入下一層；「開啟此資料夾」才會閱讀其中圖片。`Escape` 先關閉作用中的浮層，再返回目前書籍，最後才退出全螢幕。搜尋輸入保留正常文字編輯操作。

閱讀與瀏覽之間保留搜尋、排序、選取與捲動位置，以及書籍的頁碼、縮放、濾鏡與頁內位置。瀏覽時暫停投影片並停用閱讀控制。右側「瀏覽」分頁可調整封面大小、檔名與中繼資料顯示；這些瀏覽偏好不跨次啟動儲存。

提供自然／文字名稱、修改／建立時間、大小、類型、已知頁數、已知閱讀進度、最近開啟與固定隨機順序。資料夾優先不受升降冪影響；未知值永遠置後。頁數只取已開書籍或可見封面索引，排序不掃描整個書庫。「最近開啟」不是最後閱讀時間。

封面只在可見區附近載入，最多 2 個工作、32 MiB 圖片快取。資料夾封面只取第一層的第一張圖片。自動 ZIP／CBZ 封面只支援 classic、單卷、Stored／Deflated，且來源不超過 64 MiB、4096 項與 2 MiB metadata。RAR／CBR、ZIP64、不支援的結構或超限來源改用格式圖示並提供原因；不影響手動開書。按 `F5` 可重試並清除舊封面與頁數；不會修改來源漫畫。

### 工具列與面板

工具列依開啟與左側面板、閱讀模式、縮放與閱讀方向、書籤與閱讀工具及全螢幕分組。單頁／雙頁／連續按鈕直接選擇指定模式；重複選取目前模式不會切換至其他模式。切換模式會保留目前的適應方式或自訂縮放；`S`、`C` 仍保留既有切換行為。
全螢幕沿用既有命令與快捷鍵，不是新增的專注模式。介面沿用 GPUI Kit、ActiveTheme 與 Lucide icons，不新增主題切換或相依套件。

左右面板可獨立收合，拖曳邊界可調整寬度；顯示狀態與偏好寬度會儲存，重新開啟後恢復。

- 左側保留「檔案／縮圖／最近開啟」分頁。縮圖等比例顯示，頁碼與檔名分開，Tooltip 顯示完整檔名。
- 右側「閱讀工具」有三個分頁：
  - **瀏覽**：封面大小、檔名顯示與行數，以及中繼資料顯示。
  - **影像**：亮度、對比、灰階、旋轉與「重設濾鏡」。重設濾鏡只重設亮度、對比與灰階，不重設旋轉或閱讀偏好。
  - **播放**：投影片播放／暫停及 1–120 秒間隔。

切換右側工具分頁不會重設數值或未送出的輸入；目前分頁不跨次啟動儲存。
「包含子資料夾」位於 **檔案 → 開啟選項**，沿用 `recursive_folder`；變更只影響下次開啟，不重新載入目前漫畫。

### 偏好設定視窗

從工具列的「偏好設定」、**工具 → 偏好設定** 或平台原生 Preferences 入口開啟。macOS 也可按 `Command+,` 開啟。
偏好設定視窗最多只能開啟一個；重複開啟會啟用現有視窗。
這是可調整大小的獨立一般視窗，不會阻擋閱讀視窗操作。
初始大小為 560 × 520 logical px，最小為 440 × 360，支援垂直捲動；大小、位置及開啟狀態不會儲存。
開啟或關閉時，不會變更閱讀面板的顯示狀態、寬度、目前工具分頁、已開啟的漫畫或閱讀狀態。

- **閱讀**：記住閱讀位置、閱讀方向，以及黑色、深灰或紙色的閱讀背景。背景只套用於閱讀區。在任一視窗變更閱讀方向，兩邊控制項都會同步；重複選取目前方向不會變更。
- **一般設定**：語言選擇及損壞設定的備份與重設。

變更會立即生效，並沿用既有的自動儲存；視窗中沒有「儲存」或「套用」按鈕。儲存錯誤會顯示在偏好設定中。
焦點位於偏好設定時，按 `Escape` 或 `Ctrl+W` 只會關閉該視窗；macOS 的 `Command+W` 與 Windows 的 `Alt+F4` 也相同。漫畫仍保持開啟，程式不會結束。
偏好設定使用中不會觸發閱讀器的翻頁或縮放快捷鍵。**檔案 → 關閉漫畫** 仍會關閉漫畫，而非偏好設定。
關閉閱讀視窗或選擇「結束」會關閉兩個視窗並結束程式；macOS 的 `Command+Q` 也會結束程式。

### 縮放與頁碼

工具列保留唯一的「縮放」控制項，同時顯示目前模式與實際縮放比例。按下控制項可開啟 Popover，切換適合視窗、寬度、高度或原始尺寸，也可輸入百分比或使用 `−`／`+` 調整。

- 百分比以目前頁面為準；雙頁尺寸不同時，兩頁的比例可能不同。載圖尺寸尚未取得時顯示 `—`。
- 手動輸入、放大／縮小或 `Ctrl`＋滾輪會切換為「自訂縮放」。放大／縮小與滾輪以目前實際比例計算，手動輸入則套用指定比例。手動縮放限 10%–800%；自動適應的比例不受此範圍限制。
- 輸入百分比後按 `Enter` 套用並關閉 Popover。選擇適應方式也會關閉；`−`／`+` 可連續調整。按 `Escape` 或點擊外部可關閉而不套用未送出的輸入。

頁碼導覽只放在底部，包含上一頁／頁碼／總頁數／下一頁；工具列不再重複放置上一頁與下一頁。底部另有閱讀進度與頁面資訊入口，並保留必要狀態與可見錯誤，不放縮放或面板開關。
頁面資訊 Popover 顯示目前頁面的完整檔名、實際尺寸及可取得的來源檔案大小；資訊尚未取得時顯示「未取得」。換頁時更新資訊，關閉或替換漫畫時關閉浮層並清除舊資料。
閱讀進度為目前頁碼除以總頁數後四捨五入的百分比；未開啟漫畫時顯示空狀態並停用頁碼導覽。

頁碼只在按下 `Enter` 後套用；投影片間隔在按下 `Enter` 或離開欄位後套用。

## 語言

在 **偏好設定 → 一般設定 → 語言** 選擇 **English**、**日本語** 或 **正體中文**。
選擇會立即生效，並以 `preferences.language` 儲存於 `settings.json`
（`en`、`ja` 或 `zh-Hant`）。預設使用英文；舊設定未包含此欄位時也相同。
兩個視窗會立即更新。選單、控制項、工具提示、無障礙標籤與閱讀器訊息會使用所選語言。
檔案名稱與技術錯誤詳細資料維持原樣。原生檔案對話框的控制項使用作業系統語言。
GPUI 元件文字會使用對應的 locale；GPUI 未提供的元件字串（包括日文文字編輯選單）會改用英文。

## 本機資料

偏好、閱讀位置、最近開啟與書籤存於使用者的 `settings.json`，以暫存檔及 atomic replacement 儲存。
獨立偏好設定視窗不會變更設定結構，既有設定無需轉換。

- Windows：`%APPDATA%\MangaMeeya\MangaMeeya Rust\config\settings.json`
- macOS：`~/Library/Application Support/org.MangaMeeya.MangaMeeya-Rust/settings.json`

實際路徑由 `directories::ProjectDirs` 決定。設定檔損壞時會保留原檔並顯示錯誤；可在「偏好設定 → 一般設定」按「備份並重設設定」，先保留 `settings.json.corrupt-*` 再重設。
舊版 WebView local storage 不會被讀取、刪除或自動遷移。
單一程序的 lock 與經驗證的 loopback endpoint 也位於此目錄，關閉程式會移除 endpoint。

### 面板設定與降版

新增的面板偏好存於 `preferences.panel_layout`，寬度以整數 logical px 儲存：

| 欄位 | 預設值 | 規則 |
| --- | --- | --- |
| `sidebar_width` | `224` | 限制至 216–320 logical px |
| `inspector_width` | `256` | 限制至 240–360 logical px |
| `inspector_visible` | `true` | 右側閱讀工具的顯示狀態 |

左側顯示狀態仍使用既有 `preferences.sidebar`，不新增重複欄位。舊設定缺少 `panel_layout` 或部分子欄位時，使用預設值。
只有完成拖曳且寬度確實變更時才儲存。窄視窗會先限制右側、再限制左側的實際寬度，中央至少保留 280 logical px；這不會覆寫偏好寬度，放大視窗後會恢復。

舊版程式會拒絕新增欄位。降版時：

1. 關閉程式，先備份 `settings.json`。
2. 降版至本次桌面介面改版前、已支援三語系的基準版本時，只從降版用的工作副本移除 `preferences.panel_layout`；保留 `preferences.language` 及其他既有欄位。
3. 只有降版至尚未支援三語系的更舊版本時，才另外移除 `preferences.language`。

不要刪除最近開啟、閱讀位置、書籤或其他偏好，也不要為降版重設整份設定。

## 檢查

在 macOS 執行前先設定上述 `DEVELOPER_DIR`。

```sh
cargo fmt --all -- --check
cargo test --locked --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --test rendering
```

一般測試涵蓋封存檔、排序、安全限制、匯出、設定、EXIF、動畫、閱讀狀態、launch IPC 及 GPUI 控制項互動。
`rendering` 使用正式 `ReaderView`，操作原生鍵盤與滑鼠事件。在 macOS 會以真實 Metal renderer 檢查像素，將圖片寫入 `target/ui-smoke/`，不需要螢幕錄製權限。
Windows 的同一測試使用原生 text system 檢查互動；不宣稱有 Metal 或離屏 GPU 渲染。

## 打包

### Windows NSIS

另外安裝 NSIS，再執行：

```powershell
pwsh -NoProfile -NonInteractive -File packaging/windows/build.ps1
```

輸出：`target/release/bundle/nsis/*-setup.exe`。
安裝只寫入目前使用者的目錄與 HKCU。檔案關聯加入 **Open With**，不變更既有預設程式；解除安裝保留閱讀資料。

### macOS app、ZIP 與 DMG

```sh
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
  bash packaging/macos/build.sh
# 指定 Intel build 前先安裝 target：
rustup target add x86_64-apple-darwin
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
  bash packaging/macos/build.sh x86_64-apple-darwin
```

輸出：`target/release/bundle/macos/<target>/`。
套件包含 `.app`、ZIP 與 DMG，並使用 ad-hoc signing 驗證本機完整性。這不是 Developer ID signing，也未 notarize；公開發行前仍需由帳號持有人簽署及 notarize。

## CI 與驗證範圍

`.github/workflows/ci.yml` 在 Linux x64、Windows x64、macOS arm64 與 macOS x86_64 檢查 Rust 格式、測試、production UI 互動與 Clippy。
Linux 使用 headless layout 與原生文字系統，不驗證 GPU rendering，也不產生安裝套件。
Windows 與 macOS 另建置並上傳原生套件；macOS 也驗證 Metal rendering。
Windows job 另以 silent install／uninstall 測試全部 11 個檔案關聯及預設關聯保留行為。
請以實際 workflow 結果判定各平台的安裝測試狀態；本機 cross-check 不等於 Windows 執行或安裝驗證。
完整驗證記錄與尚未執行的平台檢查見 [驗證記錄](docs/verification.md)。

版本發布使用 [changenote](https://github.com/narumiruna/changenote) 與 `.github/workflows/release.yml`。
合併變更前加入 [release note](.changenote/README.md)；合併產生的 Release PR 後，工作流程會驗證指定 commit、建立 GitHub Release，並附上三平台安裝套件。
啟用前需允許 Actions 建立 PR；操作、重試及簽署限制見 [發布流程](packaging/README.md#release-workflow)。

## 結構

- `src/book.rs`：圖片、資料夾、ZIP／RAR、目錄瀏覽與安全匯出。
- `src/reader.rs`：閱讀狀態、導航與版面計算。
- `src/imaging.rs`：受限解碼、EXIF、動畫與像素處理。
- `src/settings.rs`：使用者設定、atomic save 與損壞資料保護。
- `src/i18n.rs`：English、日本語與正體中文的完整介面翻譯及安全參數替換。
- `src/instance.rs`：單一程序與經驗證的 launch forwarding。
- `src/ui.rs`、`src/ui/tests.rs`：GPUI Kit 視窗、背景載入、texture tiles 與互動測試。
- `packaging/`：雙平台原生套件；`tests/`：封存 fixture 與原生渲染測試。

## 授權

本專案依 [GNU Affero General Public License v3.0](LICENSE) 授權。
GPUI Kit、GPUI 與 Lucide icons 保留各自的授權。MangaMeeya 名稱及原始應用程式相關權利屬各自權利人所有。
