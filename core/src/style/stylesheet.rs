//! stylesheet全体(ルールの集合)のパース。

use std::cell::RefCell;
use std::rc::Rc;

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserInput,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, Token,
};
use selectors::parser::{
    Combinator, Component, NthSelectorData, NthType, ParseRelative, Selector, SelectorList,
};

use super::font_face::{parse_font_face_block, FontFaceRule};
use super::page_rule::{parse_page_rule_block, parse_page_selector, PageRule};
use super::properties::{parse_declaration, PropertyDeclaration};
use super::rule_index::RuleIndex;
use super::selector_impl::{SelectorParser, SgSelectorImpl};

#[derive(Debug, Clone)]
pub struct StyleRule {
    pub selectors: SelectorList<SgSelectorImpl>,
    pub declarations: Vec<PropertyDeclaration>,
}

#[derive(Debug, Clone, Default)]
pub struct Stylesheet {
    pub rules: Vec<StyleRule>,
    pub font_faces: Vec<FontFaceRule>,
    pub page_rules: Vec<PageRule>,
    /// [`Self::index`]が作る索引のメモ。
    index: RefCell<Option<Rc<RuleIndex>>>,
}

impl Stylesheet {
    /// セレクタマッチングの候補を絞る索引。初回の参照時に組み立てる。
    ///
    /// `rules`は公開フィールドで、パース後に連結される場合がある
    /// (ユーザーCSSをUAスタイルシートの末尾へ足す経路)。連結の際に索引を
    /// 捨て忘れると古い索引が残るため、ルール数が変わっていたら作り直す。
    pub fn index(&self) -> Rc<RuleIndex> {
        if let Some(index) = self.index.borrow().as_ref() {
            if index.rule_count() == self.rules.len() {
                return Rc::clone(index);
            }
        }
        let built = Rc::new(RuleIndex::build(&self.rules));
        *self.index.borrow_mut() = Some(Rc::clone(&built));
        built
    }
}

/// トップレベルルールの中間表現。通常のスタイルルール・`@font-face`・
/// `@media`・`@layer`・`@page`は`StyleSheetParser`の型システム上、同じ
/// `Prelude`/`Rule`型を共有する必要があるため、この列挙型で束ねる
/// ([`parse_stylesheet`]で仕分ける)。
enum TopLevelRule {
    /// スタイルルール1つと、その中にネストしていたルールをカスケード順に
    /// 平坦化したもの([`parse_style_rule_body`])。
    Style(Vec<StyleRule>),
    FontFace(FontFaceRule),
    /// ルールをまとめるat-rule(`@media`/`@layer`)の中身。書かれた順に
    /// トップレベルへ展開する。マッチしなかった`@media`と、ブロックを持たない
    /// `@layer a, b;`は空のVec。
    Nested(Vec<TopLevelRule>),
    Page(PageRule),
}

pub fn parse_stylesheet(css: &str) -> Stylesheet {
    let mut input = ParserInput::new(css);
    let mut parser = Parser::new(&mut input);
    let mut rule_parser = TopLevelRuleParser;

    let mut rules = Vec::new();
    let mut font_faces = Vec::new();
    let mut page_rules = Vec::new();
    for result in StyleSheetParser::new(&mut parser, &mut rule_parser).flatten() {
        flatten_top_level_rule(result, &mut rules, &mut font_faces, &mut page_rules);
    }

    Stylesheet {
        rules,
        font_faces,
        page_rules,
        index: RefCell::new(None),
    }
}

fn flatten_top_level_rule(
    rule: TopLevelRule,
    rules: &mut Vec<StyleRule>,
    font_faces: &mut Vec<FontFaceRule>,
    page_rules: &mut Vec<PageRule>,
) {
    match rule {
        // 宣言が空のルールはカスケードにも擬似要素の解決にも寄与しないので、
        // 索引と照合のコストだけが残る。ここで捨てる。
        // ネストしたルールの親(`.a { &:hover { } }`の`.a`)は宣言を持たない
        // ことが多く、残すとネスト1つにつきルールが2つに増える。
        TopLevelRule::Style(r) => {
            rules.extend(r.into_iter().filter(|rule| !rule.declarations.is_empty()))
        }
        TopLevelRule::FontFace(r) => font_faces.push(r),
        TopLevelRule::Page(r) => page_rules.push(r),
        TopLevelRule::Nested(inner) => {
            for r in inner {
                flatten_top_level_rule(r, rules, font_faces, page_rules);
            }
        }
    }
}

/// `style="..."`属性の値のような、セレクタを伴わない宣言リストをパースする。
pub fn parse_inline_style(css: &str) -> Vec<PropertyDeclaration> {
    let mut input = ParserInput::new(css);
    let mut parser = Parser::new(&mut input);
    let mut declaration_parser = DeclarationBlockParser;

    RuleBodyParser::new(&mut parser, &mut declaration_parser)
        .filter_map(Result::ok)
        .flatten()
        .collect()
}

/// stylesheet直下のルール(セレクタ+宣言ブロック)をパースする。
struct TopLevelRuleParser;

impl<'i> QualifiedRuleParser<'i> for TopLevelRuleParser {
    type Prelude = SelectorList<SgSelectorImpl>;
    type QualifiedRule = TopLevelRule;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, Self::Error>> {
        SelectorList::parse(&SelectorParser, input, ParseRelative::No)
            .map_err(|_| input.new_custom_error(()))
    }

    fn parse_block<'t>(
        &mut self,
        selectors: Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::QualifiedRule, ParseError<'i, Self::Error>> {
        Ok(TopLevelRule::Style(parse_style_rule_body(selectors, input)))
    }
}

/// スタイルルールの`{ }`の中身(宣言とネストしたスタイルルール)をパースし、
/// カスケード順に並んだ平坦なルール列にする。
///
/// CSS Nestingでは、ネストしたルールは親ルールの直後に(ソース順で)置かれた
/// ものとして扱う。ネストしたルールより後ろに書かれた宣言は、親と同じ
/// セレクタを持つ別のルール(仕様のCSSNestedDeclarations)として、その
/// ネストしたルールの後ろに並ぶ。先頭へ巻き上げると、`&`で親自身を上書き
/// するルールとの勝敗がソース順と食い違うため、書かれた順を保つ。
///
/// 先頭のルール(親そのもの)と末尾の宣言ルールは、宣言が無ければ空のまま
/// 返る。宣言が空のルールは[`flatten_top_level_rule`]が捨てる。
fn parse_style_rule_body(
    selectors: SelectorList<SgSelectorImpl>,
    input: &mut Parser<'_, '_>,
) -> Vec<StyleRule> {
    let mut rules = vec![StyleRule {
        selectors: selectors.clone(),
        declarations: Vec::new(),
    }];
    // 宣言を受け入れる先。ネストしたルールを挟むたびに`None`へ戻し、
    // 次の宣言が来た時点で`&`と同じセレクタのルールを新しく作る。
    let mut open: Option<usize> = Some(0);
    // ネストしたルールより後ろに書いた宣言(仕様のCSSNestedDeclarations)は
    // `&`と同じセレクタを持つ。親がセレクタリストのときは`:is(親)`へ
    // まとまって詳細度が親そのものとは変わるので、解決結果を別に用意する。
    let mut nested_selectors: Option<SelectorList<SgSelectorImpl>> = None;

    let mut body_parser = StyleRuleBodyParser { parent: &selectors };
    for item in RuleBodyParser::new(input, &mut body_parser).filter_map(Result::ok) {
        match item {
            StyleRuleBodyItem::Declarations(declarations) => {
                let index = *open.get_or_insert_with(|| {
                    let selectors = nested_selectors
                        .get_or_insert_with(|| parent_selector_reference(&selectors))
                        .clone();
                    rules.push(StyleRule {
                        selectors,
                        declarations: Vec::new(),
                    });
                    rules.len() - 1
                });
                rules[index].declarations.extend(declarations);
            }
            StyleRuleBodyItem::Nested(nested) => {
                rules.extend(nested);
                open = None;
            }
        }
    }
    rules
}

/// `&`(親セレクタ)を`parent`で解決したセレクタリスト。
///
/// 親が1つだけなら`:is()`で包んでも詳細度は変わらないため、そのまま返す
/// (出力されるセレクタを不必要に変えない)。
fn parent_selector_reference(
    parent: &SelectorList<SgSelectorImpl>,
) -> SelectorList<SgSelectorImpl> {
    if parent.slice().len() == 1 {
        return parent.clone();
    }
    let mut input = ParserInput::new("&");
    let mut parser = Parser::new(&mut input);
    match SelectorList::parse(&SelectorParser, &mut parser, ParseRelative::ForNesting) {
        Ok(list) => list.replace_parent_selector(parent),
        Err(_) => parent.clone(),
    }
}

/// [`StyleRuleBodyParser`]が返す、ルール本体の1項目。
enum StyleRuleBodyItem {
    /// 宣言1つ(ショートハンドは展開後の複数)。
    Declarations(Vec<PropertyDeclaration>),
    /// ネストしたスタイルルール(とその中のネスト)を平坦化したもの。
    Nested(Vec<StyleRule>),
}

/// スタイルルールの`{ }`の中身をパースする。宣言に加えて、ネストした
/// スタイルルール(CSS Nesting)を受け付ける。
///
/// ネストしたルールのセレクタは`&`(親セレクタ)を含む相対セレクタとして
/// パースし、`&`を親のセレクタリスト(`:is(親)`相当)へ置き換えて解決する。
/// `&`を書かない`.probe { }`は`& .probe`、先頭がコンビネータの`> li { }`は
/// `& > li`として扱う(`selectors`クレートの`ParseRelative::ForNesting`)。
///
/// ネストしたat-rule(`@media`等)は非対応で、ブロックごと読み飛ばす。
struct StyleRuleBodyParser<'a> {
    parent: &'a SelectorList<SgSelectorImpl>,
}

impl<'i> DeclarationParser<'i> for StyleRuleBodyParser<'_> {
    type Declaration = StyleRuleBodyItem;
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _declaration_start: &cssparser::ParserState,
    ) -> Result<Self::Declaration, ParseError<'i, Self::Error>> {
        parse_declaration(&name, input).map(StyleRuleBodyItem::Declarations)
    }
}

impl<'i> QualifiedRuleParser<'i> for StyleRuleBodyParser<'_> {
    type Prelude = SelectorList<SgSelectorImpl>;
    type QualifiedRule = StyleRuleBodyItem;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, Self::Error>> {
        let relative = SelectorList::parse(&SelectorParser, input, ParseRelative::ForNesting)
            .map_err(|_| input.new_custom_error(()))?;
        Ok(relative.replace_parent_selector(self.parent))
    }

    fn parse_block<'t>(
        &mut self,
        selectors: Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::QualifiedRule, ParseError<'i, Self::Error>> {
        Ok(StyleRuleBodyItem::Nested(parse_style_rule_body(
            selectors, input,
        )))
    }
}

impl<'i> AtRuleParser<'i> for StyleRuleBodyParser<'_> {
    type Prelude = ();
    type AtRule = StyleRuleBodyItem;
    type Error = ();
}

impl<'i> RuleBodyItemParser<'i, StyleRuleBodyItem, ()> for StyleRuleBodyParser<'_> {
    fn parse_declarations(&self) -> bool {
        true
    }

    fn parse_qualified(&self) -> bool {
        true
    }
}

/// `@font-face`/`@media`/`@layer`/`@page`を認識する。
enum TopLevelAtRulePrelude {
    FontFace,
    /// `applies`は[`media_query_list_matches`]による判定結果。
    Media {
        applies: bool,
    },
    /// `@layer`。レイヤーの優先順位は実装せず、ブロックの中身を書かれた順に
    /// トップレベルへ展開するだけなので、レイヤー名は保持しない(#20)。
    Layer,
    Page(super::page_rule::PageSelector),
}

impl<'i> AtRuleParser<'i> for TopLevelRuleParser {
    type Prelude = TopLevelAtRulePrelude;
    type AtRule = TopLevelRule;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, Self::Error>> {
        if name.eq_ignore_ascii_case("font-face") {
            return Ok(TopLevelAtRulePrelude::FontFace);
        }
        if name.eq_ignore_ascii_case("media") {
            let applies = media_query_list_matches(input)?;
            return Ok(TopLevelAtRulePrelude::Media { applies });
        }
        if name.eq_ignore_ascii_case("layer") {
            skip_layer_name_list(input)?;
            return Ok(TopLevelAtRulePrelude::Layer);
        }
        if name.eq_ignore_ascii_case("page") {
            let selector = parse_page_selector(input)?;
            return Ok(TopLevelAtRulePrelude::Page(selector));
        }
        Err(input.new_custom_error(()))
    }

    /// ブロックを持たない形(`@layer theme, base;`)。レイヤーの順序を宣言する
    /// だけでルールを含まないので、空のグループとして受理する。他のat-ruleに
    /// この形はない。
    fn rule_without_block(
        &mut self,
        prelude: Self::Prelude,
        _start: &cssparser::ParserState,
    ) -> Result<Self::AtRule, ()> {
        match prelude {
            TopLevelAtRulePrelude::Layer => Ok(TopLevelRule::Nested(Vec::new())),
            _ => Err(()),
        }
    }

    fn parse_block<'t>(
        &mut self,
        prelude: Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::AtRule, ParseError<'i, Self::Error>> {
        match prelude {
            TopLevelAtRulePrelude::FontFace => {
                Ok(TopLevelRule::FontFace(parse_font_face_block(input)?))
            }
            TopLevelAtRulePrelude::Page(selector) => {
                Ok(TopLevelRule::Page(parse_page_rule_block(input, selector)))
            }
            // マッチしなかった`@media`ブロックの中身は読み飛ばすだけでよい
            // (`input`は既にこのブロックにスコープされているため、何も
            // 消費せず返しても呼び出し元が正しくブロックの終端まで進める)。
            TopLevelAtRulePrelude::Media { applies: false } => Ok(TopLevelRule::Nested(Vec::new())),
            TopLevelAtRulePrelude::Media { applies: true } | TopLevelAtRulePrelude::Layer => {
                Ok(TopLevelRule::Nested(parse_nested_rules(input)))
            }
        }
    }
}

/// `@media`/`@layer`のブロックの中身を、トップレベルと同じ文法でパースする。
/// 中でさらに`@media`/`@layer`が入れ子になっていてもよい。
fn parse_nested_rules<'i, 't>(input: &mut Parser<'i, 't>) -> Vec<TopLevelRule> {
    let mut rule_parser = TopLevelRuleParser;
    StyleSheetParser::new(input, &mut rule_parser)
        .flatten()
        .collect()
}

/// `@layer`のprelude(`<layer-name>#`または空)を読み飛ばす。レイヤー名は
/// `a`や`a.b`(`.`区切りの入れ子)で、ブロック形は高々1つ、文形はカンマ区切りで
/// 複数書ける。ここでは文法だけ確かめて名前は捨てる。名前が要らないとはいえ
/// 何でも受理すると、壊れた`@layer`の中身までトップレベルに漏れてしまう。
fn skip_layer_name_list<'i, 't>(input: &mut Parser<'i, 't>) -> Result<(), ParseError<'i, ()>> {
    if input.is_exhausted() {
        return Ok(());
    }
    loop {
        input.expect_ident()?;
        while input.try_parse(|input| input.expect_delim('.')).is_ok() {
            input.expect_ident()?;
        }
        if input.is_exhausted() {
            return Ok(());
        }
        input.expect_comma()?;
    }
}

/// `@media`のprelude(トークン列)から、印刷/PDF出力用の簡略化された
/// メディアタイプ判定を行う。カンマ区切りのクエリリスト(=OR)を、それぞれ
/// 「先頭の`not`/`only`修飾子+メディアタイプ識別子」だけ見て判定する。
/// 特徴クエリ(`(min-width: ...)`等)は一切評価せず読み飛ばす。
fn media_query_list_matches<'i, 't>(
    input: &mut Parser<'i, 't>,
) -> Result<bool, ParseError<'i, ()>> {
    let mut any_matches = false;
    loop {
        let (matches, has_more) = parse_one_media_query(input)?;
        any_matches = any_matches || matches;
        if !has_more {
            break;
        }
    }
    Ok(any_matches)
}

/// 1つのメディアクエリ(次のコンマまで、またはpreludeの終端まで)を判定し、
/// その分のトークンを消費する。特徴クエリ部分は評価せず読み飛ばす。戻り値は
/// `(このクエリがマッチしたか, まだ後続クエリがあるか)`。
fn parse_one_media_query<'i, 't>(
    input: &mut Parser<'i, 't>,
) -> Result<(bool, bool), ParseError<'i, ()>> {
    let mut negate = false;
    let mut media_type: Option<CowRcStr<'i>> = None;

    if let Ok(ident) = input.try_parse(|input| input.expect_ident_cloned()) {
        if ident.eq_ignore_ascii_case("not") {
            negate = true;
            media_type = input.try_parse(|input| input.expect_ident_cloned()).ok();
        } else if ident.eq_ignore_ascii_case("only") {
            media_type = input.try_parse(|input| input.expect_ident_cloned()).ok();
        } else {
            media_type = Some(ident);
        }
    }

    // 残り(`and (min-width: ...)`等)は評価せず、次のコンマ(を消費して
    // 「後続あり」を伝える)またはpreludeの終端まで読み飛ばす。
    let has_more = loop {
        match input.next() {
            Ok(Token::Comma) => break true,
            Ok(_) => continue,
            Err(_) => break false,
        }
    };

    let is_screen = media_type
        .as_deref()
        .map(|ty| ty.eq_ignore_ascii_case("screen"))
        .unwrap_or(false);
    Ok((is_screen == negate, has_more))
}

/// `{ }`内の宣言のみをパースする(ネストしたルールは扱わない)。
/// `style`属性と`@page`のmargin box(`page_rule.rs`)が使う。スタイルルールの
/// 本体は、ネストしたルールも受け付ける[`StyleRuleBodyParser`]でパースする。
pub(super) struct DeclarationBlockParser;

impl<'i> DeclarationParser<'i> for DeclarationBlockParser {
    type Declaration = Vec<PropertyDeclaration>;
    type Error = ();

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _declaration_start: &cssparser::ParserState,
    ) -> Result<Self::Declaration, ParseError<'i, Self::Error>> {
        parse_declaration(&name, input)
    }
}

impl<'i> QualifiedRuleParser<'i> for DeclarationBlockParser {
    type Prelude = ();
    type QualifiedRule = Vec<PropertyDeclaration>;
    type Error = ();
}

impl<'i> AtRuleParser<'i> for DeclarationBlockParser {
    type Prelude = ();
    type AtRule = Vec<PropertyDeclaration>;
    type Error = ();
}

impl<'i> RuleBodyItemParser<'i, Vec<PropertyDeclaration>, ()> for DeclarationBlockParser {
    fn parse_declarations(&self) -> bool {
        true
    }

    fn parse_qualified(&self) -> bool {
        false
    }
}

/// 判定に直前の兄弟が要るセレクタ。
///
/// ストリーミングは処理済みのトップレベル要素を解放するが、これらを使う文書では
/// 解放を子孫だけに絞り、要素そのものは兄弟として見えるように残す必要がある
/// ([`crate::html::Dom::release_descendants`])。
const NEEDS_PRECEDING: &[&str] = &[
    "+",
    "~",
    ":first-child",
    ":nth-child()",
    ":first-of-type",
    ":nth-of-type()",
    ":only-child",
    ":only-of-type",
];

/// 直前の兄弟を残しても、なお判定できないセレクタ。
///
/// いずれも「この先に同じ型の要素が続くか」を知る必要があるが、トップレベル
/// 要素が確定するのは次の兄弟が現れた時点なので、その先は分からない。
const STILL_UNSAFE: &[&str] = &[
    ":last-of-type",
    ":only-of-type",
    ":nth-last-child()",
    ":nth-last-of-type()",
    ":has(~ ...)",
];

/// スタイルシートが、判定に直前の兄弟を要するセレクタを含むか。
///
/// 含む場合、ストリーミング処理はトップレベル要素の解放を子孫だけに絞る。
/// 含まない場合は従来どおりサブツリーごと解放してよい。
///
/// 使うときだけ残すのは、残すと解放できないノードが積み上がるため。
/// 実測(トップレベル要素20万個)ではピークRSSが89.5MB→93.2MB、要素あたり
/// 約19バイト。ただし積み上がるぶん[`crate::html::MAX_NODES`]には当たりうるので、
/// トップレベル要素がその数を超える文書では、これらのセレクタを使うと
/// ノード数の上限エラーになる(使わなければ従来どおり上限に当たらない)。
pub fn needs_preceding_siblings(sheet: &Stylesheet) -> bool {
    scan_sheet(sheet)
        .iter()
        .any(|name| NEEDS_PRECEDING.contains(&name.as_str()))
}

/// `Mode::Streaming`では直前の兄弟を残してもなおバッチと結果が変わるセレクタが
/// 使われていれば、その名前を返す。
///
/// トップレベル要素が確定するのは次の兄弟が現れた時点なので、そこから先に同じ型の
/// 要素が続くかどうかは分からない。判定にそれが要るものは、余分にマッチしたり
/// 取りこぼしたりする。黙って結果が変わるのを避けるため、呼び出し側が警告を出すのに使う。
///
/// 逆に`:last-child`・`:empty`・`:has()`の子孫/直後の兄弟は、確定の条件
/// (次の兄弟が現れた)と一致するのでバッチと同じ結果になる。ここでは挙げない。
pub fn streaming_unsafe_selectors(sheet: &Stylesheet) -> Vec<String> {
    scan_sheet(sheet)
        .into_iter()
        .filter(|name| STILL_UNSAFE.contains(&name.as_str()))
        .collect()
}

/// スタイルシート中の、兄弟関係に依存するセレクタの名前を重複なく集める。
fn scan_sheet(sheet: &Stylesheet) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for rule in &sheet.rules {
        for selector in rule.selectors.slice() {
            scan_selector(selector, &mut found);
        }
    }
    found
}

fn scan_selector(selector: &Selector<SgSelectorImpl>, found: &mut Vec<String>) {
    for component in selector.iter_raw_match_order() {
        match component {
            Component::Combinator(Combinator::NextSibling) => push_once(found, "+"),
            Component::Combinator(Combinator::LaterSibling) => push_once(found, "~"),
            Component::Nth(data) => push_once(found, nth_name(data)),
            // 引数の中身も同じ規則で効くので辿る。
            Component::Is(list) | Component::Where(list) | Component::Negation(list) => {
                for inner in list.slice() {
                    scan_selector(inner, found);
                }
            }
            // `:has()`の中で問題になるのは`~`(後続の兄弟全部)だけ。子孫・`>`・`+`は
            // 確定した時点で見えている。
            Component::Has(relatives) => {
                for relative in relatives.iter() {
                    if relative
                        .selector
                        .iter_raw_match_order()
                        .any(|c| matches!(c, Component::Combinator(Combinator::LaterSibling)))
                    {
                        push_once(found, ":has(~ ...)");
                    }
                }
            }
            _ => {}
        }
    }
}

/// 判定に前後の兄弟が要るものだけを名前で返す。
/// `:last-child`(`is_function`が`false`の`LastChild`)だけは、確定の条件と
/// 一致するので安全。空文字を返して除く。
fn nth_name(data: &NthSelectorData) -> &'static str {
    match (data.ty, data.is_function) {
        (NthType::Child, false) => ":first-child",
        (NthType::Child, true) => ":nth-child()",
        (NthType::OfType, false) => ":first-of-type",
        (NthType::OfType, true) => ":nth-of-type()",
        (NthType::LastChild, false) => "",
        (NthType::LastChild, true) => ":nth-last-child()",
        (NthType::LastOfType, false) => ":last-of-type",
        (NthType::LastOfType, true) => ":nth-last-of-type()",
        (NthType::OnlyChild, _) => ":only-child",
        (NthType::OnlyOfType, _) => ":only-of-type",
    }
}

fn push_once(found: &mut Vec<String>, name: &str) {
    if name.is_empty() || found.iter().any(|f| f == name) {
        return;
    }
    found.push(name.to_string());
}

#[cfg(test)]
mod tests {

    /// 直前の兄弟が要るセレクタを検出する(検出したらDOMの解放を子孫だけに絞る)。
    #[test]
    fn selectors_that_need_preceding_siblings_are_detected() {
        for css in [
            "li:first-child { color: red }",
            "p:nth-child(2) { color: red }",
            "p:first-of-type { color: red }",
            "p:nth-of-type(2) { color: red }",
            "p:only-child { color: red }",
            "h1 + p { color: red }",
            "h1 ~ p { color: red }",
            ":is(p:first-child, span) { color: red }",
            ":not(p:first-child) { color: red }",
        ] {
            assert!(
                needs_preceding_siblings(&parse_stylesheet(css)),
                "検出できていない: {css}"
            );
        }
    }

    /// 直前の兄弟が要らないセレクタでは、従来どおりサブツリーごと解放してよい。
    #[test]
    fn ordinary_selectors_do_not_need_preceding_siblings() {
        for css in [
            "li:last-child { color: red }",
            "div:empty { color: red }",
            "a:hover { color: red }",
            "section:has(h1) { color: red }",
            "div > p { color: red }",
            "div p { color: red }",
        ] {
            assert!(
                !needs_preceding_siblings(&parse_stylesheet(css)),
                "余計に検出している: {css}"
            );
        }
    }

    /// 直前の兄弟を残してもなお判定できないセレクタだけを警告する。
    #[test]
    fn only_selectors_that_stay_broken_are_reported() {
        let sheet = parse_stylesheet(
            "p:last-of-type { color: red } p:only-of-type { color: blue } \
             p:nth-last-child(2) { color: green } p:nth-last-of-type(1) { color: teal } \
             div:has(~ h1) { color: navy }",
        );
        let found = streaming_unsafe_selectors(&sheet);
        for expected in [
            ":last-of-type",
            ":only-of-type",
            ":nth-last-child()",
            ":nth-last-of-type()",
            ":has(~ ...)",
        ] {
            assert!(
                found.contains(&expected.to_string()),
                "{expected} が漏れている: {found:?}"
            );
        }
    }

    /// 直前の兄弟を残せば正しくなるものは警告しない。過剰に警告すると、
    /// 外す必要のない利用者にまで`--streaming`を諦めさせる。
    #[test]
    fn selectors_that_stay_correct_are_not_reported() {
        let sheet = parse_stylesheet(
            "li:last-child { color: red } div:empty { color: green } \
             a:hover { color: blue } section:has(h1) { color: teal } \
             div:has(> p) { color: navy } h1:has(+ p) { color: olive } \
             h1 + p { color: gray } li:first-child { color: lime }",
        );
        assert!(
            streaming_unsafe_selectors(&sheet).is_empty(),
            "got: {:?}",
            streaming_unsafe_selectors(&sheet)
        );
    }

    /// `:is()`/`:where()`/`:not()`の引数の中も見る。
    #[test]
    fn nested_selector_lists_are_scanned() {
        let sheet = parse_stylesheet(":is(p:nth-last-of-type(2), span) { color: red }");
        assert_eq!(
            streaming_unsafe_selectors(&sheet),
            vec![":nth-last-of-type()"]
        );
    }

    use cssparser::ToCss;

    use super::*;

    #[test]
    fn media_print_and_all_rules_are_applied() {
        for query in ["print", "all", "print, screen", "not screen"] {
            let sheet = parse_stylesheet(&format!(
                "@media {query} {{ div {{ color: rgb(1, 2, 3); }} }}"
            ));
            assert_eq!(
                sheet.rules.len(),
                1,
                "@media {query} should apply its rules"
            );
        }
    }

    #[test]
    fn media_screen_only_rules_are_ignored() {
        let sheet = parse_stylesheet("@media screen { div { color: rgb(1, 2, 3); } }");
        assert!(
            sheet.rules.is_empty(),
            "@media screen should not apply its rules"
        );
    }

    #[test]
    fn media_with_only_a_feature_query_defaults_to_matching_all() {
        // 型を省略した特徴クエリ単体(`(min-width: 600px)`)は仕様上
        // `all and (min-width: 600px)`と同義。特徴クエリ自体は
        // 評価しないため、常にマッチする。
        let sheet = parse_stylesheet("@media (min-width: 600px) { div { color: rgb(1, 2, 3); } }");
        assert_eq!(sheet.rules.len(), 1);
    }

    #[test]
    fn media_rules_and_subsequent_rules_are_both_parsed() {
        let sheet = parse_stylesheet(
            "@media print { div { color: rgb(1, 2, 3); } } p { color: rgb(4, 5, 6); }",
        );
        assert_eq!(sheet.rules.len(), 2);
    }

    #[test]
    fn nested_media_rules_are_flattened() {
        let sheet =
            parse_stylesheet("@media print { @media all { div { color: rgb(1, 2, 3); } } }");
        assert_eq!(sheet.rules.len(), 1);
    }

    #[test]
    fn font_face_inside_a_matching_media_rule_is_still_recognized() {
        let sheet = parse_stylesheet(
            r#"@media print { @font-face { font-family: "Test"; src: url("test.ttf"); } }"#,
        );
        assert_eq!(sheet.font_faces.len(), 1);
    }

    #[test]
    fn parse_inline_style_parses_bare_declarations() {
        let decls = parse_inline_style("color: rgb(1, 2, 3); font-size: 14px");
        assert_eq!(decls.len(), 2);
        assert!(matches!(decls[0], PropertyDeclaration::Color(_)));
        assert!(matches!(decls[1], PropertyDeclaration::FontSize(_)));
    }

    #[test]
    fn parse_inline_style_ignores_unknown_properties() {
        let decls = parse_inline_style("not-a-real-property: 5px; color: rgb(1, 2, 3)");
        assert_eq!(decls.len(), 1);
        assert!(matches!(decls[0], PropertyDeclaration::Color(_)));
    }

    #[test]
    fn parse_stylesheet_ignores_at_import_and_keeps_subsequent_rules() {
        // `@import`は`@font-face`以外のat-ruleとして
        // `TopLevelRuleParser::parse_prelude`が拒否し、cssparserの
        // `StyleSheetParser`のエラー回復で読み飛ばされる想定。フェッチした
        // 外部CSSに`@import`が含まれていても、それ以降の通常ルールの
        // パースが継続されることを確認する。
        let sheet = parse_stylesheet(
            r#"@import url("other.css"); p { color: rgb(1, 2, 3); } div { color: rgb(4, 5, 6); }"#,
        );
        assert_eq!(
            sheet.rules.len(),
            2,
            "both rules after the ignored @import should still be parsed"
        );
    }

    #[test]
    fn parse_stylesheet_ignores_unrecognized_properties_with_url_values() {
        // `url()`参照を含む値を持つが、本実装が対応していないプロパティが
        // あっても、そのプロパティ宣言だけが無視され、同じルール内の他の宣言・
        // 後続のルールは正常にパースされることを確認する。
        let sheet =
            parse_stylesheet(r#"div { border-image: url("border.png") 30; color: rgb(1, 2, 3); }"#);
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(
            sheet.rules[0].declarations.len(),
            1,
            "the unrecognized border-image declaration should be skipped, \
             leaving only the color declaration"
        );
        assert!(matches!(
            sheet.rules[0].declarations[0],
            PropertyDeclaration::Color(_)
        ));
    }

    #[test]
    fn parse_inline_style_handles_empty_string() {
        assert!(parse_inline_style("").is_empty());
    }

    #[test]
    fn deeply_nested_calc_is_rejected_instead_of_overflowing_the_stack() {
        // 再帰下降パーサなので、深さがそのままスタックの消費になる。
        // 上限(32段)までは通し、それより深い値は宣言ごと捨てる。
        let nested = |n: usize| {
            format!(
                "p {{ padding-left: {}1px{} }}",
                "calc(".repeat(n),
                ")".repeat(n)
            )
        };
        assert_eq!(parse_stylesheet(&nested(32)).rules.len(), 1, "32段は通す");
        assert!(
            parse_stylesheet(&nested(33)).rules.is_empty(),
            "33段は捨てる"
        );

        // 括弧も`calc()`と同じ深さに数える。
        let parens = format!(
            "p {{ padding-left: calc({}1px{}) }}",
            "(".repeat(40),
            ")".repeat(40)
        );
        assert!(parse_stylesheet(&parens).rules.is_empty());
    }

    #[test]
    fn negative_padding_is_rejected() {
        // CSSでは`padding`に負の値を書けない。宣言ごと捨てる。
        for css in [
            "p { padding-left: -5px }",
            "p { padding: -5px }",
            "p { padding: 5px -5px }",
            "p { padding-inline-start: -5px }",
            "p { padding-block: -1em }",
            "p { padding-top: -10% }",
        ] {
            assert!(
                parse_stylesheet(css).rules.is_empty(),
                "negative padding should be dropped: {css}"
            );
        }
        // 0と正の値、`calc()`(符号は解決するまで決まらない)は通す。
        for css in [
            "p { padding-left: 0 }",
            "p { padding: 5px }",
            "p { padding-left: calc(10px - 20px) }",
        ] {
            assert_eq!(
                parse_stylesheet(css).rules.len(),
                1,
                "should be accepted: {css}"
            );
        }
    }

    // ===== CSS Nesting (#25) =====

    fn selector_texts(sheet: &Stylesheet) -> Vec<String> {
        sheet
            .rules
            .iter()
            .map(|r| r.selectors.to_css_string())
            .collect()
    }

    #[test]
    fn nested_rule_with_explicit_parent_selector_is_flattened() {
        let sheet = parse_stylesheet(".wrap { & .probe { color: rgb(1, 2, 3) } }");
        // 宣言を持たない親(`.wrap`)はルールとして残らない。
        assert_eq!(selector_texts(&sheet), [":is(.wrap) .probe"]);
        assert_eq!(sheet.rules[0].declarations.len(), 1);
    }

    #[test]
    fn nested_rule_without_parent_selector_is_a_descendant_of_the_parent() {
        let sheet = parse_stylesheet(".wrap { .probe { color: rgb(1, 2, 3) } }");
        assert_eq!(selector_texts(&sheet), [":is(.wrap) .probe"]);
    }

    #[test]
    fn nested_compound_parent_selector_is_flattened() {
        let sheet = parse_stylesheet(".wrap { &.probe { color: rgb(1, 2, 3) } }");
        assert_eq!(selector_texts(&sheet), [":is(.wrap).probe"]);
    }

    #[test]
    fn nested_rule_with_leading_combinator_is_flattened() {
        let sheet = parse_stylesheet(".list { > li { color: rgb(1, 2, 3) } }");
        assert_eq!(selector_texts(&sheet), [":is(.list) > li"]);
    }

    #[test]
    fn nested_type_selector_is_parsed_as_a_rule() {
        let sheet = parse_stylesheet(".wrap { p { color: rgb(1, 2, 3) } }");
        assert_eq!(selector_texts(&sheet), [":is(.wrap) p"]);
    }

    #[test]
    fn nested_selector_list_parent_is_kept_as_a_list() {
        let sheet = parse_stylesheet(".a, .b { .c { color: rgb(1, 2, 3) } }");
        assert_eq!(selector_texts(&sheet), [":is(.a, .b) .c"]);
    }

    #[test]
    fn deeper_nesting_is_flattened_in_source_order() {
        let sheet = parse_stylesheet(".a { .b { .c { color: rgb(1, 2, 3) } } }");
        assert_eq!(selector_texts(&sheet), [":is(:is(.a) .b) .c"]);
    }

    #[test]
    fn declarations_around_nested_rules_keep_their_source_order() {
        // 前の宣言は親ルール、ネストしたルール、後ろの宣言は親と同じセレクタの
        // 別ルールとしてこの順に並ぶ(後ろの宣言を先頭へ巻き上げない)。
        let sheet = parse_stylesheet(
            ".wrap { color: rgb(1, 2, 3); .probe { color: rgb(4, 5, 6) } margin-left: 5px }",
        );
        assert_eq!(
            selector_texts(&sheet),
            [".wrap", ":is(.wrap) .probe", ".wrap"]
        );
        assert!(matches!(
            sheet.rules[0].declarations[..],
            [PropertyDeclaration::Color(_)]
        ));
        assert!(matches!(
            sheet.rules[2].declarations[..],
            [PropertyDeclaration::MarginLeft(_)]
        ));
    }

    #[test]
    fn a_parent_with_only_nested_rules_has_no_trailing_rule() {
        let sheet = parse_stylesheet(".wrap { .probe { color: rgb(1, 2, 3) } }");
        assert_eq!(sheet.rules.len(), 1, "空の末尾ルールを作らない");
    }

    #[test]
    fn declarations_after_a_nested_rule_use_the_resolved_parent_selector() {
        // ネストしたルールより後ろの宣言は`&`と同じセレクタを持つ。
        // 親がセレクタリストなら`:is(.p, #q)`にまとまり、詳細度は
        // 最も強いセレクタ(ここでは`#q`の(1,0,0))で揃う。
        let sheet = parse_stylesheet(".p, #q { .c { color: rgb(1, 2, 3) } margin-left: 5px }");
        assert_eq!(selector_texts(&sheet), [":is(.p, #q) .c", ":is(.p, #q)"]);
        let trailing = &sheet.rules[1].selectors;
        assert_eq!(trailing.slice().len(), 1);
        assert_eq!(
            trailing.slice()[0].specificity(),
            parse_stylesheet("#q { color: rgb(1, 2, 3) }").rules[0]
                .selectors
                .slice()[0]
                .specificity()
        );
    }

    #[test]
    fn rules_without_declarations_are_dropped() {
        // 宣言の無いルールはカスケードに寄与しないので索引にも入れない。
        let sheet = parse_stylesheet(".a { } .b { color: rgb(1, 2, 3) } .c { }");
        assert_eq!(selector_texts(&sheet), [".b"]);
    }

    #[test]
    fn nested_rules_inside_media_are_flattened() {
        let sheet = parse_stylesheet("@media print { .wrap { .probe { color: rgb(1, 2, 3) } } }");
        assert_eq!(selector_texts(&sheet), [":is(.wrap) .probe"]);
    }

    #[test]
    fn an_invalid_nested_rule_is_dropped_without_its_siblings() {
        let sheet = parse_stylesheet(
            ".wrap { .probe::first-line { color: rgb(1, 2, 3) } color: rgb(4, 5, 6); .ok { color: rgb(7, 8, 9) } }",
        );
        assert_eq!(selector_texts(&sheet), [".wrap", ":is(.wrap) .ok"]);
        assert_eq!(sheet.rules[0].declarations.len(), 1);
    }

    #[test]
    fn an_invalid_declaration_next_to_nested_rules_is_still_skipped() {
        let sheet = parse_stylesheet(
            ".wrap { border-image: url(\"b.png\") 30; color: rgb(1, 2, 3); .probe { color: rgb(4, 5, 6) } }",
        );
        assert_eq!(selector_texts(&sheet), [".wrap", ":is(.wrap) .probe"]);
        assert_eq!(sheet.rules[0].declarations.len(), 1);
    }
}

#[cfg(test)]
mod layer_tests {
    use super::*;

    /// #20: Tailwind v4は出力全体を`@layer`ブロックで包む。ブロックごと捨てると
    /// 文書が丸ごと無装飾になるので、中のルールをトップレベルへ展開する。
    #[test]
    fn layer_block_rules_are_flattened() {
        let sheet = parse_stylesheet("@layer utilities { div { color: rgb(1, 2, 3); } }");
        assert_eq!(sheet.rules.len(), 1);
    }

    #[test]
    fn anonymous_layer_block_rules_are_flattened() {
        let sheet = parse_stylesheet("@layer { div { color: rgb(1, 2, 3); } }");
        assert_eq!(sheet.rules.len(), 1);
    }

    /// `@layer a.b { }`(入れ子を`.`で書く形)と`@layer a { @layer b { } }`。
    #[test]
    fn nested_layer_blocks_are_flattened() {
        for css in [
            "@layer a { @layer b { div { color: rgb(1, 2, 3); } } }",
            "@layer a.b { div { color: rgb(1, 2, 3); } }",
        ] {
            let sheet = parse_stylesheet(css);
            assert_eq!(sheet.rules.len(), 1, "{css}");
        }
    }

    /// `@layer theme, base, components, utilities;`(順序宣言だけの文)は
    /// 従来どおり無視し、後続のルールを壊さない。
    #[test]
    fn layer_statement_is_ignored_and_keeps_subsequent_rules() {
        for css in [
            "@layer base, utilities; div { color: rgb(1, 2, 3); }",
            "@layer base; div { color: rgb(1, 2, 3); }",
        ] {
            let sheet = parse_stylesheet(css);
            assert_eq!(sheet.rules.len(), 1, "{css}");
        }
    }

    #[test]
    fn layer_and_media_nest_either_way() {
        for css in [
            "@layer base { @media print { div { color: rgb(1, 2, 3); } } }",
            "@media print { @layer base { div { color: rgb(1, 2, 3); } } }",
        ] {
            let sheet = parse_stylesheet(css);
            assert_eq!(sheet.rules.len(), 1, "{css}");
        }
        let sheet =
            parse_stylesheet("@layer base { @media screen { div { color: rgb(1, 2, 3); } } }");
        assert!(
            sheet.rules.is_empty(),
            "@media screen inside @layer must still be dropped"
        );
    }

    #[test]
    fn font_face_and_page_inside_a_layer_are_still_recognized() {
        let sheet = parse_stylesheet(
            r#"@layer base {
                @font-face { font-family: "Test"; src: url("test.ttf"); }
                @page { margin: 10mm; }
            }"#,
        );
        assert_eq!(sheet.font_faces.len(), 1);
        assert_eq!(sheet.page_rules.len(), 1);
    }

    /// 展開は書かれた順を保つ(レイヤーの優先順位は実装しないので、通常の
    /// カスケード=後勝ちに委ねる)。
    #[test]
    fn layer_rules_keep_source_order() {
        let sheet = parse_stylesheet(
            "@layer a { div { color: rgb(1, 1, 1); } } \
             div { color: rgb(2, 2, 2); } \
             @layer b { div { color: rgb(3, 3, 3); } }",
        );
        let colors: Vec<String> = sheet
            .rules
            .iter()
            .map(|r| format!("{:?}", r.declarations[0]))
            .collect();
        assert_eq!(colors.len(), 3);
        assert!(colors[0].contains("red: 1,"), "{colors:?}");
        assert!(colors[1].contains("red: 2,"), "{colors:?}");
        assert!(colors[2].contains("red: 3,"), "{colors:?}");
    }

    /// `@layer`以外の非対応at-ruleは引き続きブロックごと無視される。
    #[test]
    fn other_unsupported_at_rule_blocks_are_still_dropped() {
        for css in [
            "@supports (display: grid) { div { color: rgb(1, 2, 3); } }",
            "@container (min-width: 1px) { div { color: rgb(1, 2, 3); } }",
            "@keyframes spin { from { color: rgb(1, 2, 3); } }",
        ] {
            let sheet = parse_stylesheet(css);
            assert!(sheet.rules.is_empty(), "{css}");
        }
    }

    /// 壊れたpreludeの`@layer`はブロックごと捨て、後続のルールは生かす。
    #[test]
    fn layer_with_invalid_prelude_is_dropped_and_subsequent_rules_kept() {
        let sheet = parse_stylesheet(
            "@layer 42 { div { color: rgb(1, 2, 3); } } p { color: rgb(4, 5, 6); }",
        );
        assert_eq!(sheet.rules.len(), 1);
    }
}
