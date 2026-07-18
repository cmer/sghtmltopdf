//! PDFバイト列の書き出し先を抽象化するSink trait。
//!
//! エンジンは「バイト列をどこかに書き出す」ことだけを知っていればよく、
//! 書き出し先が何であるか(メモリ/ファイル/将来的なRackレスポンスやS3
//! マルチパートアップロード等)は一切気にしない設計にする。
//!
//! M1では一括変換(ストリーミングなし)のため、実際には`write`が1回だけ
//! 呼ばれる形になる。複数回に分けてflushする本格的なストリーミング対応は
//! マイルストーン3で行う。

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
}
