//! テスト専用のフォント細工。
//!
//! カラー絵文字フォント(`cmap`はあるが輪郭が無い)のような、リポジトリに
//! 置けない・置きたくないフォントを既存のテストフォントから合成する。

/// `data`から`drop`に挙げたテーブルを除いたTrueTypeを組み直す。
///
/// カラー絵文字フォント(`cmap`はあるが輪郭が無い)をリポジトリに置かずに
/// 再現するための細工。テーブルディレクトリを読み直して該当エントリを
/// 落とし、残りのテーブルを詰め直す。チェックサムは更新しない
/// (読み手が検証しないため)。
pub(super) fn without_tables(data: &[u8], drop: &[&[u8; 4]]) -> Vec<u8> {
    let num_tables = u16::from_be_bytes([data[4], data[5]]) as usize;
    let mut kept: Vec<(&[u8], &[u8])> = Vec::new();
    for i in 0..num_tables {
        let rec = &data[12 + i * 16..12 + i * 16 + 16];
        let tag = &rec[0..4];
        if drop.iter().any(|d| d.as_slice() == tag) {
            continue;
        }
        let offset = u32::from_be_bytes([rec[8], rec[9], rec[10], rec[11]]) as usize;
        let length = u32::from_be_bytes([rec[12], rec[13], rec[14], rec[15]]) as usize;
        kept.push((tag, &data[offset..offset + length]));
    }

    let count = kept.len();
    let mut out = Vec::new();
    out.extend_from_slice(&data[0..4]);
    out.extend_from_slice(&(count as u16).to_be_bytes());
    // searchRange/entrySelector/rangeShiftは読み飛ばされるので0で埋める。
    out.extend_from_slice(&[0u8; 6]);
    let mut body_offset = 12 + count * 16;
    let mut directory = Vec::new();
    let mut body = Vec::new();
    for (tag, table) in &kept {
        directory.extend_from_slice(tag);
        directory.extend_from_slice(&[0u8; 4]);
        directory.extend_from_slice(&(body_offset as u32).to_be_bytes());
        directory.extend_from_slice(&(table.len() as u32).to_be_bytes());
        body.extend_from_slice(table);
        // 4バイト境界へ揃える。
        let padding = (4 - table.len() % 4) % 4;
        body.extend(std::iter::repeat_n(0u8, padding));
        body_offset += table.len() + padding;
    }
    out.extend_from_slice(&directory);
    out.extend_from_slice(&body);
    out
}
