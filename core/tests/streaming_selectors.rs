//! ストリーミングモードでのセレクタ挙動を、バッチモードと突き合わせて確かめる。
//!
//! `Mode::Streaming`は`<body>`直下のトップレベル要素を、次の兄弟が現れた時点で
//! 確定として処理する。そのため「その要素より後ろ」を見るセレクタは、確定した
//! 時点での部分的なDOMに対して判定されることになり、結果がバッチと変わりうる。
//!
//! どのセレクタが実際にずれるのかを固定するのがこのファイルの役割。
//! 入力は1要素ずつ`feed`する(CLIは64KiB単位で読むが、それはチャンクの切れ目が
//! たまたま合ったかどうかの問題でしかなく、エンジンの契約は「どう刻まれても
//! 同じ結果」であるべきなので、最も細かい刻み方で確かめる)。

use sghtmltopdf_core::engine::{Engine, EngineOptions, FontSpec, Mode};
use sghtmltopdf_core::sink::MemorySink;

const FONT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fonts/DejaVuSans.ttf");
/// マッチした要素に付ける色。PDFの塗り色オペレータとして数える。
const MARK_CSS: &str = "color: #cc0000";
const MARK_OP: &[u8] = b"0.8 0 0 rg";

fn options(mode: Mode) -> EngineOptions {
    EngineOptions {
        mode,
        fonts: vec![FontSpec {
            path: std::path::PathBuf::from(FONT_PATH),
            index: 0,
        }],
        output: sghtmltopdf_core::pdf::PdfOutputOptions {
            // 塗り色オペレータを数えるため圧縮しない。
            compress: false,
            ..Default::default()
        },
        ..EngineOptions::default()
    }
}

/// `selector`にマッチした要素の数を返す。`body`は`<body>`の中身。
fn matched_count(selector: &str, body: &str, mode: Mode) -> usize {
    let head = format!("<html><head><style>{selector} {{ {MARK_CSS} }}</style></head><body>");
    let mut engine = Engine::new(options(mode), MemorySink::new());
    engine.feed(head.as_bytes()).unwrap();
    // トップレベル要素を1つずつ食わせ、チャンクの切れ目に依存させない。
    for element in split_top_level(body) {
        engine.feed(element.as_bytes()).unwrap();
    }
    engine.feed(b"</body></html>").unwrap();
    let bytes = engine.finish().unwrap();

    bytes
        .windows(MARK_OP.len())
        .filter(|w| *w == MARK_OP)
        .count()
}

/// `<p>a</p><div>b</div>`のような並びを、トップレベル要素ごとに切る。
/// ネストは1段だけ想定(このテストが使う入力に限る)。
fn split_top_level(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    let mut rest = body;
    while let Some(open) = rest.find('<') {
        current.push_str(&rest[..open]);
        let close = rest[open..].find('>').expect("tag should be closed") + open;
        let tag = &rest[open..=close];
        current.push_str(tag);
        if tag.starts_with("</") {
            depth -= 1;
        } else if !tag.ends_with("/>") {
            depth += 1;
        }
        if depth == 0 {
            out.push(std::mem::take(&mut current));
        }
        rest = &rest[close + 1..];
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// バッチとストリーミングで結果が一致するセレクタ。
///
/// 直前の兄弟を見るもの(`+`/`~`/`:first-child`等)がここに居られるのは、
/// これらを使う文書ではトップレベル要素の解放を子孫だけに絞り、要素そのものを
/// 兄弟として残しているため(`style::needs_preceding_siblings`が判断する)。
#[test]
fn these_selectors_behave_the_same_in_both_modes() {
    // (セレクタ, body, マッチ数)
    let cases: &[(&str, &str, usize)] = &[
        // 直前の兄弟が要るもの。
        ("p:first-child", "<div>D</div><p>a</p><p>b</p>", 0),
        ("p:nth-child(2)", "<div>D</div><p>a</p><p>b</p>", 1),
        ("p:first-of-type", "<div>D</div><p>a</p><p>b</p>", 1),
        ("p:nth-of-type(2)", "<p>a</p><p>b</p><p>c</p>", 1),
        ("p:only-child", "<p>a</p><p>b</p>", 0),
        ("p:only-child", "<p>a</p>", 1),
        ("div + p", "<div>D</div><p>a</p><p>b</p>", 1),
        ("div ~ p", "<div>D</div><p>a</p><p>b</p>", 2),
        ("p + p", "<p>a</p><p>b</p><p>c</p>", 2),
        // 「自分より後ろに兄弟があるか」だけを見るものは、確定の条件
        // (次の兄弟が現れた)と一致するので判定がぶれない。
        ("p:last-child", "<p>a</p><div>b</div><p>c</p>", 1),
        ("p:last-child", "<p>a</p><p>b</p><div>c</div>", 0),
        // 自分の子だけを見るものは、確定した時点で子が揃っている。
        ("p:empty", "<p></p><p>b</p>", 0),
        (
            "section:has(h1)",
            "<section><h1>x</h1></section><p>b</p>",
            1,
        ),
        // 直後の兄弟は、確定のきっかけそのものなので必ず見えている。
        ("div:has(+ p)", "<div>a</div><p>b</p><p>c</p>", 1),
        // 位置に依存しないものは当然一致する。
        (":is(div, h1)", "<p>a</p><div>b</div>", 1),
        (":where(div, h1)", "<p>a</p><div>b</div>", 1),
    ];

    for (selector, body, expected) in cases {
        assert_eq!(
            matched_count(selector, body, Mode::Batch),
            *expected,
            "バッチの結果が期待とずれている: {selector} / {body}"
        );
        assert_eq!(
            matched_count(selector, body, Mode::Streaming),
            *expected,
            "ストリーミングでずれてはいけないセレクタ: {selector} / {body}"
        );
    }
}

/// バッチとストリーミングで結果がずれるセレクタ(現状の挙動を固定する)。
///
/// いずれも「この先に同じ型の要素が続くか」を知る必要があるが、トップレベル
/// 要素が確定するのは次の兄弟が現れた時点なので、その先は分からない。
/// 直前の兄弟を残す対処では埋められないぶん。
///
/// ここに挙げたものは`style::streaming_unsafe_selectors`が警告する対象と
/// 一致していること。
#[test]
fn these_selectors_diverge_in_streaming_mode() {
    // (セレクタ, body, バッチ, ストリーミング)
    let cases: &[(&str, &str, usize, usize)] = &[
        ("p:last-of-type", "<p>a</p><div>D</div><p>b</p>", 1, 2),
        ("p:only-of-type", "<p>a</p><div>D</div><p>b</p>", 0, 1),
        ("p:nth-last-child(2)", "<p>a</p><p>b</p><p>c</p>", 1, 2),
        (
            "p:nth-last-of-type(1)",
            "<p>a</p><div>D</div><p>b</p>",
            1,
            2,
        ),
        ("div:has(~ h1)", "<div>a</div><p>x</p><h1>b</h1>", 1, 0),
    ];

    for (selector, body, batch, streaming) in cases {
        assert_eq!(
            matched_count(selector, body, Mode::Batch),
            *batch,
            "バッチの結果が期待とずれている: {selector} / {body}"
        );
        assert_eq!(
            matched_count(selector, body, Mode::Streaming),
            *streaming,
            "ストリーミングの結果が期待とずれている: {selector} / {body}"
        );
    }
}
