# CHANGELOG

CLI・Dockerイメージ・Ruby gemは**すべて同じバージョン**を使います
(docs/decisions/0061-distribution.md「バージョニング」)。
書式は[Keep a Changelog](https://keepachangelog.com/ja/1.1.0/)に従い、
バージョンは[Semantic Versioning](https://semver.org/lang/ja/)に従います
(0.xの間は互換性を壊す変更が入りえます)。

## [Unreleased]

最初のリリース(0.1.0)はまだ切っていません。ここまでに入っているもの:

### Added

- HTML/CSSレンダリングエンジン(Chromium/WebKit/Gecko非依存)。ブロック/
  インラインレイアウト、テーブル、リスト、Flexbox、Grid、`position`、
  段組みなど。対応プロパティは`docs/covered_css`を参照
- CSS Fragmentation(`break-before`/`break-after`/`break-inside`、
  `orphans`/`widows`)による改ページ制御
- ストリーミング入出力(`--streaming`)。数万要素規模のHTMLを低メモリで処理
- Webfont(`@font-face`、`unicode-range`、フォールバック)と、
  `--font`/`--gothic-font`等による決定的なフォント指定
- 画像の埋め込み、外部スタイルシート、`@import`、`url()`
- CLI(wkhtmltopdf互換に寄せたオプション体系)。ヘッダー/フッター、表紙、
  目次、ページ設定、メタデータ、アクセス制御
- HTTPサーバモード(`sghtmltopdf server`)。CLIと同じオプションをクエリ
  パラメータで受け、`?stream=1`でchunked転送に対応
- Ruby gem(`sghtmltopdf`)。`Sghtmltopdf.render`/`render_to_file`、
  Rails統合(`render pdf:`、ビューヘルパ)、`server_url`による
  HTTPサーバモードへの委譲
