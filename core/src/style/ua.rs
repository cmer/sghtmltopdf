//! UAデフォルトスタイルシート(最小限)。
//!
//! `table`関連要素は本来`display: table`等の専用フォーマッティングコンテキストを
//! 持つが、M1のレイアウトはブロック/インラインのみに対応するため`block`として扱う
//! (テーブルレイアウトへの対応は将来のマイルストーンで見直す)。

use super::stylesheet::{parse_stylesheet, Stylesheet};

const UA_CSS: &str = r#"
html, body, div, p,
h1, h2, h3, h4, h5, h6,
ul, ol, li,
table, thead, tbody, tfoot, tr, td, th,
header, footer, section, article,
blockquote, figure, figcaption,
form, fieldset, hr, pre, dl, dt, dd,
address, main, nav, aside {
  display: block;
}

head, script, style, title, meta, link {
  display: none;
}

span, a, b, strong, i, em, small, code, label, abbr, sub, sup, u, s, mark {
  display: inline;
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
"#;

pub fn user_agent_stylesheet() -> Stylesheet {
    parse_stylesheet(UA_CSS)
}
