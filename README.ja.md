# MangaMeeya Rust

[English](README.md) | [正體中文](README.tw.md) | [日本語](README.ja.md)

Rust と [GPUI Kit](https://github.com/longbridge/gpui-kit) で全面的に書き直したオフライン漫画リーダーです。**Windows と macOS** に対応しています。
UI は GPU で直接描画し、Tauri、WebView、TypeScript、Node.js は使用しません。

## 機能

- 画像、画像フォルダー、ZIP/CBZ、RAR/CBR を開けます。サブフォルダーと自然順ソートに対応しています。
- English、日本語、正體中文の UI に対応しています。設定で即座に切り替えられ、選択は自動保存されます。
- 単ページ、見開き、縦方向の連続表示と、右から左／左から右への読書に対応しています。
- ウィンドウに合わせる、幅に合わせる、高さに合わせる、原寸表示と、10%–800% の表示倍率に対応しています。
- EXIF の向き、90 度回転、明るさ、コントラスト、グレースケール、アニメーション画像に対応しています。
- 長い画像は GPU textures に分割し、元の寸法と解像度を維持します。
- 仮想化したサムネイル列、ファイルブラウザー、最近開いた項目、漫画ごとの読書位置、しおりを利用できます。
- ネイティブのファイル選択・保存ダイアログ、キーボード、ホイール、画面左右のクリック、ウィンドウ全体へのドラッグ＆ドロップに対応しています。
- 全画面表示、一定間隔のスライドショー、元の画像の別名保存に対応しています。
- 単一ウィンドウで動作します。再度起動すると、新しいパスを既存のウィンドウに渡します。macOS では Finder からの書類を開くイベントに対応しています。
- Windows インストーラーと macOS app bundle は、画像と漫画のファイル関連付けを宣言します。

元の画像は変更しません。別名保存では、symlink や hard link を含め、開いている漫画の元ファイルへの上書きを拒否します。
暗号化された RAR はエラーを表示し、パスワードの入力要求や保存はしません。
各ページの元データは 128 MiB、デコードは 32 megapixels が上限です。アニメーションはさらに 256 frames、デコード後のデータ合計 128 MiB が上限です。
画像データは Rust が直接読み込み、Base64 や WebView IPC を経由して転送しません。
画像はバックグラウンドで読み込みます。表示領域から外れた画像は CPU cache と GPU atlas から解放します。連続表示では、スクロール位置を維持するために寸法データを保持します。

## 実行

[Rust](https://rustup.rs/) をインストールしてください。`rust-toolchain.toml` で Rust 1.99.1 を固定しています。GPUI Kit のバージョンと、その GPUI snapshot は `Cargo.lock` で固定しています。

### Windows

Windows 10／11、DirectX 11 対応 GPU、Visual Studio Build Tools の **Desktop development with C++** ワークロード、Windows SDK が必要です。

```powershell
cargo run --locked
cargo run --locked -- "C:\Comics\book.cbz"
```

WebView2 Runtime や Node.js は不要です。

### macOS

macOS 12 以降の Apple Silicon と Intel Mac に対応しています。ソースからのビルドには、Command Line Tools だけでなく、完全な Xcode と Metal compiler が必要です。
Xcode の初回起動時の設定や Metal component のインストールが完了していない場合は、先に Xcode で設定を完了してください。以下のコマンドは、ユーザーに代わってライセンスに同意したり、インストールの確認画面を開いたりしません。

```sh
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
xcrun --sdk macosx metal --version
cargo run --locked
cargo run --locked -- /path/to/book.cbz
```

## 操作

| 操作 | キー |
| --- | --- |
| 漫画を開く／フォルダーを開く | `Ctrl+O`／`Ctrl+Shift+O` |
| 元の画像を別名保存／漫画を閉じる | `Ctrl+S`／`Ctrl+W` |
| 次のページ／前のページ | `Space`、`PageDown`／`PageUp` |
| 読む方向に従ってページを移動 | `←`、`→` |
| 閲覧領域をスクロール | `↑`、`↓`、マウスホイール |
| 最初のページ／最後のページ | `Home`／`End` |
| 単ページ／見開き、連続表示、読む方向 | `S`、`C`、`D` |
| ウィンドウに合わせる／幅に合わせる／高さに合わせる／原寸表示 | `F`／`W`／`H`／`1` |
| 拡大／縮小／100% | `+`／`-`／`0` |
| 回転 | `[`／`]` |
| しおりを追加／しおりに移動 | `B`／`Shift+B` |
| スライドショー／サムネイル／サイドバー | `P`／`T`／`Tab` |
| 全画面表示／全画面表示を終了 | `F11`／`Escape` |

macOS では `Command+O`、`Command+Shift+O`、`Command+S`、`Command+W`、`Command+Q`、`Command+,`、`Control+Command+F` も使用できます。
通常のホイール操作はスクロールのみで、直接ページを切り替えません。`Ctrl` を押しながらホイールを操作すると拡大・縮小できます。単ページと見開きでは、閲覧領域の左右の端をクリックしてページを切り替えられます。

### ツールバーとパネル

ツールバーは、開く操作と左側パネル、閲覧モード、表示倍率と読む方向、しおりと読書ツール、全画面表示のグループに分かれています。「単ページ／見開き／連続表示」ボタンで指定したモードを直接選べます。現在のモードを再度選んでも、別のモードには切り替わりません。モードを切り替えても、現在の表示方法やカスタム倍率は維持します。`S`、`C` の既存の切り替え動作は変更しません。
全画面表示には既存のコマンドとショートカットを使用します。新しい集中モードではありません。UI は GPUI Kit、ActiveTheme、Lucide icons を引き続き使用し、テーマ切り替えや依存パッケージは追加しません。

左右のパネルは個別に開閉でき、境界をドラッグして幅を調整できます。表示状態と指定した幅は保存し、次回起動時に復元します。

- 左側には「ファイル／サムネイル／最近開いた項目」タブがあります。サムネイルは縦横比を維持し、ページ番号とファイル名を分けて表示します。Tooltip で完全なファイル名を確認できます。
- 右側の「読書ツール」には、次の 3 つのタブがあります。
  - **画像**：明るさ、コントラスト、グレースケール、回転と「フィルターをリセット」。「フィルターをリセット」は明るさ、コントラスト、グレースケールだけをリセットします。回転や読書の設定は変更しません。
  - **読書**：読書位置の記憶、読む方向、閲覧背景。「一般設定」には、言語と、破損した設定のバックアップ・復旧の操作があります。
  - **再生**：スライドショーの再生・一時停止と、1–120 秒の間隔設定。

右側のタブを切り替えても、値や未確定の入力はリセットしません。選択中のタブは次回起動時には復元しません。
「ツール」メニューの「設定」、またはプラットフォームのネイティブな Preferences から設定を開くと、右側の「読書」タブを直接表示します。
「サブフォルダーを含める」は **ファイル → 開くときのオプション** にあります。既存の `recursive_folder` を使用し、変更は次に漫画を開くときだけ適用します。現在の漫画は再読み込みしません。

### 表示倍率とページ番号

ツールバーの「表示倍率」は、表示方法と倍率を指定する唯一のコントロールです。現在のモードと実際の表示倍率を表示します。クリックすると Popover が開き、ウィンドウに合わせる、幅に合わせる、高さに合わせる、原寸表示を選べます。パーセントの入力や `−`／`+` での調整もできます。

- パーセントは現在のページを基準にします。見開きの 2 ページの寸法が異なる場合、各ページの倍率が異なることがあります。画像の寸法がまだ取得できていない場合は `—` を表示します。
- 手動入力、拡大・縮小、`Ctrl`＋ホイール操作は「カスタム倍率」に切り替わります。拡大・縮小とホイール操作は現在の実際の倍率を基準にし、手動入力は指定した倍率を適用します。手動で指定できる倍率は 10%–800% です。自動調整の倍率にはこの制限はありません。
- パーセントを入力して `Enter` を押すと、適用して Popover を閉じます。表示方法を選んだ場合も閉じます。`−`／`+` は連続して調整できます。`Escape` または外側のクリックで、未確定の入力を適用せずに閉じられます。

ページのナビゲーションは下部だけに配置し、前のページ／ページ番号／総ページ数／次のページを表示します。ツールバーには前のページと次のページを重複して配置しません。下部には読書の進捗と「ページ情報」の操作もあり、必要な状態とエラーを表示します。表示倍率やパネルの開閉操作は配置しません。
「ページ情報」の Popover には、現在のページの完全なファイル名、実際の画像サイズ、取得可能な元のファイルサイズを表示します。情報がまだない場合は「未取得」と表示します。ページを移動すると情報を更新します。漫画を閉じるか別の漫画を開くと、Popover を閉じて古い情報を消去します。
読書の進捗は、現在のページ番号を総ページ数で割り、四捨五入したパーセントで表示します。漫画を開いていない場合は空の状態を表示し、ページのナビゲーションを無効にします。

ページ番号は `Enter` を押すと適用します。スライドショーの間隔は `Enter` を押すか、入力欄からフォーカスを移すと適用します。

## 言語

**ツール → 設定**、またはプラットフォームのネイティブな Preferences から設定を開くと、**読書ツール → 読書** を表示します。
**一般設定 → 言語** で、**English**、**日本語**、**正體中文** から選んでください。
選択は即座に反映され、`settings.json` の `preferences.language` に保存されます
（`en`、`ja`、`zh-Hant`）。既定は英語です。このフィールドがない旧設定でも同じです。
メニュー、コントロール、ツールチップ、アクセシビリティラベル、リーダーのメッセージは、選択した言語で表示します。
ファイル名と技術的なエラーの詳細は変更しません。ネイティブのファイルダイアログのコントロールは OS の言語を使用します。
GPUI コンポーネントのテキストには対応する locale を使用します。GPUI が提供していないコンポーネントの文字列（日本語のテキスト編集メニューを含む）は英語で表示します。

## ローカルデータ

設定、読書位置、最近開いた項目、しおりは、ユーザーの `settings.json` に保存します。保存には一時ファイルと atomic replacement を使用します。

- Windows：`%APPDATA%\MangaMeeya\MangaMeeya Rust\config\settings.json`
- macOS：`~/Library/Application Support/org.MangaMeeya.MangaMeeya-Rust/settings.json`

実際のパスは `directories::ProjectDirs` が決定します。設定ファイルが破損している場合は、元のファイルを保持してエラーを表示します。「読書ツール → 読書 → 一般設定」の「設定をバックアップしてリセット」を押すと、まず `settings.json.corrupt-*` として保存してからリセットできます。
旧版の WebView local storage は、読み込み、削除、自動移行のいずれも行いません。
単一ウィンドウ用の lock と、認証付きの loopback endpoint もこのディレクトリに置きます。アプリ終了時に endpoint を削除します。

### パネル設定とダウングレード

パネルの設定は `preferences.panel_layout` に保存します。幅の単位は整数の logical px です。

| フィールド | 既定値 | 規則 |
| --- | --- | --- |
| `sidebar_width` | `224` | 216–320 logical px に制限 |
| `inspector_width` | `256` | 240–360 logical px に制限 |
| `inspector_visible` | `true` | 右側の読書ツールの表示状態 |

左側の表示状態には既存の `preferences.sidebar` だけを使用し、重複するフィールドは追加しません。旧設定に `panel_layout` または一部の子フィールドがない場合は、既定値を使用します。
幅は、ドラッグを完了し、値が実際に変わった場合だけ保存します。狭いウィンドウでは、右側、左側の順に実際の幅を制限し、中央の閲覧領域を少なくとも 280 logical px 確保します。この自動調整で保存済みの幅は上書きしません。ウィンドウを広げると、指定した幅に戻ります。

旧版は追加された設定フィールドを拒否します。ダウングレードする場合は、次の手順を使用してください。

1. アプリを終了し、先に `settings.json` をバックアップします。
2. このデスクトップ UI 変更より前の、3 言語に対応した版に戻す場合は、ダウングレード用の作業用コピーから `preferences.panel_layout` だけを削除します。`preferences.language` と、その他の既存フィールドは保持します。
3. 3 言語対応より前の版に戻す場合だけ、さらに `preferences.language` を削除します。

最近開いた項目、読書位置、しおり、その他の設定は削除しないでください。ダウングレードのために設定全体をリセットしないでください。

## チェック

macOS では、実行前に上記の `DEVELOPER_DIR` を設定してください。

```sh
cargo fmt --all -- --check
cargo test --locked --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --test rendering
```

通常のテストは、アーカイブ、ソート、安全上の制限、エクスポート、設定、EXIF、アニメーション、読書状態、launch IPC、GPUI コントロールの操作を対象にしています。
`rendering` は実際の `ReaderView` を使用し、ネイティブのキーボードとマウスのイベントを操作します。macOS では実際の Metal renderer でピクセルを検査し、画像を `target/ui-smoke/` に書き出します。画面収録の権限は不要です。
Windows の同じテストは、ネイティブの text system を使って操作を検査します。Metal やオフスクリーン GPU 描画を検証したと主張するものではありません。

## パッケージ作成

### Windows NSIS

別途 NSIS をインストールしてから、次を実行してください。

```powershell
pwsh -NoProfile -NonInteractive -File packaging/windows/build.ps1
```

出力先：`target/release/bundle/nsis/*-setup.exe`。
インストールは現在のユーザーのディレクトリと HKCU にのみ書き込みます。ファイル関連付けは **Open With** に追加し、既存の既定アプリは変更しません。アンインストールしても読書データは保持します。

### macOS app、ZIP、DMG

```sh
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
  bash packaging/macos/build.sh
# Intel build を指定する前に target をインストール：
rustup target add x86_64-apple-darwin
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer \
  bash packaging/macos/build.sh x86_64-apple-darwin
```

出力先：`target/release/bundle/macos/<target>/`。
パッケージには `.app`、ZIP、DMG が含まれ、ad-hoc signing でローカルの整合性を検証します。これは Developer ID signing ではなく、notarize もされていません。一般公開の前には、アカウント所有者による署名と notarize が必要です。

## CI と検証範囲

`.github/workflows/ci.yml` は、Linux x64、Windows x64、macOS arm64、macOS x86_64 で Rust のフォーマット、テスト、production UI の操作、Clippy を検証します。
Linux は headless layout とネイティブのテキストシステムを使用します。GPU rendering の検証やパッケージ作成は行いません。
Windows と macOS では、ネイティブパッケージの作成とアップロードも実行します。macOS では Metal rendering も検証します。
Windows job ではさらに、silent install／uninstall で全 11 種類のファイル関連付けと、既定の関連付けが保持されることをテストします。
各プラットフォームのインストールテストの状態は、実際の workflow の結果で確認してください。ローカルの cross-check は、Windows での実行やインストールの検証とは異なります。
完全な検証記録と、未実行のプラットフォーム別チェックは[検証記録](docs/verification.md)を参照してください。

リリースには [changenote](https://github.com/narumiruna/changenote) と `.github/workflows/release.yml` を使用します。
変更をマージする前に [release note](.changenote/README.md) を追加してください。生成された Release PR をマージすると、workflow が指定の commit を検証し、GitHub Release を作成して、3 プラットフォームのインストールパッケージを添付します。
有効化する前に、Actions による PR の作成を許可する必要があります。操作、再試行、署名の制限は[リリース手順](packaging/README.md#release-workflow)を参照してください。

## 構成

- `src/book.rs`：画像、フォルダー、ZIP／RAR、ディレクトリ参照、安全なエクスポート。
- `src/reader.rs`：読書状態、ナビゲーション、レイアウト計算。
- `src/imaging.rs`：制限付きデコード、EXIF、アニメーション、ピクセル処理。
- `src/settings.rs`：ユーザー設定、atomic save、破損データの保護。
- `src/i18n.rs`：English、日本語、正體中文の完全な UI 翻訳と、安全なパラメーター置換。
- `src/instance.rs`：単一プロセスと、認証付きの launch forwarding。
- `src/ui.rs`、`src/ui/tests.rs`：GPUI Kit ウィンドウ、バックグラウンド読み込み、texture tiles、操作テスト。
- `packaging/`：両プラットフォームのネイティブパッケージ。`tests/`：アーカイブの fixture とネイティブ描画テスト。

## ライセンス

このプロジェクトは [GNU Affero General Public License v3.0](LICENSE) に基づいてライセンスされています。
GPUI Kit、GPUI、Lucide icons には、それぞれのライセンスが適用されます。MangaMeeya の名称と元のアプリケーションに関する権利は、それぞれの権利者に帰属します。
