# syntax=docker/dockerfile:1

# 公式Dockerイメージ(ghcr.io/waka/sghtmltopdf)。
# 方針は .claude/plans/decisions/0061-distribution.md を参照:
#
# * 素の実行ファイルは配布せず、サーバモード用のイメージとRuby gemだけを出す(決定2)
# * 対象は linux/amd64 と linux/arm64(Debian系=glibcのみ、決定3)
# * 日本語フォントを同梱して、何も用意しなくても日本語のPDFが出る状態にする(決定4)
# * ENTRYPOINT/CMDで「引数なし=サーバ・引数あり=CLI」の両方に使えるようにする(決定5)

ARG RUST_VERSION=1.96

# ---------------------------------------------------------------------------
# 1. 同梱する日本語フォントを取得する
# ---------------------------------------------------------------------------
# BIZ UDPGothic / BIZ UDPMincho の Regular と Bold(SIL OFL 1.1、計約23MB)。
# **静的なTrueType(glyf)であること**が要件(0061 決定4改訂)。
# CFFベースのOTFはPDFへ`/FontFile2`として埋め込まれてしまい、可変フォントは
# サブセット時に`gvar`が落ちてデフォルトマスタ(=最軽量)の字面になる。
#
# curlのためだけにパッケージを足さずに済むよう、ビルド段と同じrustイメージを使う
# (buildpack-depsベースなのでcurlとCA証明書が入っている)。ビルドホストのアーキで
# 動かせばよいので --platform=$BUILDPLATFORM を付ける。
FROM --platform=$BUILDPLATFORM rust:${RUST_VERSION}-bookworm AS fonts
# google/fontsのコミットで固定する。上げるときは docker/fonts.sha256 も更新すること。
ARG GOOGLE_FONTS_COMMIT=7ff85c87f93ea6cca5f41c69f2e4edcb90240f26
WORKDIR /fonts
COPY docker/fonts.sha256 ./
RUN set -eux; \
    base="https://raw.githubusercontent.com/google/fonts/${GOOGLE_FONTS_COMMIT}/ofl"; \
    curl -fsSL -o BIZUDPGothic-Regular.ttf "${base}/bizudpgothic/BIZUDPGothic-Regular.ttf"; \
    curl -fsSL -o BIZUDPGothic-Bold.ttf    "${base}/bizudpgothic/BIZUDPGothic-Bold.ttf"; \
    curl -fsSL -o BIZUDPMincho-Regular.ttf "${base}/bizudpmincho/BIZUDPMincho-Regular.ttf"; \
    curl -fsSL -o BIZUDPMincho-Bold.ttf    "${base}/bizudpmincho/BIZUDPMincho-Bold.ttf"; \
    curl -fsSL -o OFL-BIZUDPGothic.txt     "${base}/bizudpgothic/OFL.txt"; \
    curl -fsSL -o OFL-BIZUDPMincho.txt     "${base}/bizudpmincho/OFL.txt"; \
    sha256sum -c fonts.sha256; \
    rm fonts.sha256

# ---------------------------------------------------------------------------
# 2. 実行ファイルをビルドする
# ---------------------------------------------------------------------------
# **ビルドは常にビルドホストのアーキで行う**(--platform=$BUILDPLATFORM)。
# QEMUの中でrustcを動かすと数十分かかるため、arm64向けはクロスコンパイルする。
# システムライブラリへのリンクは無いが、rustlsが使うringがCのソースを
# コンパイルするため、クロスのgccと**ターゲット側のlibcヘッダ**
# (libc6-dev-arm64-cross)が要る。これが無いとホストの/usr/includeを
# 拾って`bits/libc-header-start.h`が無いと言われる。
FROM --platform=$BUILDPLATFORM rust:${RUST_VERSION}-bookworm AS builder
ARG TARGETARCH
RUN set -eux; \
    case "${TARGETARCH}" in \
      amd64) target=x86_64-unknown-linux-gnu ;; \
      arm64) target=aarch64-unknown-linux-gnu; \
             apt-get update; \
             apt-get install -y --no-install-recommends \
                 gcc-aarch64-linux-gnu libc6-dev-arm64-cross; \
             rm -rf /var/lib/apt/lists/* ;; \
      *) echo "対応していないTARGETARCH: ${TARGETARCH}" >&2; exit 1 ;; \
    esac; \
    echo "${target}" > /target.txt; \
    rustup target add "${target}"
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc

WORKDIR /src
# ワークスペースのメンバーは core だけ(bindings/rubyはexclude)。
COPY Cargo.toml Cargo.lock ./
COPY core ./core
# targetディレクトリはキャッシュマウントなので、成果物は同じRUNの中で取り出す。
RUN --mount=type=cache,target=/usr/local/cargo/registry,id=cargo-registry \
    --mount=type=cache,target=/src/target,id=cargo-target-${TARGETARCH} \
    set -eux; \
    target=$(cat /target.txt); \
    cargo build --release --locked --target "${target}"; \
    cp "target/${target}/release/sghtmltopdf" /usr/local/bin/sghtmltopdf

# ---------------------------------------------------------------------------
# 3. 実行イメージ
# ---------------------------------------------------------------------------
# TLSのルート証明書は実行ファイルに埋め込まれている(rustls + webpki-roots)ため、
# ca-certificatesは要らない。
FROM debian:bookworm-slim
LABEL org.opencontainers.image.title="sghtmltopdf" \
      org.opencontainers.image.description="Chromium/WebKit/Geckoに依存しないHTML→PDFレンダラー" \
      org.opencontainers.image.source="https://github.com/waka/sghtmltopdf" \
      org.opencontainers.image.licenses="MIT"

COPY --from=builder /usr/local/bin/sghtmltopdf /usr/local/bin/sghtmltopdf
# fontdbが走査する標準ディレクトリの下に置く(fontconfigは要らない)。
COPY --from=fonts /fonts/BIZUDP*.ttf /usr/share/fonts/truetype/sghtmltopdf/
COPY --from=fonts /fonts/OFL-*.txt /usr/share/doc/sghtmltopdf/fonts/

# rootでは動かさない。/etc/passwdへ登録せず数字のUIDだけを使うのは、
# **この段でRUNを1つも実行しないため**(実行するとarm64のビルドにQEMUが要る)。
# 入出力はホストからマウントしたディレクトリで行うので、書き込み権限は
# `--user "$(id -u):$(id -g)"`を付けてホスト側の所有者に合わせる。
WORKDIR /work
USER 10001:10001

# 既定の`--listen 127.0.0.1`はコンテナの外から届かないので、CMDで明示する(0061 決定5)。
EXPOSE 8080
ENTRYPOINT ["sghtmltopdf"]
CMD ["server", "--listen", "0.0.0.0:8080"]
