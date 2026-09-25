#!/usr/bin/env python3
"""Swift 版与 Rust 版的输出对拍。

两版必须给出逐字节相同的 JSON（动态字段如文件路径、时间戳除外）。
只依赖标准库，测试图由本脚本自己生成。

用法：
    python3 parity.py                 # 用默认的两个二进制
    python3 parity.py --rust PATH --swift PATH
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def write_png(path: Path, width: int, height: int, rows: list[list[tuple[int, int, int, int]]]) -> None:
    """写一个 8 位 RGBA、无过滤的 PNG。"""

    def chunk(tag: bytes, data: bytes) -> bytes:
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    raw = b"".join(
        b"\x00" + b"".join(bytes(pixel) for pixel in row) for row in rows
    )
    header = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    blob = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(raw))
        + chunk(b"IEND", b"")
    )
    path.write_bytes(blob)


def make_fixtures(directory: Path) -> tuple[Path, Path]:
    """200x100 白底；b 图在 (40,60) 加 30x20 红块、(150,10) 加 10x10 蓝块。"""
    width, height = 200, 100
    white = (255, 255, 255, 255)
    base = [[white for _ in range(width)] for _ in range(height)]
    changed = [row[:] for row in base]

    for y in range(60, 80):
        for x in range(40, 70):
            changed[y][x] = (255, 0, 0, 255)
    for y in range(10, 20):
        for x in range(150, 160):
            changed[y][x] = (0, 0, 255, 255)

    a = directory / "a.png"
    b = directory / "b.png"
    write_png(a, width, height, base)
    write_png(b, width, height, changed)
    return a, b


def run(binary: Path, args: list[str]) -> tuple[int, object]:
    proc = subprocess.run(
        [str(binary), *args], capture_output=True, text=True, timeout=60
    )
    text = proc.stdout.strip().splitlines()
    payload = None
    if text:
        try:
            payload = json.loads(text[-1])
        except json.JSONDecodeError:
            payload = {"__unparsed__": text[-1]}
    return proc.returncode, payload


def normalize(value: object, drop: set[str]) -> object:
    if isinstance(value, dict):
        return {
            k: normalize(v, drop) for k, v in value.items() if k not in drop
        }
    if isinstance(value, list):
        return [normalize(v, drop) for v in value]
    return value


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--rust", default=str(ROOT / "target" / "release" / "uitap"))
    parser.add_argument("--swift", default=str(ROOT / "legacy-swift" / ".build" / "release" / "uitap"))
    args = parser.parse_args()

    rust = Path(args.rust)
    swift = Path(args.swift)
    missing = [(name, path) for name, path in (("rust", rust), ("swift", swift)) if not path.exists()]
    if missing:
        for name, path in missing:
            print(f"缺少 {name} 二进制：{path}", file=sys.stderr)
        print("\n先构建：", file=sys.stderr)
        print("  cargo build --release", file=sys.stderr)
        if any(name == "swift" for name, _ in missing):
            print("  swift build -c release --package-path legacy-swift", file=sys.stderr)
        return 2

    work = Path(tempfile.mkdtemp(prefix="uitap-parity-"))
    try:
        fixture_a, fixture_b = make_fixtures(work)

        cases: list[tuple[str, list[str], set[str]]] = [
            ("diff 有变化", ["diff", "--before", str(fixture_a), "--after", str(fixture_b), "--units", "pixel"], set()),
            ("diff 无变化", ["diff", "--before", str(fixture_a), "--after", str(fixture_a), "--units", "pixel"], set()),
            ("diff 限制区域", ["diff", "--before", str(fixture_a), "--after", str(fixture_b), "--units", "pixel", "--region", "0,0,100,100"], set()),
            ("diff 过滤小区域", ["diff", "--before", str(fixture_a), "--after", str(fixture_b), "--units", "pixel", "--minPixels", "200"], set()),
            ("pixel 像素坐标", ["pixel", "--path", str(fixture_a), "--at", "10,10", "--at", "150,15", "--units", "pixel"], set()),
            ("pixel 越界", ["pixel", "--path", str(fixture_a), "--at", "9999,10", "--units", "pixel"], set()),
            ("screen 列表", ["screens"], set()),
            ("前台应用", ["frontmost"], set()),
            ("授权自检", ["doctor"], set()),
            ("窗口列表", ["windows", "--layer", "0", "--minWidth", "300", "--limit", "8"], set()),
            ("窗口无匹配", ["windows", "--app", "NoSuchAppZZZ"], set()),
        ]

        failures = 0
        for label, argv, drop in cases:
            swift_code, swift_out = run(swift, argv)
            rust_code, rust_out = run(rust, argv)
            same_code = swift_code == rust_code
            same_body = normalize(swift_out, drop) == normalize(rust_out, drop)

            if same_code and same_body:
                print(f"  ok   {label}")
                continue

            failures += 1
            print(f"  FAIL {label}")
            if not same_code:
                print(f"       退出码 swift={swift_code} rust={rust_code}")
            if not same_body:
                print(f"       swift: {json.dumps(normalize(swift_out, drop), ensure_ascii=False)[:300]}")
                print(f"       rust : {json.dumps(normalize(rust_out, drop), ensure_ascii=False)[:300]}")

        # 截图只比对元数据，路径等动态字段忽略。
        for label, extra in (
            ("截图 区域", ["--region", "0,0,200,150"]),
            ("截图 主屏", []),
        ):
            swift_path = work / "swift-shot.png"
            rust_path = work / "rust-shot.png"
            swift_code, swift_out = run(swift, ["shot", *extra, "--path", str(swift_path)])
            rust_code, rust_out = run(rust, ["shot", *extra, "--path", str(rust_path)])
            drop = {"path"}
            if swift_code == rust_code and normalize(swift_out, drop) == normalize(rust_out, drop):
                print(f"  ok   {label}")
            else:
                failures += 1
                print(f"  FAIL {label}")
                print(f"       swift: {json.dumps(normalize(swift_out, drop), ensure_ascii=False)[:300]}")
                print(f"       rust : {json.dumps(normalize(rust_out, drop), ensure_ascii=False)[:300]}")

        total = len(cases) + 2
        passed = total - failures
        print(f"\n{passed}/{total} 通过")
        return 1 if failures else 0
    finally:
        shutil.rmtree(work, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
