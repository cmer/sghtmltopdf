# CHANGELOG

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Fixed

- Streaming mode (`--streaming`) ignored `break-before` / `break-after` on top-level elements
- A flex or grid item with padding or a border measured its content one line short, so wrapped text overflowed the item box
- Text fell back to a bold face for its whole script when a bold run came first (Japanese in a document starting with a bold heading was rendered bold throughout)
- A float was painted below the background of a following block, which hid it
- Ligature glyphs mapped to a single character in the `/ToUnicode` CMap, so text extraction and search dropped characters (`float` came out as `foat`)

## 0.1.0

### Added

- 1st release on 2026-08-02
