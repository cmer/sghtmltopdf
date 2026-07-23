//! `<img src>`のURL/パス分類。
//!
//! ネットワークフェッチ([0013](../../../docs/decisions/0013-image-fetch-security.md))・
//! ローカルファイル読み込み・`data:` URIのデコードのどれを行うべきかを、
//! 実際に取得を試みる前に判別する。判別のみを行い、実際のフェッチ/読み込み
//! (T46)は行わない。

use std::path::{Path, PathBuf};

use base64::alphabet::STANDARD as BASE64_STANDARD_ALPHABET;
use base64::engine::general_purpose::GeneralPurposeConfig;
use base64::engine::{DecodePaddingMode, GeneralPurpose};
use base64::Engine;

/// `<img src>`の値を分類した結果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImgSrc {
    /// `base_dir`相対のローカルファイルパスとして扱う値。
    ///
    /// `http`/`https`/`data:`のいずれにも一致しなかった値がここに来る。
    /// `file:`は明示的に別扱い(拒否)のため、ここには来ない
    /// ([0013](../../../docs/decisions/0013-image-fetch-security.md)の
    /// 「ローカル相対パスとURLスキームを取り違えさせない」方針通り)。
    LocalPath(String),
    /// `http`/`https`の絶対URL。実際のフェッチはT46が
    /// [0013](../../../docs/decisions/0013-image-fetch-security.md)の
    /// ポリシーに従って行う。
    RemoteUrl(String),
    /// `data:`URI(base64エンコードされたペイロードのみ対応。
    /// パーセントエンコードされた非base64ペイロードは画像用途では
    /// 実質使われないため未対応)。ネットワークもファイルI/Oも介さないため、
    /// T42のセキュリティポリシーの対象外。
    DataUri { mime_type: String, bytes: Vec<u8> },
}

/// `raw`(生の参照値)を`<base href>`に対して解決する([0040](
/// ../../../docs/decisions/0040-base-href-design.md)決定2)。
///
/// `base`が`None`、または`raw`が絶対参照(`http(s)`/`data:`)の場合は`raw`を
/// そのまま返す。`base`が`http(s)`の絶対URLならURLとして結合し、そうでなければ
/// ローカルパスのディレクトリ前置として扱う(root-relativeな`raw`はどちらの
/// 場合も基準のルートを使うため前置しない)。
pub fn resolve_against_base_href(base: Option<&str>, raw: &str) -> String {
    let raw_trimmed = raw.trim();
    let Some(base) = base.map(str::trim).filter(|b| !b.is_empty()) else {
        return raw_trimmed.to_string();
    };
    if starts_with_ignore_ascii_case(raw_trimmed, "http://")
        || starts_with_ignore_ascii_case(raw_trimmed, "https://")
        || starts_with_ignore_ascii_case(raw_trimmed, "data:")
        || starts_with_ignore_ascii_case(raw_trimmed, "file:")
    {
        return raw_trimmed.to_string();
    }

    let base_is_url = starts_with_ignore_ascii_case(base, "http://")
        || starts_with_ignore_ascii_case(base, "https://");
    if !base_is_url {
        // ローカルパスの基準ディレクトリとして前置する。root-relativeな参照は
        // `resolve_local_asset_path`が`base_dir`のルートとして解決するので触らない。
        if raw_trimmed.starts_with('/') {
            return raw_trimmed.to_string();
        }
        let base_dir = base.trim_end_matches('/');
        if base_dir.is_empty() {
            return raw_trimmed.to_string();
        }
        return format!("{base_dir}/{raw_trimmed}");
    }

    // プロトコル相対(`//example.com/x`)。
    if let Some(rest) = raw_trimmed.strip_prefix("//") {
        let scheme = base.split(':').next().unwrap_or("https");
        return format!("{scheme}://{rest}");
    }
    // ルート相対(`/x`)は基準URLのオリジンに対して解決する。
    let scheme_end = base.find("://").map(|i| i + 3).unwrap_or(0);
    if raw_trimmed.starts_with('/') {
        let origin_end = base[scheme_end..]
            .find('/')
            .map(|i| scheme_end + i)
            .unwrap_or(base.len());
        return format!("{}{raw_trimmed}", &base[..origin_end]);
    }
    // それ以外は基準URLの「最後の`/`まで」に連結する。
    let dir_end = base[scheme_end..]
        .rfind('/')
        .map(|i| scheme_end + i + 1)
        .unwrap_or(base.len());
    let mut resolved = base[..dir_end].to_string();
    if !resolved.ends_with('/') {
        resolved.push('/');
    }
    resolved.push_str(raw_trimmed);
    resolved
}

/// `src`属性の値を分類する。デコード不能な`data:`URI・`file:`スキームなど
/// 「そもそも取得を試みるべきでない」値は`None`を返す
/// (呼び出し側は[0014](../../../docs/decisions/0014-image-streaming-and-fallback.md)
/// の方針通り、画像なしの置換要素として扱う)。
pub fn classify_img_src(src: &str) -> Option<ImgSrc> {
    let trimmed = src.trim();

    if let Some(rest) = strip_prefix_ignore_ascii_case(trimmed, "data:") {
        return parse_data_uri(rest);
    }
    if starts_with_ignore_ascii_case(trimmed, "http://")
        || starts_with_ignore_ascii_case(trimmed, "https://")
    {
        return Some(ImgSrc::RemoteUrl(trimmed.to_string()));
    }
    if starts_with_ignore_ascii_case(trimmed, "file:") {
        return None;
    }

    Some(ImgSrc::LocalPath(trimmed.to_string()))
}

fn starts_with_ignore_ascii_case(value: &str, prefix: &str) -> bool {
    value.len() >= prefix.len()
        && value.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes())
}

fn strip_prefix_ignore_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    starts_with_ignore_ascii_case(value, prefix).then(|| &value[prefix.len()..])
}

/// `data:`の直後(`data:`自体は含まない)を`[<mediatype>][;base64],<data>`
/// として解釈する(RFC 2397)。
fn parse_data_uri(rest: &str) -> Option<ImgSrc> {
    let (meta, data) = rest.split_once(',')?;

    let mut mime_type = String::new();
    let mut is_base64 = false;
    for (i, segment) in meta.split(';').enumerate() {
        if i == 0 {
            mime_type = segment.to_string();
        } else if segment.eq_ignore_ascii_case("base64") {
            is_base64 = true;
        }
        // charset等それ以外のパラメータは画像埋め込みには関係ないため無視する。
    }
    if !is_base64 {
        return None;
    }

    // base64ペイロード中に改行等の空白が挟まれるケースを許容するため、
    // デコード前に取り除く。パディングの有無(`=`)もどちらも受け付ける。
    let cleaned: Vec<u8> = data.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let bytes = lenient_base64().decode(cleaned).ok()?;

    Some(ImgSrc::DataUri { mime_type, bytes })
}

/// パディングあり/なしのどちらも受け付ける標準base64デコーダ。
fn lenient_base64() -> GeneralPurpose {
    GeneralPurpose::new(
        &BASE64_STANDARD_ALPHABET,
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
    )
}

/// [`ImgSrc::LocalPath`](や`@font-face`の`url()`・`<link href>`等、同じ
/// 性質を持つ他のローカル資産参照)を`base_dir`基準で実際のファイルパスへ
/// 解決する。
///
/// `raw`の先頭が`/`(root-relative、`<link href="/stylesheets/main.css" />`
/// のようなRailsのアセットパイプラインでよくある書き方)の場合、これを
/// "サイトルート"の意味と解釈し`base_dir`相対として扱う。素朴に
/// `base_dir.join(raw)`すると、`Path::join`は引数が絶対パスの場合
/// (Unix)`base_dir`を丸ごと捨ててしまい、OSのファイルシステムルートを
/// 読みに行ってしまう(意図しない・環境依存の挙動)ため、先頭の`/`を
/// 明示的に取り除いてから結合する。
///
/// `..`によるディレクトリトラバーサルの制限は行わない(`base_dir.join`を
/// 使う既存のローカルパス解決全般と対称的な、意図的な簡略化。
/// `core/src/img/fetch.rs`の`read_local`のドキュメント参照)。
pub fn resolve_local_asset_path(base_dir: &Path, raw: &str) -> PathBuf {
    base_dir.join(raw.trim_start_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_a_bare_relative_path_as_local() {
        assert_eq!(
            classify_img_src("logo.png"),
            Some(ImgSrc::LocalPath("logo.png".to_string()))
        );
    }

    #[test]
    fn classifies_dot_relative_and_absolute_local_paths() {
        assert_eq!(
            classify_img_src("./assets/logo.png"),
            Some(ImgSrc::LocalPath("./assets/logo.png".to_string()))
        );
        assert_eq!(
            classify_img_src("../images/logo.png"),
            Some(ImgSrc::LocalPath("../images/logo.png".to_string()))
        );
        assert_eq!(
            classify_img_src("/var/www/images/logo.png"),
            Some(ImgSrc::LocalPath("/var/www/images/logo.png".to_string()))
        );
    }

    #[test]
    fn classifies_http_and_https_urls_as_remote() {
        assert_eq!(
            classify_img_src("http://example.com/x.png"),
            Some(ImgSrc::RemoteUrl("http://example.com/x.png".to_string()))
        );
        assert_eq!(
            classify_img_src("HTTPS://example.com/x.png"),
            Some(ImgSrc::RemoteUrl("HTTPS://example.com/x.png".to_string())),
            "scheme comparison should be case-insensitive"
        );
    }

    #[test]
    fn rejects_the_file_scheme() {
        assert_eq!(classify_img_src("file:///etc/passwd"), None);
        assert_eq!(classify_img_src("FILE:///etc/passwd"), None);
    }

    #[test]
    fn decodes_a_base64_data_uri() {
        // "hi"のbase64表現。
        let src = "data:image/png;base64,aGk=";
        assert_eq!(
            classify_img_src(src),
            Some(ImgSrc::DataUri {
                mime_type: "image/png".to_string(),
                bytes: b"hi".to_vec(),
            })
        );
    }

    #[test]
    fn decodes_a_data_uri_missing_padding() {
        let src = "data:image/png;base64,aGk";
        assert_eq!(
            classify_img_src(src),
            Some(ImgSrc::DataUri {
                mime_type: "image/png".to_string(),
                bytes: b"hi".to_vec(),
            })
        );
    }

    #[test]
    fn ignores_whitespace_inside_a_data_uri_payload() {
        let src = "data:image/png;base64,\n  aGk=\n";
        assert_eq!(
            classify_img_src(src),
            Some(ImgSrc::DataUri {
                mime_type: "image/png".to_string(),
                bytes: b"hi".to_vec(),
            })
        );
    }

    #[test]
    fn rejects_a_non_base64_data_uri() {
        // パーセントエンコードされたプレーンテキストのdata:URIは非対応。
        assert_eq!(classify_img_src("data:text/plain,Hello%20World"), None);
    }

    #[test]
    fn rejects_a_data_uri_with_invalid_base64_payload() {
        assert_eq!(
            classify_img_src("data:image/png;base64,not-valid-base64!!!"),
            None
        );
    }

    #[test]
    fn rejects_a_data_uri_missing_a_comma() {
        assert_eq!(classify_img_src("data:image/png;base64"), None);
    }

    #[test]
    fn resolve_local_asset_path_joins_a_plain_relative_path() {
        let resolved = resolve_local_asset_path(Path::new("/var/www/app"), "logo.png");
        assert_eq!(resolved, Path::new("/var/www/app/logo.png"));
    }

    #[test]
    fn resolve_local_asset_path_treats_a_leading_slash_as_relative_to_base_dir() {
        // 素朴な`base_dir.join(raw)`だと、Path::joinは絶対パスを渡されると
        // base_dirを丸ごと捨ててしまう(Unix)。root-relativeなhref
        // (CLAUDE.mdの`<link href="/stylesheets/main.css" />`の例)が
        // base_dirの外(OSのファイルシステムルート)へ逃げないことを確認する。
        let resolved = resolve_local_asset_path(Path::new("/var/www/app"), "/stylesheets/main.css");
        assert_eq!(
            resolved,
            Path::new("/var/www/app/stylesheets/main.css"),
            "a root-relative href must stay inside base_dir, not escape to the OS filesystem root"
        );
    }

    #[test]
    fn resolve_local_asset_path_strips_multiple_leading_slashes() {
        let resolved = resolve_local_asset_path(Path::new("/var/www/app"), "//evil.example/x");
        assert_eq!(resolved, Path::new("/var/www/app/evil.example/x"));
    }

    #[test]
    fn resolve_local_asset_path_leaves_dot_relative_paths_unchanged() {
        let resolved = resolve_local_asset_path(Path::new("/var/www/app"), "./assets/x.css");
        assert_eq!(resolved, Path::new("/var/www/app/./assets/x.css"));
    }

    // ===== `<base href>`([0040]) =====

    #[test]
    fn base_href_is_ignored_for_absolute_references() {
        for raw in [
            "https://cdn.example.com/a.png",
            "http://cdn.example.com/a.png",
            "data:image/png;base64,AAA",
        ] {
            assert_eq!(
                resolve_against_base_href(Some("https://example.com/docs/"), raw),
                raw
            );
        }
    }

    #[test]
    fn no_base_href_leaves_the_reference_untouched() {
        assert_eq!(resolve_against_base_href(None, "img/a.png"), "img/a.png");
        assert_eq!(
            resolve_against_base_href(Some("   "), "img/a.png"),
            "img/a.png"
        );
    }

    #[test]
    fn a_url_base_resolves_relative_references_against_its_directory() {
        assert_eq!(
            resolve_against_base_href(Some("https://example.com/docs/index.html"), "img/a.png"),
            "https://example.com/docs/img/a.png"
        );
        assert_eq!(
            resolve_against_base_href(Some("https://example.com/docs/"), "a.png"),
            "https://example.com/docs/a.png"
        );
        // 末尾に`/`が無い基準はディレクトリとみなせる部分までを使う。
        assert_eq!(
            resolve_against_base_href(Some("https://example.com/docs"), "a.png"),
            "https://example.com/a.png"
        );
    }

    #[test]
    fn a_url_base_resolves_root_relative_and_protocol_relative_references() {
        assert_eq!(
            resolve_against_base_href(Some("https://example.com/docs/index.html"), "/a.png"),
            "https://example.com/a.png"
        );
        assert_eq!(
            resolve_against_base_href(Some("https://example.com/docs/"), "//cdn.example.net/a.png"),
            "https://cdn.example.net/a.png"
        );
    }

    #[test]
    fn a_path_base_is_prepended_as_a_directory() {
        assert_eq!(
            resolve_against_base_href(Some("assets/"), "img/a.png"),
            "assets/img/a.png"
        );
        assert_eq!(
            resolve_against_base_href(Some("assets"), "a.png"),
            "assets/a.png"
        );
        // root-relativeな参照は基準ディレクトリを前置しない
        // (`resolve_local_asset_path`が`base_dir`のルートとして解決するため)。
        assert_eq!(
            resolve_against_base_href(Some("assets/"), "/a.png"),
            "/a.png"
        );
    }

    #[test]
    fn a_resolved_relative_reference_classifies_as_a_remote_url() {
        let resolved = resolve_against_base_href(Some("https://example.com/docs/"), "a.png");
        assert_eq!(
            classify_img_src(&resolved),
            Some(ImgSrc::RemoteUrl(
                "https://example.com/docs/a.png".to_string()
            ))
        );
    }
}
