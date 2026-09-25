#!/usr/bin/env bash
# 编译 uitap 并在 bin/ 下建稳定软链，供 CLI 与 MCP 配置引用。
# 切换实现（如 legacy-swift → Rust）只需改这里，.vscode/mcp.json 不用动。
set -euo pipefail
cd "$(dirname "$0")"

cargo build --release

mkdir -p bin
ln -sfn ../target/release/uitap bin/uitap

echo "built: $(pwd)/bin/uitap"
"$(pwd)/bin/uitap" doctor
