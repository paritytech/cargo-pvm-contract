#!/bin/bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
EXAMPLE_DIR="$REPO_ROOT/examples/example-mytoken"
REPORT_FILE="$REPO_ROOT/examples/BINARY_SIZES.md"

"$REPO_ROOT/scripts/regenerate-examples.sh"

cd "$EXAMPLE_DIR"
env -u CARGO -u RUSTUP_TOOLCHAIN cargo build --release

python3 - "$EXAMPLE_DIR/target" "$REPORT_FILE" <<'PYTHON'
import pathlib
import sys

target = pathlib.Path(sys.argv[1])
report_path = pathlib.Path(sys.argv[2])

rows = [
    ("example-mytoken-dsl-no-alloc", "dsl-no-alloc"),
    ("example-mytoken-macro-no-alloc", "macro-no-alloc"),
    ("example-mytoken-macro-alloc", "macro-alloc"),
    ("example-mytoken-alloy-alloc", "alloy-alloc"),
]


def bytes_to_kb(size: int) -> str:
    return f"{size / 1024:.1f} KB"


def format_row(binary: str, flavor: str, profile: str) -> str:
    file_path = target / f"{binary}.{profile}.polkavm"
    if not file_path.exists():
        raise FileNotFoundError(f"Missing artifact: {file_path}")
    size = file_path.stat().st_size
    return f"| {binary} | {flavor} | {size:,} | {bytes_to_kb(size)} |"


release_rows = [format_row(binary, flavor, "release") for binary, flavor in rows]

content = "\n".join(
    [
        "# Binary Sizes",
        "",
        "PolkaVM binary sizes for `examples/example-mytoken` variants.",
        "",
        "## How to regenerate",
        "",
        "Run the helper script:",
        "",
        "```bash",
        "./scripts/regenerate-example-binary-sizes.sh",
        "```",
        "",
        "This script regenerates the examples, builds release artifacts, and rewrites this file.",
        "",
        "## Release Profile",
        "",
        "| Binary | Flavor | Size (bytes) | Size |",
        "|--------|--------|-------------:|-----:|",
        *release_rows,
        "",
    ]
)

report_path.write_text(content)
print(f"Updated {report_path}")
PYTHON
