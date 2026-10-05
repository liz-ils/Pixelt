# Pixelt

Rust + Tauri 製の画像総合ツール（Windows 向け）。

## 機能

- 変換（JPEG / PNG / WebP / GIF / BMP / TIFF / QOI / AVIF、品質調整つき）
- 一括変換（並列処理・進捗表示・上書き設定）
- モザイク（四角選択・ブラシ塗り・ライブプレビュー）
- リサイズ（割合 / 幅・高さ、縦横比維持あり）
- メタデータ閲覧・編集・削除（PNG テキスト情報、JPEG EXIF）
- 3テーマ（ライト / ダーク / ハイコントラスト、デジタル庁デザインシステム基準の青基調）

## 開発

要件: Rust 1.99 以降、Node.js 22 以降、pnpm、Visual Studio Build Tools、WebView2 Runtime。

```sh
pnpm install
pnpm tauri dev
```

テスト・静的解析:

```sh
pnpm exec tsc --noEmit
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
cargo test --manifest-path src-tauri/Cargo.toml
```

配布用ビルド:

```sh
pnpm tauri build
```

`src-tauri/target/release/bundle/` に NSIS インストーラ等が生成される。

## ライセンス

MIT License。詳細は [LICENSE](LICENSE) を参照。
