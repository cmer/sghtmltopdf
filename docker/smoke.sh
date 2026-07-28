#!/usr/bin/env bash
# 公式Dockerイメージのスモークテスト。CIとローカルの両方から使う。
#
#   docker/smoke.sh ghcr.io/waka/sghtmltopdf:latest
#   PLATFORM=linux/arm64 docker/smoke.sh sghtmltopdf:arm64   # QEMUで動かす場合
#
# 見ているのは3つ:
#   1. 実行ファイルがそのアーキで動くこと(--version)
#   2. CLIとして日本語のPDFが出せること(同梱フォントが効いていること。
#      豆腐の警告が出たらフォントを見つけられていない)
#   3. 引数なしでサーバとして起動し、コンテナの外から /healthz と /pdf に
#      届くこと(`--listen 0.0.0.0`がCMDで効いていること)
set -euo pipefail

image="${1:?usage: docker/smoke.sh <image> }"
platform_args=()
if [ -n "${PLATFORM:-}" ]; then
    platform_args=(--platform "$PLATFORM")
fi

workdir="$(mktemp -d)"
container="sghtmltopdf-smoke-$$"
cleanup() {
    docker rm -f "$container" >/dev/null 2>&1 || true
    rm -rf "$workdir"
}
trap cleanup EXIT

cat > "$workdir/smoke.html" <<'HTML'
<!doctype html>
<html lang="ja"><body>
<h1>請求書</h1>
<p style="font-family: sans-serif">ゴシック体の日本語 Gothic 123</p>
<p style="font-family: serif">明朝体の日本語 Mincho 123</p>
<p style="font-weight: bold">太字の日本語</p>
</body></html>
HTML

echo "== 1. --version"
docker run --rm "${platform_args[@]}" "$image" --version

echo "== 2. CLIとして変換する"
# 出力ファイルをホスト側の所有者で書けるよう --user を渡す(Dockerfile参照)。
docker run --rm "${platform_args[@]}" --user "$(id -u):$(id -g)" \
    -v "$workdir:/work" -w /work "$image" smoke.html -o out.pdf 2> "$workdir/stderr.txt" || {
    cat "$workdir/stderr.txt" >&2
    echo "::error::CLIでの変換に失敗しました" >&2
    exit 1
}
cat "$workdir/stderr.txt"
if grep -q "豆腐" "$workdir/stderr.txt"; then
    echo "::error::同梱フォントで描画できない文字があります(フォントが見つかっていない可能性)" >&2
    exit 1
fi
head -c 5 "$workdir/out.pdf" | grep -q "%PDF" || {
    echo "::error::出力がPDFではありません" >&2
    exit 1
}
size=$(wc -c < "$workdir/out.pdf")
# 日本語のグリフが埋め込まれていれば数KB以上になる(空PDFとの区別)。
if [ "$size" -lt 5000 ]; then
    echo "::error::PDFが小さすぎます(${size}バイト)。フォントが埋め込まれていない可能性" >&2
    exit 1
fi
echo "   -> ${size}バイトのPDFができました"

echo "== 3. サーバとして起動する"
docker run -d --name "$container" "${platform_args[@]}" \
    -p 127.0.0.1:18080:8080 "$image" >/dev/null
for _ in $(seq 1 60); do
    if curl -fsS http://127.0.0.1:18080/healthz >/dev/null 2>&1; then
        break
    fi
    sleep 1
done
curl -fsS http://127.0.0.1:18080/healthz || {
    docker logs "$container" >&2
    echo "::error::/healthz に届きません" >&2
    exit 1
}
echo
curl -fsS http://127.0.0.1:18080/version
echo
status=$(curl -sS -o "$workdir/server.pdf" -w '%{http_code}' \
    -X POST -H 'Content-Type: text/html' \
    --data-binary "@$workdir/smoke.html" http://127.0.0.1:18080/pdf)
if [ "$status" != "200" ]; then
    docker logs "$container" >&2
    echo "::error::/pdf が $status を返しました" >&2
    exit 1
fi
head -c 5 "$workdir/server.pdf" | grep -q "%PDF" || {
    echo "::error::サーバの出力がPDFではありません" >&2
    exit 1
}
echo "   -> サーバからも $(wc -c < "$workdir/server.pdf")バイトのPDFが返りました"

echo "== スモークテスト成功: $image"
