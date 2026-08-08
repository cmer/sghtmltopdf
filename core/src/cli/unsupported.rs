//! wkhtmltopdfにあってsghtmltopdfが実装しないオプションは拒否する。

/// 非対応の理由(同じ理由のオプションでまとめる)。
struct Reason {
    message: &'static str,
    options: &'static [&'static str],
}

const REASONS: &[Reason] = &[
    Reason {
        message: "sghtmltopdfはJavaScriptを実行しません(設計上の非目標)。\n  \
                  動的に生成される内容は、呼び出し側でHTMLを組み立ててから渡してください",
        options: &[
            "--enable-javascript",
            "--disable-javascript",
            "--javascript-delay",
            "--run-script",
            "--window-status",
            "--debug-javascript",
            "--no-debug-javascript",
            "--stop-slow-scripts",
            "--no-stop-slow-scripts",
            "--enable-plugins",
            "--disable-plugins",
        ],
    },
    Reason {
        message: "PDFのアウトライン(ブックマーク)は未対応です。\n  \
                  文書内の見出し一覧が必要な場合は --toc で目次ページを作れます",
        options: &[
            "--outline",
            "--no-outline",
            "--outline-depth",
            "--dump-outline",
            "--exclude-from-outline",
            "--include-in-outline",
        ],
    },
    Reason {
        message: "XSLTには対応していません。\n  \
                  目次の見た目は --toc-header-text 等のオプションと、\n  \
                  --user-style-sheet で渡すCSSで変えられます",
        options: &["--xsl-style-sheet", "--dump-default-toc-xsl"],
    },
    Reason {
        message: "画像の再エンコード・縮小には対応していません\n  \
                  (JPEGはデコードせずそのまま埋め込む方式のため)。\n  \
                  必要な場合は、渡す前に画像側を縮小してください",
        options: &["--image-quality", "--image-dpi"],
    },
    Reason {
        message: "認証・プロキシ付きの取得には対応していません。\n  \
                  取得が必要なリソースは、呼び出し側で取得してから\n  \
                  ローカルパスまたはdata:URIで渡してください",
        options: &[
            "--proxy",
            "--proxy-hostname-lookup",
            "--bypass-proxy-for",
            "--cookie",
            "--cookie-jar",
            "--custom-header",
            "--custom-header-propagation",
            "--no-custom-header-propagation",
            "--username",
            "--password",
            "--ssl-crt-path",
            "--ssl-key-path",
            "--ssl-key-password",
            "--post",
            "--post-file",
        ],
    },
    Reason {
        message: "WebKit固有の描画設定には対応していません\n  \
                  (sghtmltopdfは常に印刷メディアとしてレンダリングし、\n  \
                  ビューポートという概念を持ちません)",
        options: &[
            "--disable-smart-shrinking",
            "--enable-smart-shrinking",
            "--viewport-size",
            "--lowquality",
            "--print-media-type",
            "--no-print-media-type",
            "--use-xserver",
        ],
    },
    Reason {
        message: "PDFフォーム(AcroForm)の生成には対応していません。\n  \
                  フォーム要素は静的な見た目としてのみ描画されます",
        options: &["--enable-forms", "--disable-forms"],
    },
    Reason {
        message: "SVGを描画できないため、チェックボックス等の画像差し替えには\n  \
                  対応していません(内蔵の描画が使われます)",
        options: &[
            "--checkbox-svg",
            "--checkbox-checked-svg",
            "--radiobutton-svg",
            "--radiobutton-checked-svg",
        ],
    },
    Reason {
        message: "印刷部数の指定はPDF生成では意味を持たないため対応していません",
        options: &["--copies", "--collate", "--no-collate"],
    },
    Reason {
        message: "標準入力はHTMLの入力に使うため、引数の読み込みには使えません",
        options: &["--read-args-from-stdin"],
    },
    Reason {
        message: "取得結果のキャッシュは持ちません",
        options: &["--cache-dir"],
    },
    Reason {
        message: "ドキュメントとREADMEを参照してください",
        options: &[
            "--extended-help",
            "--htmldoc",
            "--manpage",
            "--readme",
            "--license",
        ],
    },
];

/// `name`(`--`付きのロングオプション名)が非対応なら、その理由を返す。
pub fn unsupported_reason(name: &str) -> Option<&'static str> {
    REASONS
        .iter()
        .find(|reason| reason.options.contains(&name))
        .map(|reason| reason.message)
}

/// コマンドライン引数に非対応オプションが含まれていればエラーメッセージを返す。
///
/// `--foo=bar`の形にも対応する。`--`より後ろは値として扱い、照合しない。
pub fn check_arguments(args: &[String]) -> Option<String> {
    for arg in args {
        if arg == "--" {
            break;
        }
        let name = arg.split('=').next().unwrap_or(arg);
        if let Some(reason) = unsupported_reason(name) {
            return Some(format!("{name} は対応していません。\n  {reason}"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn javascript_options_are_rejected_with_a_reason() {
        let message = check_arguments(&["--enable-javascript".to_string()]).unwrap();
        assert!(message.contains("--enable-javascript は対応していません"));
        assert!(message.contains("JavaScript"));
    }

    #[test]
    fn the_value_form_is_also_detected() {
        assert!(check_arguments(&["--outline-depth=3".to_string()]).is_some());
    }

    #[test]
    fn supported_options_pass_through() {
        assert!(check_arguments(&[
            "input.html".to_string(),
            "--page-size".to_string(),
            "A4".to_string(),
            "--toc".to_string(),
        ])
        .is_none());
    }

    #[test]
    fn arguments_after_a_double_dash_are_values() {
        assert!(check_arguments(&["--".to_string(), "--outline".to_string()]).is_none());
    }

    #[test]
    fn each_reason_mentions_an_alternative_or_the_cause() {
        // どの理由も「なぜ駄目か」を書いていること(空メッセージを防ぐ)。
        for reason in REASONS {
            assert!(reason.message.len() > 10);
            assert!(!reason.options.is_empty());
        }
    }

    #[test]
    fn the_query_side_can_reuse_the_same_table() {
        // HTTPサーバはクエリキー(`--`なし)を照合するため、`--`を足して引く。
        assert!(unsupported_reason("--xsl-style-sheet").is_some());
        assert!(unsupported_reason("--page-size").is_none());
    }
}
