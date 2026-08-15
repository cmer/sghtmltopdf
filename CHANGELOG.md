# CHANGELOG

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Fixed

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

### Added

- Support `<wbr>`, and U+200B ZERO WIDTH SPACE, as a line break opportunity. Neither
  adds width nor leaves a character in the PDF text layer.

## 0.1.1

### Changed

- Change gemspec from Japanese to English.
- Change required Ruby version for precompiled gem.

## 0.1.0

### Added

- 1st release on 2026-08-08
