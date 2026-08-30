//! `<img src>`のURL/パス分類。

use std::path::{Component, Path, PathBuf};

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
    /// `file:`は明示的に別扱い(拒否)のため、ここには来ない(「ローカル相対
    /// パスとURLスキームを取り違えさせない」方針通り)。
    LocalPath(String),
    /// `http`/`https`の絶対URL。実際のフェッチは
    /// フェッチャがセキュリティポリシーに従って行う。
    RemoteUrl(String),
    /// `data:`URI。`;base64`が付いていればbase64として、付いていなければ
    /// パーセントエンコードとしてデコードする(RFC 2397)。
    ///
    /// 非base64のペイロードはラスタ画像ではまず使われないが、SVGでは
    /// `data:image/svg+xml,%3Csvg...%3E`が最も一般的な書き方なので、
    /// どちらも受ける。デコードした結果が画像として読めるかは
    /// `pdf::img::decode_image`が中身のバイト列を見て判断する。
    DataUri { mime_type: String, bytes: Vec<u8> },
}

/// `raw`(生の参照値)を`<base href>`に対して解決する。
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
/// 「そもそも取得を試みるべきでない」値は`None`を返す(呼び出し側は画像なしの置換要素として扱う)。
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

/// `data:`の直後(`data:`自体は含まない)を`[<mediatype>][;base64],<data>`として解釈する(RFC 2397)。
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
    let bytes = if is_base64 {
        // base64ペイロード中に改行等の空白が挟まれるケースを許容するため、
        // デコード前に取り除く。パディングの有無(`=`)もどちらも受け付ける。
        let cleaned: Vec<u8> = data.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
        lenient_base64().decode(cleaned).ok()?
    } else {
        percent_decode(data)
    };

    Some(ImgSrc::DataUri { mime_type, bytes })
}

/// パーセントエンコード(`%XX`)を解く。`%XX`の形でないものはそのまま通す。
///
/// タブ・改行だけは取り除く(URL標準がURLの解析前にこの2つを落とすため。
/// HTMLの属性やCSSの`url()`の中で折り返して書かれた`data:`URIを、余計な
/// 制御文字を混ぜずに受けられるようにする)。**空白は残す**。
/// エンコードされていないSVG(`data:image/svg+xml,<svg ...>`)では
/// タグの区切りとして意味を持つため。
fn percent_decode(data: &str) -> Vec<u8> {
    let bytes = data.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\t' | b'\r' | b'\n' => i += 1,
            // バイト単位で16進2桁を読む。`data`はUTF-8なので`%`の後ろが
            // マルチバイト文字の途中でも、バイトで見ている限り安全
            // (16進として読めなければリテラルの`%`として通す)。
            b'%' if i + 3 <= bytes.len() => {
                match (hex_value(bytes[i + 1]), hex_value(bytes[i + 2])) {
                    (Some(hi), Some(lo)) => {
                        out.push(hi << 4 | lo);
                        i += 3;
                    }
                    _ => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    out
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
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
/// 解決する。`base_dir`の外へ出る参照は`None`を返す。
///
/// `raw`の先頭が`/`(root-relative、`<link href="/stylesheets/main.css" />`
/// のようなRailsのアセットパイプラインでよくある書き方)の場合、これを
/// "サイトルート"の意味と解釈し`base_dir`相対として扱う。素朴に
/// `base_dir.join(raw)`すると、`Path::join`は引数が絶対パスの場合
/// (Unix)`base_dir`を丸ごと捨ててしまい、OSのファイルシステムルートを
/// 読みに行ってしまう(意図しない・環境依存の挙動)ため、先頭の`/`を
/// 明示的に取り除いてから結合する。
///
/// # `..`の扱い
///
/// `base_dir`をルートとみなし、そこから出る`..`は拒否する。信頼できない
/// HTMLを変換したときに、`<img src="../../../../etc/passwd">`のような参照で
/// base_dirの外を読み出せてしまうため。意図して外を参照したい場合は
/// `--allow`で範囲を明示する。
///
/// 判定は字句的に行い、ファイルシステムには触れない(存在しないパスでも
/// 同じように判定できるようにするため)。したがってbase_dir配下の
/// シンボリックリンクは辿る。Capistranoの`public/system`のような運用を
/// 壊さないための意図的な線引きで、シンボリックリンクまで含めて閉じたい
/// 場合は`--allow`(実パスで判定する)を使う。
pub fn resolve_local_asset_path(base_dir: &Path, raw: &str) -> ResolvedAssetPath {
    let mut parts = Vec::new();
    // base_dirより上へ出た段数。`--allow`が無ければこれが1以上で拒否になる。
    let mut up = 0usize;
    let mut absolute = false;

    // 先頭の`/`を落として"サイトルート相対"にしてから、`.`/`..`を畳む。
    for component in Path::new(raw.trim_start_matches('/')).components() {
        match component {
            Component::Normal(part) => parts.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                // 畳める要素が無ければbase_dirより上へ出たということ。
                if parts.pop().is_none() {
                    up += 1;
                }
            }
            // 絶対パスの目印(`/`やWindowsの`C:`)。先頭の`/`は落としてあるので、
            // ここへ来るのは`raw`が別の絶対パス表記だった場合。
            Component::RootDir | Component::Prefix(_) => absolute = true,
        }
    }

    let mut path = base_dir.to_path_buf();
    for _ in 0..up {
        path.push("..");
    }
    path.extend(&parts);

    ResolvedAssetPath {
        path,
        escapes_base_dir: up > 0 || absolute,
    }
}

/// [`resolve_local_asset_path`]の結果。
pub struct ResolvedAssetPath {
    /// 解決後のパス。
    pub path: PathBuf,
    /// `..`等で`base_dir`の外へ出ているか。
    /// 既定ではこれが`true`の参照を拒否する。`--allow`が指定されている場合は
    /// そちらが範囲を決めるので、この値ではなく許可ディレクトリで判定する。
    pub escapes_base_dir: bool,
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

    /// SVGは`data:image/svg+xml,%3Csvg...%3E`(base64でない)が一般的な
    /// 書き方なので、パーセントエンコードのペイロードも受ける。
    #[test]
    fn decodes_a_percent_encoded_data_uri() {
        assert_eq!(
            classify_img_src("data:text/plain,Hello%20World"),
            Some(ImgSrc::DataUri {
                mime_type: "text/plain".to_string(),
                bytes: b"Hello World".to_vec(),
            })
        );
    }

    #[test]
    fn decodes_a_percent_encoded_svg_data_uri() {
        let src =
            "data:image/svg+xml,%3Csvg%20xmlns%3D%22http%3A%2F%2Fwww.w3.org%2F2000%2Fsvg%22%2F%3E";
        assert_eq!(
            classify_img_src(src),
            Some(ImgSrc::DataUri {
                mime_type: "image/svg+xml".to_string(),
                bytes: br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#.to_vec(),
            })
        );
    }

    /// エンコードされていないペイロードもそのまま通す(`;utf8,`のような
    /// 慣習的なパラメータ付きも含む)。空白は意味を持つので落とさない。
    #[test]
    fn passes_through_an_unencoded_data_uri_payload() {
        assert_eq!(
            classify_img_src("data:image/svg+xml;utf8,<svg id='a b'/>"),
            Some(ImgSrc::DataUri {
                mime_type: "image/svg+xml".to_string(),
                bytes: b"<svg id='a b'/>".to_vec(),
            })
        );
    }

    /// 折り返して書かれたdata:URIのタブ・改行は落とす(URL標準と同じ)。
    #[test]
    fn strips_tabs_and_newlines_but_not_spaces_from_a_percent_encoded_payload() {
        assert_eq!(
            classify_img_src("data:image/svg+xml,%3Csvg\n\t id='a b'%2F%3E"),
            Some(ImgSrc::DataUri {
                mime_type: "image/svg+xml".to_string(),
                bytes: b"<svg id='a b'/>".to_vec(),
            })
        );
    }

    /// `%`が16進2桁の形になっていなければリテラルの`%`として通す
    /// (壊れたエンコードでSVG全体を捨てない)。
    #[test]
    fn a_stray_percent_is_kept_verbatim() {
        assert_eq!(
            classify_img_src("data:text/plain,100% and %zz and %4"),
            Some(ImgSrc::DataUri {
                mime_type: "text/plain".to_string(),
                bytes: b"100% and %zz and %4".to_vec(),
            })
        );
    }

    /// パーセントエンコードは非ASCIIバイトも復元できる(UTF-8の途中で
    /// 切らずにバイト単位で読んでいることの確認)。
    #[test]
    fn decodes_percent_escapes_of_non_ascii_bytes() {
        // "あ" = E3 81 82
        assert_eq!(
            classify_img_src("data:text/plain,%E3%81%82"),
            Some(ImgSrc::DataUri {
                mime_type: "text/plain".to_string(),
                bytes: "あ".as_bytes().to_vec(),
            })
        );
        // エンコードされていない非ASCIIもそのまま通る。
        assert_eq!(
            classify_img_src("data:text/plain,あ%20い"),
            Some(ImgSrc::DataUri {
                mime_type: "text/plain".to_string(),
                bytes: "あ い".as_bytes().to_vec(),
            })
        );
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

    /// base_dir配下に収まる参照だけを取り出すヘルパ(封じ込めの確認込み)。
    fn within(base: &str, raw: &str) -> Option<PathBuf> {
        let resolved = resolve_local_asset_path(Path::new(base), raw);
        (!resolved.escapes_base_dir).then_some(resolved.path)
    }

    #[test]
    fn resolve_local_asset_path_joins_a_plain_relative_path() {
        assert_eq!(
            within("/var/www/app", "logo.png"),
            Some(PathBuf::from("/var/www/app/logo.png"))
        );
    }

    #[test]
    fn a_parent_reference_that_escapes_base_dir_is_flagged() {
        // 信頼できないHTMLからの`<img src="../../../../etc/passwd">`。
        let resolved =
            resolve_local_asset_path(Path::new("/var/www/app"), "../../../../etc/passwd");
        assert!(
            resolved.escapes_base_dir,
            "base_dirの外へ出る参照は印が付くべき"
        );
        // `--allow`が指定されたときの判定に使えるよう、パス自体は素直に解決する。
        assert_eq!(
            resolved.path,
            Path::new("/var/www/app/../../../../etc/passwd")
        );
    }

    #[test]
    fn a_parent_reference_that_stays_inside_base_dir_is_allowed() {
        // `assets/../images/x.png`はbase_dirの中で完結するので許す。
        assert_eq!(
            within("/var/www/app", "assets/../images/x.png"),
            Some(PathBuf::from("/var/www/app/images/x.png"))
        );
    }

    #[test]
    fn stacked_parent_references_are_counted_correctly() {
        // `..`が2段続いても1段ぶんに畳まれない(数え落とすと素通りする)。
        let resolved = resolve_local_asset_path(Path::new("/var/www/app"), "../../etc/passwd");
        assert!(resolved.escapes_base_dir);
        assert_eq!(resolved.path, Path::new("/var/www/app/../../etc/passwd"));
    }

    #[test]
    fn a_parent_reference_after_descending_only_escapes_when_it_goes_too_far() {
        // a/../.. は1段ぶん外に出る。
        let resolved = resolve_local_asset_path(Path::new("/var/www/app"), "a/../../x");
        assert!(resolved.escapes_base_dir);
        assert_eq!(resolved.path, Path::new("/var/www/app/../x"));
    }

    #[test]
    fn resolve_local_asset_path_treats_a_leading_slash_as_relative_to_base_dir() {
        // 素朴な`base_dir.join(raw)`だと、Path::joinは絶対パスを渡されると
        // base_dirを丸ごと捨ててしまう(Unix)。root-relativeなhref
        // (`<link href="/stylesheets/main.css" />`の例)が
        // base_dirの外(OSのファイルシステムルート)へ逃げないことを確認する。
        assert_eq!(
            within("/var/www/app", "/stylesheets/main.css"),
            Some(PathBuf::from("/var/www/app/stylesheets/main.css")),
            "a root-relative href must stay inside base_dir, not escape to the OS filesystem root"
        );
    }

    #[test]
    fn resolve_local_asset_path_strips_multiple_leading_slashes() {
        assert_eq!(
            within("/var/www/app", "//evil.example/x"),
            Some(PathBuf::from("/var/www/app/evil.example/x"))
        );
    }

    #[test]
    fn resolve_local_asset_path_normalizes_dot_relative_paths() {
        // `.`は畳まれる
        assert_eq!(
            within("/var/www/app", "./assets/x.css"),
            Some(PathBuf::from("/var/www/app/assets/x.css"))
        );
    }

    // ===== `<base href>` =====

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
