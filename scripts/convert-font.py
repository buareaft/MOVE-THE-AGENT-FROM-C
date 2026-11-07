"""Convert the pinned full Noto Sans SC font; do not subset user-visible glyphs."""
from pathlib import Path
from fontTools.ttLib import TTFont
import hashlib
import json

root = Path(__file__).resolve().parent.parent
source = root / ".tools/font-source/NotoSansSC-variable.ttf"
assets = root / "src/assets/fonts"
manifest_path = assets / "sources.json"
manifest = json.loads(manifest_path.read_text(encoding="utf-8-sig"))
assert hashlib.sha256(source.read_bytes()).hexdigest().upper() == manifest["notoOriginalTtfSha256"], "Unexpected source font"
font = TTFont(source)
font.flavor = "woff2"
destination = assets / "noto-sans-sc-variable.woff2"
font.save(destination)
output = TTFont(destination)
assert font.getBestCmap() == output.getBestCmap(), "Conversion changed the character map"
assert "wght" in [axis.axisTag for axis in output["fvar"].axes]
assert all(ord(c) in output.getBestCmap() for c in "空间总览文件项目对话迁移目录帮助")
manifest["notoWoff2Sha256"] = hashlib.sha256(destination.read_bytes()).hexdigest()
manifest["notoGlyphCount"] = len(output.getBestCmap())
manifest["conversion"] = "fonttools 4.68.0; brotli 1.2.1; complete character map, no subset"
manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
print(json.dumps({"file": destination.name, "bytes": destination.stat().st_size,
                  "characters": len(output.getBestCmap())}))
