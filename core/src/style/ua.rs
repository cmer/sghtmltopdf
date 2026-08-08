//! UAデフォルトスタイルシート。
//!
//! WHATWG HTML仕様の"Rendering"節を出発点に、印刷/PDF出力で意味を持つ
//! 宣言だけを移植したもの。対話状態(フォーカス・
//! hover)、bidi、スクロール関連は移植していない。
//!
//! `thead`/`tbody`/`tfoot`は`display: block`のままで、専用のボックスは持たない。
//! テーブルの行収集([`crate::layout::box_tree`])がこれらを透過的に素通りして
//! `table-row`の子孫を探すため、実質的に「テーブル本体との間の透明な入れ物」として
//! 扱われる。`caption`は`display: table-caption`専用の値を持ち、`box_tree.rs`が
//! `table-row`と並んで専用に検出する。
//!
//! `display: none`にする要素の考え方: `ComputedStyle`の`display`初期値は
//! `Inline`なので、UA規則の無い要素はインラインとして扱われ、その子孫テキストが
//! 本文に流れ込む。描画できない埋め込みコンテンツ(`svg`/`canvas`/`video`等)や
//! フォームコントロールは、代替内容・選択肢のテキストが本文に漏れないよう
//! 明示的に`display: none`にする。フォームコントロールは
//! `display: inline-block`の静的な見た目に置き換えた。

use super::stylesheet::{parse_stylesheet, Stylesheet};

const UA_CSS: &str = r#"
/* ===== ブロックレベル要素 ===== */

html, body, div, p,
h1, h2, h3, h4, h5, h6,
ul, ol, menu, dl, dt, dd,
thead, tbody, tfoot,
header, footer, section, article, aside, nav, main, hgroup, search,
blockquote, figure, figcaption, pre, hr, address,
form, fieldset, legend, details, summary, dialog, center {
  display: block;
}

table {
  display: table;
}

tr {
  display: table-row;
}

td, th {
  display: table-cell;
}

caption {
  display: table-caption;
}

li {
  display: list-item;
}

/* ===== インライン要素 ===== */

span, a, b, strong, i, em, small, big, code, kbd, samp, var, cite, dfn,
label, abbr, q, sub, sup, u, s, strike, ins, del, mark, tt, font,
bdi, bdo, ruby, rt, rp, time, data, output, wbr, picture {
  display: inline;
}

/* ===== 非表示にする要素 ===== */

/* 文書メタデータ。`template`の中身はパース時点で別ツリーへ退避されるが
   (`html5ever`の`TreeSink`)、念のため明示する。 */
head, script, style, title, meta, link, base, noscript, template {
  display: none;
}

/* 描画できない埋め込みコンテンツ。要素名だけで判定し、
   名前空間は見ない。`layout::box_tree::child_kind`が`display: none`の要素で
   再帰を止めるため、ルート1つを消せばサブツリー全体(`<svg><text>`等)が
   消える。`picture`はその中の`<img>`を描画したいので対象外。 */
svg, math, canvas, video, audio, iframe, embed, object, param, track, source,
area, map {
  display: none;
}

/* フォームコントロールのうち、値の視覚化が帳票用途で意味を持たないもの・
   選択肢そのもの(表示テキストは`<select>`側が生成する)は非表示のまま。 */
option, optgroup, datalist, progress, meter {
  display: none;
}

input[type="hidden"] {
  display: none;
}

/* `hidden`属性。Originの優先度(UA < Author)により、
   作者CSSの`[hidden] { display: block }`が必ず勝つ。 */
[hidden] {
  display: none;
}

/* `<dialog>`は`open`属性が無ければ非表示。 */
dialog:not([open]) {
  display: none;
}

/* `<details>`は`open`属性が無ければ`summary`以外の子要素を隠す。
   直下の裸テキストノードはセレクタで指定できないため隠せない。 */
details:not([open]) > *:not(summary) {
  display: none;
}

/* ===== 文字の装飾 ===== */

b, strong, th,
h1, h2, h3, h4, h5, h6 {
  font-weight: bold;
}

i, em, cite, dfn, var, address {
  font-style: italic;
}

u, ins {
  text-decoration: underline;
}

s, strike, del {
  text-decoration: line-through;
}

/* `:link`は`href`を持つ`<a>`にマッチする(`style::element_ref`)。 */
a:link {
  color: #0000ee;
  text-decoration: underline;
}

mark {
  background-color: #ffff00;
  color: #000000;
}

/* ===== フォントサイズ ===== */

/* `font-size`の`em`は親のフォントサイズ基準で解決されるため、UA規則は
   相対値で書ける(`smaller`/`larger`等のキーワード値は非対応)。 */

h1 { font-size: 2em; }
h2 { font-size: 1.5em; }
h3 { font-size: 1.17em; }
h4 { font-size: 1em; }
h5 { font-size: 0.83em; }
h6 { font-size: 0.67em; }

small, sub, sup { font-size: 0.83em; }
big { font-size: 1.17em; }

/* 上下ずらしは`vertical-align`で行う。
   縮小(上の`font-size`)とは独立した指定である点はCSS仕様どおり。 */
sub { vertical-align: sub; }
sup { vertical-align: super; }

/* 等幅フォント。汎用family名`monospace`は`fonts::system`が自前の候補リストで
   具体フォントへ解決する。 */
pre, code, kbd, samp, tt {
  font-family: monospace;
}

/* ===== マージン・パディング ===== */

body {
  margin: 8px;
}

h1 { margin: 0.67em 0; }
h2 { margin: 0.83em 0; }
h3 { margin: 1em 0; }
h4 { margin: 1.33em 0; }
h5 { margin: 1.67em 0; }
h6 { margin: 2.33em 0; }

p, ul, ol, menu, dl, pre {
  margin: 16px 0;
}

blockquote, figure {
  margin: 16px 40px;
}

dd {
  margin-left: 40px;
}

ul, ol, menu {
  padding-left: 40px;
}

fieldset {
  margin: 0 2px;
  padding: 0.35em 0.75em 0.625em;
  border: 2px groove #c0c0c0;
}

legend {
  padding: 0 2px;
}

/* ===== 各要素固有 ===== */

ul, menu {
  list-style-type: disc;
}

ol {
  list-style-type: decimal;
}

pre {
  white-space: pre;
}

hr {
  margin: 8px auto;
  border-top: 1px inset #808080;
}

center {
  text-align: center;
}

caption {
  text-align: center;
}

th {
  text-align: center;
}

/* ===== フォームコントロールの静的描画 ===== */

/* 枠線付きの箱として行の中に置く。中身のテキスト(`value`/`placeholder`/
   選択中の`<option>`)はbox tree構築時に生成する。
   サイズは属性ではなくここで決める。 */
input, select, textarea, button {
  display: inline-block;
  border: 1px solid #767676;
  padding: 1px 2px;
  background-color: #ffffff;
  color: #000000;
  text-align: left;
  white-space: pre;
}

input, select {
  width: 12em;
  height: 1.6em;
  /* 中身の行の高さ(フォントによってはCJKのように大きくなる)が箱の高さを
     超えても外へはみ出さないようにする。ブラウザのフォームコントロールと
     同じ挙動。 */
  overflow: hidden;
}

textarea {
  width: 20em;
  height: 4em;
  overflow: hidden;
  font-family: monospace;
}

button, input[type="submit"], input[type="reset"], input[type="button"] {
  width: auto;
  padding: 2px 8px;
  background-color: #efefef;
  text-align: center;
}

/* チェックボックス・ラジオは小さな枠。`checked`なら塗りつぶす。 */
input[type="checkbox"], input[type="radio"] {
  width: 11px;
  height: 11px;
  padding: 0;
}

/* `border-radius`のパーセンテージは非対応なので、箱の半分の
   px値で円にする。 */
input[type="radio"] {
  border-radius: 6px;
}

input[type="checkbox"][checked], input[type="radio"][checked] {
  background-color: #333333;
}

/* `disabled`は薄いグレーで表す。 */
input[disabled], select[disabled], textarea[disabled], button[disabled] {
  background-color: #ebebeb;
  color: #6d6d6d;
}

fieldset {
  min-width: 0;
}

/* `<q>`の自動引用符。`quotes`の初期値を実装済みなので、
   入れ子の深さに応じた引用符がこれだけで出る。 */
q::before {
  content: open-quote;
}

q::after {
  content: close-quote;
}
"#;

pub fn user_agent_stylesheet() -> Stylesheet {
    parse_stylesheet(UA_CSS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::{self, Dom, NodeData, NodeId};
    use crate::style::values::{Display, FontStyle, FontWeight, TextAlign};
    use crate::style::{compute_styles, parse_stylesheet, ComputedStyle};

    fn find(dom: &Dom, id: NodeId, tag: &str) -> Option<NodeId> {
        if let NodeData::Element { name, .. } = &dom.node(id).data {
            if &*name.local == tag {
                return Some(id);
            }
        }
        dom.children(id).find_map(|child| find(dom, child, tag))
    }

    /// `html_src`をUAスタイルシートだけで計算し、`tag`の計算スタイルを返す。
    fn style_of(html_src: &str, tag: &str) -> ComputedStyle {
        let dom = html::parse(html_src.as_bytes());
        let styles = compute_styles(&dom, &user_agent_stylesheet(), &parse_stylesheet(""));
        let node = find(&dom, dom.document(), tag).expect("element not found");
        (*styles[&node]).clone()
    }

    #[test]
    fn html5_sectioning_elements_are_block_level() {
        for tag in [
            "article",
            "section",
            "header",
            "footer",
            "aside",
            "nav",
            "main",
            "hgroup",
            "figure",
            "figcaption",
            "details",
            "summary",
            "dialog",
        ] {
            let html_src = format!("<{tag}>x</{tag}>");
            // `dialog`は`open`が無いと非表示なので、その確認は別テストで行う。
            let expected = if tag == "dialog" {
                Display::None
            } else {
                Display::Block
            };
            assert_eq!(
                style_of(&html_src, tag).display,
                expected,
                "unexpected display for <{tag}>"
            );
        }
    }

    #[test]
    fn phrasing_elements_stay_inline() {
        for tag in [
            "cite", "dfn", "var", "abbr", "time", "data", "output", "bdi", "bdo", "ruby", "rt",
            "rp", "mark", "big", "tt",
        ] {
            let html_src = format!("<p><{tag}>x</{tag}></p>");
            assert_eq!(
                style_of(&html_src, tag).display,
                Display::Inline,
                "unexpected display for <{tag}>"
            );
        }
    }

    #[test]
    fn undisplayable_embedded_content_is_hidden() {
        for tag in [
            "svg", "math", "canvas", "video", "audio", "iframe", "embed", "object",
        ] {
            let html_src = format!("<{tag}>x</{tag}>");
            assert_eq!(
                style_of(&html_src, tag).display,
                Display::None,
                "<{tag}> should be hidden"
            );
        }
    }

    #[test]
    fn form_controls_are_inline_blocks_with_a_border() {
        for tag in ["input", "select", "textarea", "button"] {
            let html_src = format!("<form><{tag}>x</{tag}></form>");
            let style = style_of(&html_src, tag);
            assert_eq!(
                style.display,
                Display::InlineBlock,
                "<{tag}> should be drawn as a static box"
            );
            assert_eq!(
                style.border_top_width.0, 1.0,
                "<{tag}> should have a border"
            );
        }
        // 値の視覚化に意味が無いもの・選択肢そのものは非表示のまま。
        for tag in ["progress", "meter", "datalist"] {
            let html_src = format!("<form><{tag}>x</{tag}></form>");
            assert_eq!(style_of(&html_src, tag).display, Display::None);
        }
        assert_eq!(
            style_of("<select><option>a</option></select>", "option").display,
            Display::None
        );
        assert_eq!(
            style_of(r#"<input type="hidden" value="x">"#, "input").display,
            Display::None
        );
        // フォームそのもの・label・fieldsetは中身が文章として意味を持つので隠さない。
        assert_eq!(style_of("<form>x</form>", "form").display, Display::Block);
        assert_eq!(
            style_of("<p><label>x</label></p>", "label").display,
            Display::Inline
        );
    }

    #[test]
    fn a_checked_checkbox_is_filled() {
        let unchecked = style_of(r#"<input type="checkbox">"#, "input");
        let checked = style_of(r#"<input type="checkbox" checked>"#, "input");
        assert_ne!(
            checked.background_color, unchecked.background_color,
            "a checked box must be visually distinct"
        );
    }

    #[test]
    fn a_disabled_control_is_greyed_out() {
        let normal = style_of("<input>", "input");
        let disabled = style_of("<input disabled>", "input");
        assert_ne!(disabled.background_color, normal.background_color);
        assert_ne!(disabled.color, normal.color);
    }

    #[test]
    fn headings_are_bold_and_shrink_with_the_level() {
        let sizes: Vec<f32> = ["h1", "h2", "h3", "h4", "h5", "h6"]
            .iter()
            .map(|tag| {
                let html_src = format!("<{tag}>x</{tag}>");
                let style = style_of(&html_src, tag);
                assert_eq!(
                    style.font_weight,
                    FontWeight::Bold,
                    "<{tag}> should be bold"
                );
                style.font_size.0
            })
            .collect();
        for pair in sizes.windows(2) {
            assert!(pair[0] > pair[1], "font sizes should decrease: {sizes:?}");
        }
        // 既定16px基準: h1 = 2em = 32px、h4 = 1em = 16px。
        assert_eq!(sizes[0], 32.0);
        assert_eq!(sizes[3], 16.0);
    }

    #[test]
    fn relative_font_sizes_resolve_against_the_parent() {
        // `<small>`は0.83em。親(p)が20pxなら16.6pxになる(ルート基準ではない)。
        let dom = html::parse(b"<p><small>x</small></p>");
        let styles = compute_styles(
            &dom,
            &user_agent_stylesheet(),
            &parse_stylesheet("p { font-size: 20px; }"),
        );
        let small = find(&dom, dom.document(), "small").expect("small not found");
        assert!((styles[&small].font_size.0 - 16.6).abs() < 0.01);
    }

    #[test]
    fn preformatted_and_code_use_the_monospace_generic_family() {
        for tag in ["pre", "code", "kbd", "samp", "tt"] {
            let html_src = format!("<{tag}>x</{tag}>");
            assert_eq!(
                style_of(&html_src, tag).font_family,
                vec!["monospace".to_string()],
                "<{tag}> should request the monospace generic family"
            );
        }
    }

    #[test]
    fn emphasis_elements_are_italic() {
        for tag in ["i", "em", "cite", "dfn", "var"] {
            let html_src = format!("<p><{tag}>x</{tag}></p>");
            assert_eq!(style_of(&html_src, tag).font_style, FontStyle::Italic);
        }
        assert_eq!(
            style_of("<address>x</address>", "address").font_style,
            FontStyle::Italic
        );
    }

    #[test]
    fn table_header_cells_are_bold_and_centered() {
        let style = style_of("<table><tr><th>x</th></tr></table>", "th");
        assert_eq!(style.font_weight, FontWeight::Bold);
        assert_eq!(style.text_align, TextAlign::Center);
    }

    #[test]
    fn hr_has_a_top_border_so_it_draws_a_line() {
        let style = style_of("<hr>", "hr");
        assert_eq!(style.display, Display::Block);
        assert_eq!(style.border_top_width.0, 1.0);
        assert_ne!(
            style.border_top_style,
            super::super::values::BorderStyle::None
        );
    }

    #[test]
    fn a_link_gets_the_default_link_decoration() {
        // `:link`のマッチングは`style::element_ref`が`href`の有無で静的に判定する。
        let with_href = style_of(r#"<p><a href="x">link</a></p>"#, "a");
        assert_eq!(with_href.color.blue, 0xee);
        assert!(with_href.text_decoration_line.underline);

        let without_href = style_of("<p><a>anchor</a></p>", "a");
        assert_eq!(
            without_href.color.blue, 0,
            "an <a> without href is not a link and keeps the inherited color"
        );
    }

    #[test]
    fn a_closed_details_hides_everything_but_its_summary() {
        let dom = html::parse(b"<details><summary>s</summary><p>body</p></details>");
        let styles = compute_styles(&dom, &user_agent_stylesheet(), &parse_stylesheet(""));
        let summary = find(&dom, dom.document(), "summary").expect("summary not found");
        let body = find(&dom, dom.document(), "p").expect("p not found");
        assert_eq!(styles[&summary].display, Display::Block);
        assert_eq!(styles[&body].display, Display::None);
    }

    #[test]
    fn an_open_details_shows_everything() {
        let dom = html::parse(b"<details open><summary>s</summary><p>body</p></details>");
        let styles = compute_styles(&dom, &user_agent_stylesheet(), &parse_stylesheet(""));
        let body = find(&dom, dom.document(), "p").expect("p not found");
        assert_eq!(styles[&body].display, Display::Block);
    }

    #[test]
    fn the_hidden_attribute_hides_any_element() {
        assert_eq!(
            style_of("<div hidden>x</div>", "div").display,
            Display::None
        );
    }

    #[test]
    fn author_css_overrides_the_hidden_attribute() {
        let dom = html::parse(b"<div hidden>x</div>");
        let styles = compute_styles(
            &dom,
            &user_agent_stylesheet(),
            &parse_stylesheet("[hidden] { display: block; }"),
        );
        let div = find(&dom, dom.document(), "div").expect("div not found");
        assert_eq!(
            styles[&div].display,
            Display::Block,
            "author origin must win over the UA rule"
        );
    }

    #[test]
    fn q_generates_quotation_marks() {
        let style = style_of("<p><q>x</q></p>", "q");
        assert_eq!(style.pseudo_before_content.as_deref(), Some("\u{201c}"));
        assert_eq!(style.pseudo_after_content.as_deref(), Some("\u{201d}"));
    }
}
