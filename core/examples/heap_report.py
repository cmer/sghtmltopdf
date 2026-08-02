#!/usr/bin/env python3
"""`heap_profile`が出した`dhat-heap.json`を、ピーク時点の確保元ごとに集計する。

使い方:
    cargo run --release --example heap_profile 5000 table
    python3 core/examples/heap_report.py dhat-heap.json

dhatのJSONは確保地点(program point)ごとに、そのコールスタックと
「ピーク時点で生きていたバイト数(tgb)」を持っている。ここではそれを
自分のコードのフレームでまとめ直して、多い順に出す。
"""
import json
import re
import sys
from collections import defaultdict

# 自分のコードのフレームだけを見出しに使う(標準ライブラリの中で切らない)。
OWN = re.compile(r'sghtmltopdf_core::([\w:]+)')


def main(path: str, top: int = 15) -> None:
    data = json.loads(open(path).read())
    frames = data['ftbl']
    total_peak = 0
    by_site = defaultdict(lambda: [0, 0])  # 見出し -> [バイト, ブロック数]

    for pp in data['pps']:
        peak_bytes = pp.get('gb', 0)
        if not peak_bytes:
            continue
        total_peak += peak_bytes
        # スタックの下から上へ辿り、最初に見つかった自前の関数を見出しにする。
        label = '(自前のフレーム無し)'
        for index in pp['fs']:
            found = OWN.search(frames[index])
            if found:
                label = found.group(1)
                break
        entry = by_site[label]
        entry[0] += peak_bytes
        entry[1] += pp.get('gbk', 0)

    print(f'ピーク時点の合計: {total_peak / 1024 / 1024:.1f}MB\n')
    print(f'{"確保元":<52} {"MB":>8} {"件数":>10}')
    ranked = sorted(by_site.items(), key=lambda kv: kv[1][0], reverse=True)
    for label, (size, blocks) in ranked[:top]:
        print(f'{label:<52} {size / 1024 / 1024:>8.1f} {blocks:>10,}')


def detail(path: str, needle: str, top: int = 8) -> None:
    """`needle`を含むスタックの確保地点を、ピーク時のバイト数順に出す。"""
    data = json.loads(open(path).read())
    frames = data['ftbl']
    rows = []
    for pp in data['pps']:
        peak_bytes = pp.get('gb', 0)
        stack = [frames[i] for i in pp['fs']]
        if not peak_bytes or not any(needle in f for f in stack):
            continue
        rows.append((peak_bytes, pp.get('gbk', 0), stack))
    rows.sort(reverse=True, key=lambda r: r[0])
    for size, blocks, stack in rows[:top]:
        avg = size / blocks if blocks else 0
        print(f'{size / 1024 / 1024:.1f}MB  {blocks:,}件  平均{avg:.0f}B')
        for frame in stack[:6]:
            print(f'    {frame[:150]}')
        print()


if __name__ == '__main__':
    if len(sys.argv) > 2:
        detail(sys.argv[1], sys.argv[2])
    else:
        main(sys.argv[1] if len(sys.argv) > 1 else 'dhat-heap.json')
