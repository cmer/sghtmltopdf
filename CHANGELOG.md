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

## 0.1.1

### Changed

- Change gemspec from Japanese to English.
- Change required Ruby version for precompiled gem.

## 0.1.0

### Added

- 1st release on 2026-08-08
