//! 入力HTMLの文字エンコーディング判定とデコード(M12 T299)。
//!
//! sghtmltopdfの内部表現はUTF-8だが、入力が常にUTF-8とは限らない
//! (M10 Phase 5の積み残し)。優先順位は
//! **BOM > `--encoding`の明示 > `<meta charset>` > UTF-8** とする。
//! BOMを最優先にするのはHTML Standardのsniffing手順に合わせるため。
//!
//! ストリーミング入力ではチャンク境界がマルチバイト文字を割りうるため、
//! ここでは**入力全体が揃っている前提**のAPIだけを提供する
//! (CLIは一括読み込み。HTTPサーバモードでUTF-8以外を扱う場合は、
//! ボディを読み切ってから通すこと)。

use encoding_rs::Encoding;

/// `<meta charset>`を探す範囲。HTML Standardのprescan相当。
const PRESCAN_LIMIT: usize = 1024;

/// 入力バイト列をUTF-8文字列へデコードする。
///
/// `declared`は`--encoding`で明示された名前(`Shift_JIS`など)。
/// 未知のラベルはエラーにする(黙ってUTF-8扱いにしない)。
pub fn decode_html(bytes: &[u8], declared: Option<&str>) -> Result<String, String> {
    if let Some((encoding, bom_len)) = Encoding::for_bom(bytes) {
        let (text, _, _) = encoding.decode(&bytes[bom_len..]);
        return Ok(text.into_owned());
    }

    let encoding = match declared {
        Some(label) => Encoding::for_label(label.as_bytes())
            .ok_or_else(|| format!("未知のエンコーディングです: {label}"))?,
        None => match sniff_meta_charset(bytes) {
            Some(encoding) => encoding,
            None => encoding_rs::UTF_8,
        },
    };

    let (text, _, _) = encoding.decode(bytes);
    Ok(text.into_owned())
}

/// 先頭1KBから`<meta charset=...>`または
/// `<meta http-equiv="Content-Type" content="...; charset=...">`を探す。
fn sniff_meta_charset(bytes: &[u8]) -> Option<&'static Encoding> {
    let limit = bytes.len().min(PRESCAN_LIMIT);
    let head = String::from_utf8_lossy(&bytes[..limit]).to_ascii_lowercase();

    let mut search_from = 0;
    while let Some(pos) = head[search_from..].find("charset") {
        let after = &head[search_from + pos + "charset".len()..];
        let value = after
            .trim_start()
            .strip_prefix('=')
            .map(|rest| rest.trim_start())
            .unwrap_or("");
        let value: String = value
            .trim_start_matches(['"', '\''])
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if !value.is_empty() {
            if let Some(encoding) = Encoding::for_label(value.as_bytes()) {
                return Some(encoding);
            }
        }
        search_from += pos + "charset".len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_input_passes_through() {
        let html = "<html><body><p>日本語</p></body></html>";
        assert_eq!(decode_html(html.as_bytes(), None).unwrap(), html);
    }

    #[test]
    fn an_explicit_encoding_is_used() {
        // Shift_JISの「日本語」。
        let bytes = b"\x93\xfa\x96\x7b\x8c\xea";
        assert_eq!(decode_html(bytes, Some("Shift_JIS")).unwrap(), "日本語");
        assert_eq!(decode_html(bytes, Some("sjis")).unwrap(), "日本語");
    }

    #[test]
    fn meta_charset_is_detected_when_no_encoding_is_given() {
        let mut bytes = b"<html><head><meta charset=\"shift_jis\"></head><body><p>".to_vec();
        bytes.extend_from_slice(b"\x93\xfa\x96\x7b\x8c\xea");
        bytes.extend_from_slice(b"</p></body></html>");
        let text = decode_html(&bytes, None).unwrap();
        assert!(text.contains("日本語"), "got: {text}");
    }

    #[test]
    fn http_equiv_content_type_is_also_detected() {
        let mut bytes =
            b"<html><head><meta http-equiv=\"Content-Type\" content=\"text/html; charset=euc-jp\">"
                .to_vec();
        // EUC-JPの「日本語」。
        bytes.extend_from_slice(b"</head><body><p>\xc6\xfc\xcb\xdc\xb8\xec</p></body></html>");
        let text = decode_html(&bytes, None).unwrap();
        assert!(text.contains("日本語"), "got: {text}");
    }

    #[test]
    fn an_explicit_encoding_wins_over_meta_charset() {
        let mut bytes = b"<html><head><meta charset=\"utf-8\"></head><body><p>".to_vec();
        bytes.extend_from_slice(b"\x93\xfa\x96\x7b\x8c\xea");
        bytes.extend_from_slice(b"</p></body></html>");
        let text = decode_html(&bytes, Some("Shift_JIS")).unwrap();
        assert!(text.contains("日本語"), "got: {text}");
    }

    #[test]
    fn a_bom_wins_over_everything() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("日本語".as_bytes());
        // BOMがUTF-8を示すので、--encodingの指定より優先される。
        assert_eq!(decode_html(&bytes, Some("Shift_JIS")).unwrap(), "日本語");
    }

    #[test]
    fn an_unknown_label_is_an_error() {
        assert!(decode_html(b"x", Some("no-such-encoding")).is_err());
    }
}
