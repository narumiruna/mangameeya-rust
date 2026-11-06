# MangaMeeya Rust

以 Rust 與 [GPUI Kit](https://github.com/longbridge/gpui-kit) 完整改寫的離線漫畫閱讀器，同時支援 **Windows 與 macOS**。
介面直接由 GPU 繪製，不使用 Tauri、WebView、TypeScript 或 Node.js。

## 功能

- 開啟圖片、圖片資料夾、ZIP/CBZ 與 RAR/CBR；支援子資料夾與自然排序。
- 單頁、雙頁跨頁、垂直連續頁面，以及右向左／左向右閱讀。
- 適合視窗、寬度、高度、原始尺寸，以及 10%–800% 縮放。
- EXIF 方向、90 度旋轉、亮度、對比、灰階與動畫圖片。
- 長圖片分割成 GPU textures，保留原始尺寸與解析度。
- 虛擬化縮圖列、檔案瀏覽器、最近開啟、每本書閱讀位置與書籤。
- 原生檔案選擇與儲存視窗、鍵盤、滾輪、畫面兩側點擊及全視窗拖放。
- 全螢幕、定時投影片及另存原始圖片。
- 單一視窗：再次啟動會將新路徑交給現有視窗；macOS 支援 Finder 的文件開啟事件。
- Windows 安裝程式與 macOS app bundle 均宣告圖片及漫畫檔案關聯。

來源圖片不會被修改；另存操作會拒絕覆寫本書的來源檔案，包括 symlink 與 hard link。
加密 RAR 會顯示錯誤，不會要求或保存密碼。
每頁原始資料限 128 MiB；解碼限 32 megapixels。動畫另限 256 frames 與合計 128 MiB 的解碼資料。
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
頁碼與縮放欄位在按下 `Enter` 後套用；投影片間隔在按下 `Enter` 或離開欄位後套用。

## 本機資料

偏好、閱讀位置、最近開啟與書籤存於使用者的 `settings.json`，以暫存檔及 atomic replacement 儲存。

- Windows：`%APPDATA%\MangaMeeya\MangaMeeya Rust\config\settings.json`
- macOS：`~/Library/Application Support/org.MangaMeeya.MangaMeeya-Rust/settings.json`

實際路徑由 `directories::ProjectDirs` 決定。設定檔損壞時會保留原檔並顯示錯誤；可在偏好設定按「備份並重設設定」，先保留 `settings.json.corrupt-*` 再重設。
舊版 WebView local storage 不會被讀取、刪除或自動遷移。
單一視窗的 lock 與經驗證的 loopback endpoint 也位於此目錄，關閉程式會移除 endpoint。

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

`.github/workflows/windows.yml` 在 Windows x64、macOS arm64 與 macOS x86_64 執行測試、Clippy、打包與 artifact 上傳。
Windows job 另以 silent install／uninstall 測試全部 11 個檔案關聯及預設關聯保留行為。
請以實際 workflow 結果判定各平台的安裝測試狀態；本機 cross-check 不等於 Windows 執行或安裝驗證。
完整驗證記錄與尚未執行的平台檢查見 [驗證記錄](docs/verification.md)。

## 結構

- `src/book.rs`：圖片、資料夾、ZIP／RAR、目錄瀏覽與安全匯出。
- `src/reader.rs`：閱讀狀態、導航與版面計算。
- `src/imaging.rs`：受限解碼、EXIF、動畫與像素處理。
- `src/settings.rs`：使用者設定、atomic save 與損壞資料保護。
- `src/instance.rs`：單一程序與經驗證的 launch forwarding。
- `src/ui.rs`、`src/ui/tests.rs`：GPUI Kit 視窗、背景載入、texture tiles 與互動測試。
- `packaging/`：雙平台原生套件；`tests/`：封存 fixture 與原生渲染測試。

## 授權

本專案依 [GNU Affero General Public License v3.0](LICENSE) 授權。
GPUI Kit、GPUI 與 Lucide icons 保留各自的授權。MangaMeeya 名稱及原始應用程式相關權利屬各自權利人所有。
