# sghtmltopdf

An HTML-to-PDF renderer written in Rust that does **not** depend on Chromium,
WebKit, or Gecko.

📖 **[Documentation](https://waka.github.io/sghtmltopdf/)**
([English](https://waka.github.io/sghtmltopdf/en/))

## What is this

sghtmltopdf turns HTML into PDF without starting a browser process. It is aimed
at documents that flow top to bottom with explicit breaks — invoices, receipts,
reports — rather than at rendering arbitrary web pages.

The name follows wkhtmltopdf (archived in 2023): `sg` stands for *Second
Generation*.

Compared with the common headless-Chrome approach:

* **No browser process.** One binary, or a native extension living inside your
  Ruby process.
* **Fonts are resolved during rendering.** There is no `document.fonts.ready` to
  wait for, so a PDF is never emitted with unresolved webfonts.
* **Streaming.** HTML is read in chunks and each page is written out as soon as
  its layout is final, so memory does not grow with document size
  (measured: 516MB → 40MB on a 60,000-element document).
* **Page breaks are first class.** CSS Fragmentation (`break-before`,
  `break-inside`, `orphans`, `widows`) and `@page` are implemented directly.

Non-goals: executing JavaScript, pixel-perfect parity with browsers, and full
CSS coverage. See
[what is not supported](https://waka.github.io/sghtmltopdf/appendix/limitations.html).

## Usage

Three entry points share the same engine and the same options.

```sh
# CLI
sghtmltopdf invoice.html -o invoice.pdf --page-size A4 --margin-top 20mm

# HTTP server
sghtmltopdf server --listen 127.0.0.1:8080
curl --data-binary @invoice.html 'http://127.0.0.1:8080/pdf?page-size=A4' -o invoice.pdf
```

```ruby
# Ruby / Rails (gem "sghtmltopdf")
pdf = Sghtmltopdf.render("<h1>Invoice</h1>", page_size: "A4")

# In Rails, wicked_pdf-compatible renderer
render pdf: "invoice", template: "invoices/show", layout: "pdf"
```

Everything else — the full option reference, the HTTP API, CSS support tables,
and migration guides from wkhtmltopdf and wicked_pdf — lives in the
**[documentation site](https://waka.github.io/sghtmltopdf/)**.

> **Note**
> Version 0.1.0 has not been released yet. The gem is not on rubygems.org and no
> Docker image is published. Build from source for now.

## Guide for developer

### Layout

```
core/           # Rust engine + CLI + HTTP server (no Ruby dependency)
bindings/ruby/  # Ruby binding (magnus + rb-sys) and the Rails integration
docs/           # Documentation site (mdbook, ja + en)
```

`bindings/ruby` is excluded from the root Cargo workspace, so a plain
`cargo build` works without a Ruby toolchain.

### Rust

```sh
cargo build --release                                   # binary at target/release/sghtmltopdf
cargo test --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

Feature flags: `cli` (argument parsing) and `server` (HTTP server) are on by
default. Build the library alone with `--no-default-features`.

### Ruby gem

Requires a Rust toolchain and libclang (for `rb-sys`).

```sh
cd bindings/ruby
bundle install
bundle exec rake            # compile, then run specs
bundle exec rake compile
```

Some specs compare gem output with the CLI byte for byte, and are skipped unless
`target/release/sghtmltopdf` exists — run `cargo build --release` first.

### Documentation site

```sh
cargo install mdbook mdbook-mermaid mdbook-i18n-helpers

cd docs
mdbook serve                                        # Japanese (the source language)
MDBOOK_BOOK__LANGUAGE=en mdbook serve -d book/en    # English
```

Markdown under `docs/src` is written in Japanese; the English version is a
gettext catalog at `docs/po/en.po`. After editing the Japanese text, refresh the
catalog (requires the `gettext` package):

```sh
MDBOOK_OUTPUT='{"xgettext": {}}' mdbook build -d po
msgmerge --update po/en.po po/messages.pot
```

Entries whose source changed are marked `fuzzy` — review and update those, then
remove the marker. Pushing to `main` builds both languages and deploys to GitHub
Pages.

## License

MIT License ([LICENSE](LICENSE)).
