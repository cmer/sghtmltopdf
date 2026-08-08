//! 入力HTMLの文字エンコーディング判定とデコード。
//!
//! sghtmltopdfの内部表現はUTF-8だが、入力が常にUTF-8とは限らない。
//! 優先順位は
//! BOM > `--encoding`の明示 > `<meta charset>` > UTF-8 とする。
//! BOMを最優先にするのはHTML Standardのsniffing手順に合わせるため。
//!
//! 入力全体が揃っている場合は[`decode_html`]を、読みながら処理する場合は
//! [`StreamingDecoder`]を使う。後者は`encoding_rs`のインクリメンタル
//! デコーダを持ち、チャンク境界がマルチバイト文字を割っても正しく
//! 復元する(HTTPサーバがボディを読みながら`Engine::feed`へ渡す経路で使う)。

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

/// 読みながらUTF-8へ変換していくデコーダ。
///
/// エンコーディングの判定には先頭の一定バイト([`PRESCAN_LIMIT`])が要るため、
/// 確定するまでは内部にバッファし、確定後は`encoding_rs`の
/// インクリメンタルデコーダへ流す。チャンク境界がマルチバイト文字の途中でも、
/// デコーダが持ち越すので壊れない。
pub struct StreamingDecoder {
    /// `--encoding`で明示された値(確定済み)。
    declared: Option<&'static Encoding>,
    state: State,
}

enum State {
    /// エンコーディング確定待ち。溜めたバイト列を持つ。
    Buffering(Vec<u8>),
    Decoding(encoding_rs::Decoder),
}

impl StreamingDecoder {
    /// `declared`は`--encoding`の値。未知のラベルはここでエラーにする。
    pub fn new(declared: Option<&str>) -> Result<Self, String> {
        let declared = match declared {
            Some(label) => Some(
                Encoding::for_label(label.as_bytes())
                    .ok_or_else(|| format!("未知のエンコーディングです: {label}"))?,
            ),
            None => None,
        };
        Ok(Self {
            declared,
            state: State::Buffering(Vec::new()),
        })
    }

    /// チャンクを与え、確定できた分のUTF-8文字列を返す。
    pub fn push(&mut self, chunk: &[u8]) -> String {
        match &mut self.state {
            State::Buffering(buffer) => {
                buffer.extend_from_slice(chunk);
                if buffer.len() < PRESCAN_LIMIT {
                    return String::new();
                }
                self.settle()
            }
            State::Decoding(decoder) => decode_chunk(decoder, chunk, false),
        }
    }

    /// 入力の終わり。残りを吐き出す。
    pub fn finish(&mut self) -> String {
        // 入力が[`PRESCAN_LIMIT`]に満たなかった場合、ここで初めて確定する。
        let mut out = if matches!(self.state, State::Buffering(_)) {
            self.settle()
        } else {
            String::new()
        };
        if let State::Decoding(decoder) = &mut self.state {
            out.push_str(&decode_chunk(decoder, &[], true));
        }
        out
    }

    /// 溜めたバッファからエンコーディングを決め、デコーダへ移行する。
    fn settle(&mut self) -> String {
        let State::Buffering(buffer) = &mut self.state else {
            return String::new();
        };
        let buffer = std::mem::take(buffer);

        // BOMがあればそれが最優先(`new_decoder`のBOM sniffingが処理する)。
        // 次に`--encoding`、次に`<meta charset>`、最後にUTF-8。
        let encoding = self
            .declared
            .or_else(|| sniff_meta_charset(&buffer))
            .unwrap_or(encoding_rs::UTF_8);
        let mut decoder = encoding.new_decoder();
        let out = decode_chunk(&mut decoder, &buffer, false);
        self.state = State::Decoding(decoder);
        out
    }
}

/// 1チャンク分をUTF-8へ変換する。出力バッファは`max_utf8_buffer_length`で
/// 十分な容量を先に確保するため、`OutputFull`のループは要らない。
fn decode_chunk(decoder: &mut encoding_rs::Decoder, input: &[u8], last: bool) -> String {
    let capacity = decoder
        .max_utf8_buffer_length(input.len())
        .unwrap_or(input.len().saturating_mul(3) + 4);
    let mut out = String::with_capacity(capacity);
    let (_result, _read, _had_errors) = decoder.decode_to_string(input, &mut out, last);
    out
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

    /// チャンクに分けて食わせ、連結した結果を返す。
    fn stream(chunks: &[&[u8]], declared: Option<&str>) -> String {
        let mut decoder = StreamingDecoder::new(declared).unwrap();
        let mut out = String::new();
        for chunk in chunks {
            out.push_str(&decoder.push(chunk));
        }
        out.push_str(&decoder.finish());
        out
    }

    #[test]
    fn streaming_decoder_handles_a_split_multibyte_character() {
        // 「日本語」のUTF-8を文字の途中で割る。
        let bytes = "日本語".as_bytes();
        let out = stream(&[&bytes[..4], &bytes[4..]], None);
        assert_eq!(out, "日本語");
    }

    #[test]
    fn streaming_decoder_handles_a_split_shift_jis_character() {
        // Shift_JISの「日本語」(2バイト×3)を1バイト目と2バイト目で割る。
        let bytes: &[u8] = b"\x93\xfa\x96\x7b\x8c\xea";
        let out = stream(&[&bytes[..1], &bytes[1..3], &bytes[3..]], Some("Shift_JIS"));
        assert_eq!(out, "日本語");
    }

    #[test]
    fn streaming_decoder_detects_meta_charset_after_buffering() {
        // prescan分を超える長さにして、確定が走る経路を通す。
        let mut html = b"<html><head><meta charset=\"shift_jis\"></head><body>".to_vec();
        html.extend(std::iter::repeat_n(b'x', PRESCAN_LIMIT));
        html.extend_from_slice(b"<p>\x93\xfa\x96\x7b\x8c\xea</p></body></html>");

        let chunks: Vec<&[u8]> = html.chunks(97).collect();
        let out = stream(&chunks, None);
        assert!(out.contains("日本語"), "got: {out}");
    }

    #[test]
    fn streaming_decoder_flushes_short_input_on_finish() {
        // PRESCAN_LIMITに満たない入力は、finishで初めて確定して出てくる。
        let mut decoder = StreamingDecoder::new(None).unwrap();
        assert_eq!(decoder.push(b"<p>short</p>"), "");
        assert_eq!(decoder.finish(), "<p>short</p>");
    }

    #[test]
    fn streaming_decoder_lets_a_bom_win() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("日本語".as_bytes());
        assert_eq!(stream(&[&bytes], Some("Shift_JIS")), "日本語");
    }

    #[test]
    fn streaming_decoder_rejects_an_unknown_label() {
        assert!(StreamingDecoder::new(Some("no-such-encoding")).is_err());
    }

    #[test]
    fn an_unknown_label_is_an_error() {
        assert!(decode_html(b"x", Some("no-such-encoding")).is_err());
    }
}
