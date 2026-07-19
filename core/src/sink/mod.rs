//! PDFバイト列の書き出し先を抽象化するSink trait。
//!
//! エンジンは「バイト列をどこかに書き出す」ことだけを知っていればよく、
//! 書き出し先が何であるか(メモリ/ファイル/将来的なRackレスポンスや
//! マルチパートアップロード等)は一切気にしない設計にする。
//!
//! マイルストーン3のストリーミング対応により、`write`はページ確定の
//! たびに複数回呼ばれるようになった(`pdf::StreamingPdfWriter`参照)。
//! `MemorySink`/`FileSink`はいずれも複数回の`write`に対応済み。

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

pub trait Sink {
    type Output;
    type Error;

    fn write(&mut self, bytes: &[u8]) -> Result<(), Self::Error>;
    fn finish(self) -> Result<Self::Output, Self::Error>;
}

/// テスト・同期返却モード向けのメモリバッファSink。
#[derive(Debug, Default)]
pub struct MemorySink {
    buf: Vec<u8>,
}

impl MemorySink {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Sink for MemorySink {
    type Output = Vec<u8>;
    type Error = io::Error;

    fn write(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        self.buf.extend_from_slice(bytes);
        Ok(())
    }

    fn finish(self) -> Result<Self::Output, Self::Error> {
        Ok(self.buf)
    }
}

/// ファイルへ書き出すSink(CLI向け)。
pub struct FileSink {
    file: File,
}

impl FileSink {
    pub fn create(path: impl AsRef<Path>) -> io::Result<Self> {
        Ok(Self {
            file: File::create(path)?,
        })
    }
}

impl Sink for FileSink {
    type Output = ();
    type Error = io::Error;

    fn write(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        self.file.write_all(bytes)
    }

    fn finish(self) -> Result<Self::Output, Self::Error> {
        Ok(())
    }
}

/// マルチパートアップロードの最小パートサイズ(最後のパートを除く)。
pub const MULTIPART_MIN_PART_SIZE: usize = 5 * 1024 * 1024;

/// `threshold`バイト溜まるごとに`on_part`を呼ぶSink。
///
/// CLAUDE.mdが挙げる「5MB溜めてマルチパートPUTするバッファ付きSink」
/// ([`MULTIPART_MIN_PART_SIZE`]を`threshold`に使う想定)向けの汎用実装。
/// マルチパートアップロードは最後のパート以外に最小サイズ制約がある
/// ため、flush boundary由来の小さいPDFチャンクをそのまま都度PUTするのでは
/// なく、閾値まで溜めてからパートとしてまとめて渡す必要がある。
///
/// コアはRuby/AWS SDKに一切依存しない設計方針(CLAUDE.md参照)のため、
/// 実際のアップロード処理(HTTP PUT等)自体は行わない。
/// `on_part`コールバックへ1パート分のバイト列を渡すところまでがこの型の
/// 責務で、ストレージサービスへの実際のPUT呼び出しはFFI層(Ruby bindings)が`on_part`の
/// 中で行う想定。
///
/// 最後のパート(`finish`が呼ばれた時点でバッファに残っている端数)は
/// `threshold`未満でもそのまま`on_part`に渡す(最後のパートは最小サイズ未満が許される)。
/// バッファがちょうど0バイトで`finish`を迎えた場合は、空のパートを送らない。
pub struct BufferedSink<T, E, F: FnMut(Vec<u8>) -> Result<T, E>> {
    buf: Vec<u8>,
    threshold: usize,
    on_part: F,
    parts: Vec<T>,
}

impl<T, E, F: FnMut(Vec<u8>) -> Result<T, E>> BufferedSink<T, E, F> {
    /// `threshold`バイト溜まるごとに`on_part`を呼ぶ`BufferedSink`を作る。
    /// `threshold`が0の場合、`write`のたびに毎回1バイト単位で`on_part`が
    /// 呼ばれることになり非効率なため、呼び出し側で正の値を渡すこと。
    pub fn new(threshold: usize, on_part: F) -> Self {
        Self {
            buf: Vec::new(),
            threshold,
            on_part,
            parts: Vec::new(),
        }
    }
}

impl<T, E, F: FnMut(Vec<u8>) -> Result<T, E>> Sink for BufferedSink<T, E, F> {
    /// 各パートを`on_part`に渡した際の戻り値(例: ETag)の一覧。
    type Output = Vec<T>;
    type Error = E;

    fn write(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        self.buf.extend_from_slice(bytes);
        while self.threshold > 0 && self.buf.len() >= self.threshold {
            let part: Vec<u8> = self.buf.drain(..self.threshold).collect();
            self.parts.push((self.on_part)(part)?);
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Self::Output, Self::Error> {
        if !self.buf.is_empty() {
            self.parts.push((self.on_part)(self.buf)?);
        }
        Ok(self.parts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_sink_accumulates_written_bytes() {
        let mut sink = MemorySink::new();
        sink.write(b"hello, ").unwrap();
        sink.write(b"world").unwrap();
        assert_eq!(sink.finish().unwrap(), b"hello, world");
    }

    #[test]
    fn file_sink_writes_to_disk() {
        let dir =
            std::env::temp_dir().join(format!("sghtmltopdf-sink-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.bin");

        let mut sink = FileSink::create(&path).unwrap();
        sink.write(b"pdf bytes").unwrap();
        sink.finish().unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"pdf bytes");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn buffered_sink_does_not_flush_a_part_before_the_threshold_is_reached() {
        let mut parts_seen: Vec<Vec<u8>> = Vec::new();
        let mut sink: BufferedSink<(), io::Error, _> = BufferedSink::new(10, |part| {
            parts_seen.push(part);
            Ok(())
        });
        sink.write(b"12345").unwrap();
        sink.write(b"6789").unwrap();

        assert!(
            parts_seen.is_empty(),
            "9 bytes written should not cross the 10-byte threshold yet"
        );
    }

    #[test]
    fn buffered_sink_flushes_a_part_once_the_threshold_is_crossed() {
        let mut parts_seen: Vec<Vec<u8>> = Vec::new();
        let mut sink: BufferedSink<(), io::Error, _> = BufferedSink::new(10, |part| {
            parts_seen.push(part);
            Ok(())
        });
        sink.write(b"12345").unwrap();
        sink.write(b"67890").unwrap();

        assert_eq!(parts_seen, vec![b"1234567890".to_vec()]);
    }

    #[test]
    fn buffered_sink_flushes_multiple_parts_from_a_single_large_write() {
        let mut parts_seen: Vec<Vec<u8>> = Vec::new();
        let mut sink: BufferedSink<(), io::Error, _> = BufferedSink::new(3, |part| {
            parts_seen.push(part);
            Ok(())
        });
        sink.write(b"abcdefghi").unwrap();

        assert_eq!(
            parts_seen,
            vec![b"abc".to_vec(), b"def".to_vec(), b"ghi".to_vec()]
        );
    }

    #[test]
    fn buffered_sink_flushes_the_remaining_partial_data_on_finish() {
        let mut parts_seen: Vec<Vec<u8>> = Vec::new();
        let mut sink: BufferedSink<(), io::Error, _> = BufferedSink::new(10, |part| {
            parts_seen.push(part);
            Ok(())
        });
        sink.write(b"hello").unwrap();
        sink.finish().unwrap();

        assert_eq!(
            parts_seen,
            vec![b"hello".to_vec()],
            "the final short part (5 bytes, below the 10-byte threshold) should still \
             be flushed on finish, as the last part is allowed to be smaller"
        );
    }

    #[test]
    fn buffered_sink_does_not_send_an_empty_final_part() {
        let mut parts_seen: Vec<Vec<u8>> = Vec::new();
        let mut sink: BufferedSink<(), io::Error, _> = BufferedSink::new(5, |part| {
            parts_seen.push(part);
            Ok(())
        });
        sink.write(b"12345").unwrap();
        sink.finish().unwrap();

        assert_eq!(
            parts_seen,
            vec![b"12345".to_vec()],
            "exactly one full part should be sent, with no trailing empty part on finish"
        );
    }

    #[test]
    fn buffered_sink_preserves_byte_order_across_parts() {
        let mut parts_seen: Vec<Vec<u8>> = Vec::new();
        let mut sink: BufferedSink<(), io::Error, _> = BufferedSink::new(4, |part| {
            parts_seen.push(part);
            Ok(())
        });
        for chunk in [b"ab".as_slice(), b"cdef".as_slice(), b"gh".as_slice(), b"i"] {
            sink.write(chunk).unwrap();
        }
        sink.finish().unwrap();

        let reassembled: Vec<u8> = parts_seen.concat();
        assert_eq!(reassembled, b"abcdefghi");
    }

    #[test]
    fn buffered_sink_propagates_errors_from_on_part() {
        let mut sink: BufferedSink<(), io::Error, _> =
            BufferedSink::new(4, |_part| Err(io::Error::other("upload failed")));
        let result = sink.write(b"abcd");
        assert!(result.is_err());
    }

    #[test]
    fn buffered_sink_output_collects_each_parts_return_value() {
        // `on_part`の戻り値(例: ETag)がOutputとして順番に集約されること。
        let mut next_etag = 0u32;
        let mut sink: BufferedSink<u32, io::Error, _> = BufferedSink::new(4, |_part| {
            next_etag += 1;
            Ok(next_etag)
        });
        sink.write(b"abcdefgh").unwrap();
        let etags = sink.finish().unwrap();

        assert_eq!(etags, vec![1, 2]);
    }
}
