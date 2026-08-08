//! `<img>`要素のDOM属性抽出。
//!
//! `width`/`height`はCSSの値ではなくHTML属性(単位なしの整数px)を指す。
//!
//! HTML仕様の"rules for parsing non-negative integers"は、先頭の空白を
//! 読み飛ばした後、数字が続く限り読んで残りは無視する(文字列全体が数字列
//! である必要はない)。そのため`width="100px"`のような単位付きの値も
//! ブラウザ実装では`100`として解釈される。この挙動に合わせ、`str::parse`の
//! ような文字列全体一致ではなく、先頭の数字列だけを取り出して解釈する。

use html5ever::Attribute;

use crate::html::{Dom, NodeData, NodeId};

/// `<img>`要素から読み取った属性(URL解決前の生の値)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImgAttrs {
    /// `src`属性の値(空文字列は`None`扱いになるためここには来ない)。
    pub src: String,
    /// `width`属性の値(px単位、未指定または非数値は`None`)。
    pub width: Option<u32>,
    /// `height`属性の値(px単位、未指定または非数値は`None`)。
    pub height: Option<u32>,
    /// `alt`属性の値。属性そのものが無ければ`None`、`alt=""`のような
    /// 明示的な空値は`Some(String::new())`として区別する(装飾目的の画像を
    /// 表す`alt=""`と、未指定を区別するHTMLの慣習に合わせる)。
    pub alt: Option<String>,
}

/// `node`が`src`属性を持つ`<img>`要素の場合のみ属性を読み取る。
///
/// `<img>`要素でない、または`src`が無い(あっても空文字列)場合は`None`を
/// 返す。呼び出し側はこれを「画像なしの置換要素」として扱う。
pub fn read_img_attrs(dom: &Dom, node: NodeId) -> Option<ImgAttrs> {
    let NodeData::Element { name, attrs, .. } = &dom.node(node).data else {
        return None;
    };
    if &*name.local != "img" {
        return None;
    }

    let src = find_attr(attrs, "src")
        .map(|value| value.to_string())
        .filter(|s| !s.is_empty())?;
    let width = read_pixel_attr(attrs, "width");
    let height = read_pixel_attr(attrs, "height");
    let alt = find_attr(attrs, "alt").map(|value| value.to_string());

    Some(ImgAttrs {
        src,
        width,
        height,
        alt,
    })
}

fn find_attr<'a>(attrs: &'a [Attribute], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|attr| &*attr.name.local == name)
        .map(|attr| attr.value.as_ref())
}

fn read_pixel_attr(attrs: &[Attribute], name: &str) -> Option<u32> {
    find_attr(attrs, name).and_then(parse_non_negative_integer_prefix)
}

/// HTML仕様の"rules for parsing non-negative integers"を簡略化して適用する:
/// 先頭の空白を読み飛ばし、後続の数字列を読めるだけ読んで10進数として
/// 解釈する(以降の非数字はすべて無視する)。
/// 数字が1つも無ければ`None`。先頭が`-`なら数字の収集そのものが始まらないため自然に`None`になる。
fn parse_non_negative_integer_prefix(value: &str) -> Option<u32> {
    let digits: String = value
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u32>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html;

    fn find(dom: &Dom, id: NodeId, tag: &str) -> Option<NodeId> {
        if let NodeData::Element { name, .. } = &dom.node(id).data {
            if &*name.local == tag {
                return Some(id);
            }
        }
        dom.children(id).find_map(|child| find(dom, child, tag))
    }

    #[test]
    fn reads_all_attributes_when_present() {
        let dom =
            html::parse(br#"<img src="logo.png" width="120" height="40" alt="Company logo">"#);
        let img = find(&dom, dom.document(), "img").expect("img not found");

        let attrs = read_img_attrs(&dom, img).expect("expected Some");
        assert_eq!(attrs.src, "logo.png");
        assert_eq!(attrs.width, Some(120));
        assert_eq!(attrs.height, Some(40));
        assert_eq!(attrs.alt.as_deref(), Some("Company logo"));
    }

    #[test]
    fn missing_optional_attributes_are_none() {
        let dom = html::parse(br#"<img src="logo.png">"#);
        let img = find(&dom, dom.document(), "img").expect("img not found");

        let attrs = read_img_attrs(&dom, img).expect("expected Some");
        assert_eq!(attrs.width, None);
        assert_eq!(attrs.height, None);
        assert_eq!(attrs.alt, None);
    }

    #[test]
    fn empty_alt_is_distinguished_from_missing_alt() {
        let dom = html::parse(br#"<img src="deco.png" alt="">"#);
        let img = find(&dom, dom.document(), "img").expect("img not found");

        let attrs = read_img_attrs(&dom, img).expect("expected Some");
        assert_eq!(attrs.alt.as_deref(), Some(""));
    }

    #[test]
    fn width_and_height_accept_a_trailing_unit_suffix() {
        // HTML仕様上、width/height属性値は先頭の数字列だけを読んで残りは
        // 無視するため、`100px`や`50%`のような単位付きの値も実質px指定として
        // 解釈される(ブラウザの実際の挙動と一致させる)。
        let dom = html::parse(br#"<img src="logo.png" width="100px" height="50%">"#);
        let img = find(&dom, dom.document(), "img").expect("img not found");

        let attrs = read_img_attrs(&dom, img).expect("expected Some");
        assert_eq!(attrs.width, Some(100));
        assert_eq!(attrs.height, Some(50));
    }

    #[test]
    fn width_ignores_leading_and_trailing_whitespace() {
        let dom = html::parse(br#"<img src="logo.png" width=" 42 ">"#);
        let img = find(&dom, dom.document(), "img").expect("img not found");

        let attrs = read_img_attrs(&dom, img).expect("expected Some");
        assert_eq!(attrs.width, Some(42));
    }

    #[test]
    fn non_numeric_width_and_height_are_none() {
        let dom = html::parse(br#"<img src="logo.png" width="huge" height="-1">"#);
        let img = find(&dom, dom.document(), "img").expect("img not found");

        let attrs = read_img_attrs(&dom, img).expect("expected Some");
        assert_eq!(attrs.width, None);
        assert_eq!(
            attrs.height, None,
            "negative numbers are not valid non-negative integers"
        );
    }

    #[test]
    fn missing_src_is_none() {
        let dom = html::parse(br#"<img alt="no src">"#);
        let img = find(&dom, dom.document(), "img").expect("img not found");

        assert_eq!(read_img_attrs(&dom, img), None);
    }

    #[test]
    fn empty_src_is_none() {
        let dom = html::parse(br#"<img src="">"#);
        let img = find(&dom, dom.document(), "img").expect("img not found");

        assert_eq!(read_img_attrs(&dom, img), None);
    }

    #[test]
    fn non_img_element_is_none() {
        let dom = html::parse(br#"<div src="not-an-img.png"></div>"#);
        let div = find(&dom, dom.document(), "div").expect("div not found");

        assert_eq!(read_img_attrs(&dom, div), None);
    }

    #[test]
    fn released_node_is_none() {
        let mut dom = html::parse(br#"<div><img src="logo.png"></div>"#);
        let img = find(&dom, dom.document(), "img").expect("img not found");
        dom.release_subtree(img);

        assert_eq!(read_img_attrs(&dom, img), None);
    }
}
