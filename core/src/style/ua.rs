//! UAデフォルトスタイルシート(最小限)。
//!
//! `thead`/`tbody`/`tfoot`は`display: block`のままで、専用のボックスは持たない。
//! テーブルの行収集([`crate::layout::box_tree`])がこれらを透過的に素通りして
//! `table-row`の子孫を探すため、実質的に「テーブル本体との間の透明な入れ物」として
//! 扱われる。`caption`は`display: table-caption`専用の値を持ち、`box_tree.rs`が
//! `table-row`と並んで専用に検出する([0021](
//! ../../../docs/decisions/0021-table-layout-design.md)決定4関連)。

use super::stylesheet::{parse_stylesheet, Stylesheet};

const UA_CSS: &str = r#"
html, body, div, p,
h1, h2, h3, h4, h5, h6,
ul, ol, li,
thead, tbody, tfoot,
header, footer, section, article,
blockquote, figure, figcaption,
form, fieldset, hr, pre, dl, dt, dd,
address, main, nav, aside {
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

head, script, style, title, meta, link {
  display: none;
}

span, a, b, strong, i, em, small, code, label, abbr, sub, sup,
u, s, strike, ins, del, mark {
  display: inline;
}

b, strong {
  font-weight: bold;
}

i, em {
  font-style: italic;
}

u, ins {
  text-decoration: underline;
}

s, strike, del {
  text-decoration: line-through;
}

body {
  margin: 8px;
}

h1 { font-size: 32px; }
h2 { font-size: 24px; }
h3 { font-size: 19px; }

p, ul, ol, blockquote {
  margin: 16px 0;
}

pre {
  white-space: pre;
}
"#;

pub fn user_agent_stylesheet() -> Stylesheet {
    parse_stylesheet(UA_CSS)
}
