# CHANGELOG

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Added

- Map the logical box properties to their physical sides (#21). `margin-inline-start`
  becomes `margin-left`, `padding-block` becomes `padding-top` and `padding-bottom`, and so
  on for `margin-*`, `padding-*`, `inset-*` and `border-*` (including the `-width`, `-style`
  and `-color` longhands), plus the `inset` shorthand. The engine only supports
  `horizontal-tb` LTR, so the mapping is fixed rather than driven by `writing-mode`.
  Tailwind v4 emits these for `px-*`, `py-*`, `mx-auto` and `space-y-*`, and until now a
  document built with it silently lost its horizontal padding and every centred block.

### Fixed

- Parse nested style rules (CSS Nesting) instead of silently dropping them (#25).
  `.wrap { & .probe { } }`, `.wrap { .probe { } }`, `.wrap { &.probe { } }` and
  `.list { > li { } }` now reach the cascade with the meaning the spec gives them; `&`
  takes the parent's specificity, and declarations written after a nested rule keep
  their source position instead of being hoisted above it. Nested at-rules such as
  `@media` inside a style rule are still ignored.
- Accept `calc()` as a term inside another `calc()` (#17). CSS Values 4 treats a nested
  `calc()` the same as a parenthesised group, but the parser only handled the parentheses,
  so `calc(calc(45px * 2) * calc(1 - 0))` was rejected as invalid and the declaration was
  dropped while `calc((45px * 2) * (1 - 0))` resolved to 90px. Tailwind v4 emits the nested
  form for every `space-y-*` and `divide-*` utility, so a Tailwind bundle lost all of its
  vertical rhythm and divider gaps.
- Stop rounding flex and grid item sizes to whole pixels (#15). taffy rounds its final
  layout to integers so that a rasteriser does not leave gaps or overlaps between boxes;
  the output here is PDF, which has no such constraint, and the rounding truncated the
  measured max-content width so that text which fit was wrapped onto a second line. Which
  way the fraction rounded depended on the exact string, so the same row wrapped for one
  value and not for another (`1 USD = 0.9143 EUR` wrapped, `1 USD 0.9143 EUR` did not).

### Added

- Render SVG referenced from `<img src>` and `background-image: url()`, as vector graphics
  rather than by rasterising. Parsing goes through [usvg] and the translation to PDF
  drawing operators through [svg2pdf], both from typst. An SVG becomes a form XObject
  normalised to the unit square, so the existing drawing, `object-fit`, background tiling
  and per-`src` caching all apply unchanged. `.svgz` (gzipped) is accepted too.
  Behind the `svg` feature, on by default.
- The `svg-text` feature (off by default) renders `<text>` inside an SVG as embedded,
  selectable glyphs, using **the document's own fonts**. Whatever is available to the HTML
  is available inside the SVG, resolved the same way: by the font's internal family name, by
  the name it is declared under in CSS (`@font-face`), or through the generic families —
  `serif` / `sans-serif` / `monospace` in an SVG land on `--serif-font` / `--gothic-font` /
  `--mono-font`. Text with no `font-family`, and text naming a family the document does not
  have, both fall back to the document's default font. No separate system-font scan happens
  for SVG, so an SVG can never come out in a font the document never had.

  It is off by default because enabling it adds 25 crates (rustybuzz, resvg and friends,
  pulled in by svg2pdf's `text` feature). Without it, text inside an SVG is **not drawn at
  all** — not even converted to paths, since svg2pdf discards text nodes outright. That is
  now reported: an SVG containing `<text>` warns once per document. usvg and svg2pdf do log
  it themselves, but through the `log` crate, and this crate installs no logger, so it never
  reached anyone.

- Accept non-base64 `data:` URIs. A payload without `;base64` is now percent-decoded per
  RFC 2397 instead of rejected. This is how SVG data URIs are normally written
  (`data:image/svg+xml,%3Csvg...%3E`), in `<img src>` and CSS `url()` alike; requiring
  base64 made the common form fail. Tabs and newlines are dropped from the payload (as the
  URL standard does) but spaces are kept, since they separate tokens in an unencoded SVG.

### Changed

- Pinned `pdf-writer` to 0.12 so that svg2pdf's `Chunk` is the same type as the one used to
  write the document, which is what lets an SVG be spliced in without going through bytes.
  No API of ours changed as a result.
- `PreparedImage`'s intrinsic size is now `f32` rather than `u32`. An SVG's intrinsic size
  can be fractional (`width="40.6"`, a fractional `viewBox`), and rounding it changed the
  aspect ratio — 40.6×10.4 became 41×10, a 5% error that visibly skewed `object-fit`
  (`contain` gave a height of 24.4 instead of 25.6) and the height derived from a
  `width`-only rule. Raster sizes are unaffected: they are whole pixels either way.
- An inline `<svg>` in the HTML now warns once per document instead of silently rendering
  nothing. It is still not drawn — only `<img>` and `background-image` references are — but
  saying "SVG is supported" and then dropping inline SVG without a word was misleading.

### Known limitations

- SVG filters (`<filter>`) and raster images inside an SVG (`<image>`) are not drawn.
  Filters would require rasterising, which this deliberately avoids.
- Inline `<svg>` written directly in the HTML is not rendered; reference the SVG from
  `<img>` or `background-image` instead. Supporting it means rebuilding SVG XML out of the
  HTML DOM and deciding how attribute case (`viewBox`), CSS inheritance and `currentColor`
  carry across — a different problem from referencing a file, so this phase covers only
  references.
- `--grayscale` does not apply to SVG. It warns and leaves the SVG in colour.
- External references from inside an SVG (`<image href="...">`) are refused with a warning
  rather than resolved. usvg's default resolver reads such an href straight off disk, which
  would bypass the containment that applies to `<img>` (base directory, `--allow`,
  `--disable-local-file-access`), so the path is closed off entirely. `data:` URIs are
  unaffected, being self-contained.

[usvg]: https://github.com/linebender/resvg
[svg2pdf]: https://github.com/typst/svg2pdf

## 0.2.0 - 2026-08-16

### Added

- Support the `:has()`, `:is()` and `:where()` selectors (#10). Specificity follows the
  spec: `:is()` and `:has()` count as their most specific argument and `:where()` counts as
  zero, and the argument list of `:is()` / `:where()` is forgiving. In streaming mode
  `:has(~ ...)` cannot be decided and warns.
- Support `color-mix()` (#11), in the `srgb`, `srgb-linear`, `lab`, `oklab`, `xyz`, `hsl`,
  `hwb`, `lch` and `oklch` colour spaces with all four hue interpolation methods. Weight
  normalisation and premultiplied alpha follow the spec. Wide-gamut spaces and
  `currentcolor` operands are rejected; see the docs for why.
- Accept `data:` URIs and `http(s)` URLs in the `src: url()` of `@font-face` (#5). They are
  resolved through the same fetcher, `<base href>` handling and access control as `<img>`,
  `<link>` and `@import`.
- Support `<wbr>`, and U+200B ZERO WIDTH SPACE, as a line break opportunity. Neither
  adds width nor leaves a character in the PDF text layer.

### Changed

- Decline a font that has no glyph outlines, with a warning naming the font, instead of
  selecting it (#9). Colour emoji fonts such as Noto Color Emoji are bitmap-only: font
  selection consulted `cmap` alone, so such a font was chosen as one that could draw the
  character, and the result was text that vanished entirely rather than showing tofu, with
  no warning, a PDF inflated to the size of the source font because subsetting had nothing
  to strip, and an embedded font some readers refused to parse. Emoji now fall back to tofu
  with a warning naming the characters. Colour emoji rendering itself is tracked in #12; a
  monochrome outline font such as Noto Emoji works today through `--font`.
- Report only the selectors that actually behave differently in streaming mode. The warning
  used to name `:last-child` and `:empty`, which are correct there, while staying silent
  about `+`, `~` and `:first-child`, which were not.

### Fixed

- Measure the natural width of a nested table, flex or grid box instead of treating it as
  zero (#5). A grid or flex container nested inside another one collapsed to zero width,
  so its content overflowed one word per line. This was never specific to grid-in-grid:
  flex-in-flex, flex-in-grid, grid-in-flex and any of those inside a table cell took the
  same path.
- Let `auto` grid tracks absorb the leftover width. `justify-content` had `flex-start` as
  its initial value internally, which is not the same as the initial `normal` and stopped
  the tracks from stretching.
- Collect absolutely positioned descendants of a flex item, a grid item, a table cell and
  an `inline-block` (#5). They were laid out through helpers that discarded them, so the
  element was silently dropped.
- Keep the preceding siblings of a processed top-level element visible in streaming mode.
  The subtree was released as soon as it had been laid out, so every later element looked
  like the first child: `+` and `~` stopped matching and `:first-child` matched everything.
  The nodes are now kept when the stylesheet needs them, which costs about 19 bytes per
  top-level element and nothing at all otherwise.
- Memoise the natural width and the measured height of each box. Deeply nested flex and
  grid re-measured the same subtree once per ancestor level, growing exponentially with
  depth; a five-level structure repeated 200 times went from 0.15 s to 0.04 s.
- Keep the whitespace between two inline elements, so `<span>one</span> <span>two</span>`
  renders as `one two` instead of `onetwo` (#3).
- Collapse only the whitespace CSS Text 3 says is collapsible (space, tab, newline).
  `&nbsp;` and the other Unicode spaces are no longer collapsed into a single space
  and keep their own advance width, so `&nbsp;&nbsp;&nbsp;` is three spaces wide and
  thin/hair/em spaces are no longer all rendered as one plain space.
- Do not wrap around `&nbsp;`, narrow no-break space, figure space or word joiner
  (UAX #14 glue), including under `word-break: break-all`. Thin space and friends
  offer a wrap opportunity after them, and U+200B ZERO WIDTH SPACE now provides a
  zero-width break opportunity inside a word.
- Treat a cell holding only `&nbsp;` as non-empty, so `empty-cells: hide` no longer
  strips the borders of a `<td>&nbsp;</td>`.
- Do not let a glyph shared by several characters lose its `/ToUnicode` mapping to
  whichever character happened to come first in the document. A font without its own
  `&nbsp;` glyph made every space in the document extract as U+00A0, breaking
  copy-paste and text search in the PDF.
- Draw glyphs at the advance width the layout used. A PDF advances a glyph by its
  single `/W` entry, which cannot express the two cases where the shaper reports a
  different advance for the same glyph: the stretched word gaps of a justified line,
  and a fixed-width space (`&thinsp;` and friends) that the font has no glyph for.
  A `text-align: justify` line was drawn short of the right edge by the whole stretch
  amount, and text following a substituted fixed-width space was drawn off its laid
  out position. The difference is now made up with `TJ` adjustments.
- Keep the leading whitespace of a `white-space: pre` element when it comes from a
  whitespace-only text node, so `<pre>   <b>x</b>y</pre>` keeps its indentation
  instead of rendering as `xy`.

## 0.1.1

### Changed

- Change gemspec from Japanese to English.
- Change required Ruby version for precompiled gem.

## 0.1.0

### Added

- 1st release on 2026-08-08
