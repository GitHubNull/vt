#!/usr/bin/env python3
"""把发布版本号同步进 package.json / src-tauri/tauri.conf.json / src-tauri/Cargo.toml。

用法: python3 sync-version.py <version>   # version 不带前导 v，如 0.2.0
跨平台（Linux/macOS/Windows）统一走 Python，避免 sed 方言与 shell 差异。
"""
import json
import re
import sys
from pathlib import Path

# Windows 控制台默认非 UTF-8 编码，打印中文会抛 UnicodeEncodeError
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(encoding="utf-8", errors="replace")

version = sys.argv[1]
if not re.fullmatch(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(-[0-9A-Za-z-.]+)?(\+[0-9A-Za-z-.]+)?", version):
    sys.exit(f"版本号不符合语义化规范: {version}")

root = Path(__file__).resolve().parent.parent.parent

pkg_path = root / "package.json"
pkg = json.loads(pkg_path.read_text(encoding="utf-8"))
pkg["version"] = version
pkg_path.write_text(json.dumps(pkg, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

conf_path = root / "src-tauri" / "tauri.conf.json"
conf = json.loads(conf_path.read_text(encoding="utf-8"))
conf["version"] = version
conf_path.write_text(json.dumps(conf, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")

cargo_path = root / "src-tauri" / "Cargo.toml"
cargo = cargo_path.read_text(encoding="utf-8")
cargo_new, n = re.subn(r'^version = "[^"]*"', f'version = "{version}"', cargo, count=1, flags=re.M)
if n != 1:
    sys.exit("src-tauri/Cargo.toml 中未找到 [package] version 行")
cargo_path.write_text(cargo_new, encoding="utf-8")

print(f"版本号已同步: {version}")
