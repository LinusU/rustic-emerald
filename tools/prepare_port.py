"""Prepare pinned Emerald C input for the freestanding C2Rust frontend."""
import json
from pathlib import Path
import re
import struct
import subprocess


PATCHES = {"main": "reset-keys", "m4a": "m4a", "sound": "cry"}
ASSET_UNITS = {"graphics", "anim_mon_front_pics", "tilesets"}
# These top-level declarations use a single asset as their entire initializer.
# Compound palette tables, structure initializers, etc. still go through C2Rust.
ASSET = re.compile(
    r'(?P<prefix>\bconst\s+u(?P<bits>8|16|32)\s+(?P<symbol>\w+))\s*\[\s*\]\s*=\s*'
    r'INC(?P<kind>GFX|BIN)_U(?P=bits)\(\s*"(?P<path>[^"]+)"'
    r'(?:\s*,\s*"(?P<suffix>[^"]+)")?(?:\s*,\s*"(?P<args>[^"]*)")?\s*\)\s*;'
)


def preprocess(unit, tmp, vendor, clang, flags):
    source = vendor / "src" / f"{unit}.c"
    if unit in PATCHES:
        patched = tmp / f"{unit}.input.c"
        patch = vendor.parent.parent / "native" / (PATCHES[unit] + ".patch")
        result = subprocess.check_output(["patch", "-s", "-o", "-", source, patch])
        patched.write_bytes(result)
        source = patched
    if unit == "libisagbprn" and "-DNDEBUG" in flags:
        # All debug printers (including variadic calls) are excluded by the
        # native game's NDEBUG build. Its unconditional stdio include adds no
        # declarations used by the resulting unit and freestanding has no libc.
        patched = tmp / "libisagbprn.input.c"
        patched.write_text(source.read_text().replace("#include <stdio.h>", ""))
        source = patched
    raw = subprocess.check_output(
        [*clang, *flags, "-iquote", vendor / "src", "-E", source], cwd=vendor)
    if unit not in ASSET_UNITS:
        return raw

    # clang's JSON AST gets hundreds of megabytes of integer nodes if compressed
    # graphics go through the text preprocessor. Retain C's element count and
    # type for sizeof, declarations, pointer arithmetic and dependent tables.
    assets = {}
    def placeholder(match):
        path = Path(match["path"])
        if match["kind"] == "GFX":
            if match["suffix"] is None:
                raise ValueError(f"missing INCGFX suffix for {path}")
            encoded = re.sub(r"[^a-zA-Z0-9]", "_", match["args"] or "")
            path = Path("build/assets") / (str(path) + encoded + match["suffix"])
        elif match["suffix"] is not None:
            return match[0]  # concatenated INCBINs
        size = (vendor / path).stat().st_size
        width = int(match["bits"]) // 8
        if size % width:
            raise ValueError(f"unaligned {match['bits']}-bit asset {path}")
        assets[match["symbol"]] = [str(path), width]
        return f"{match['prefix']}[{size // width}] = {{0}};"

    raw = ASSET.sub(placeholder, raw.decode())
    (tmp / f"{unit}.assets.json").write_text(json.dumps(assets))
    return raw.encode()


def restore_assets(text, source):
    manifest = source.with_suffix(".assets.json")
    if not manifest.exists():
        return text
    assets = json.loads(manifest.read_text())
    # The vendored preprocessor treats every element as little endian. Emit
    # the numbers themselves so a clean build of a Rust crate does not have
    # to wait for another crate's build script to convert the graphics.
    vendor = Path(__file__).resolve().parent.parent / "vendor/pokeemerald"
    declaration = re.compile(r"^(?:pub )?static (?:mut )?(\w+):[^=\n]+=", re.M)
    out = []
    cursor = 0
    for match in declaration.finditer(text):
        asset = assets.pop(match[1], None)
        if asset is None:
            continue
        path, width = asset
        words = [f"0x{word:0{width * 2}x}" for (word,) in struct.iter_unpack(
            {1: "<B", 2: "<H", 4: "<I"}[width], (vendor / path).read_bytes())]
        out.append(text[cursor:match.end()])
        out.append(" [\n")
        out.extend("    " + ", ".join(words[i:i+16]) + ",\n" for i in range(0, len(words), 16))
        out.append("]")
        cursor = text.index(";\n", match.end())
    if assets:
        raise ValueError(f"missing Rust asset declaration in {source.name}: {next(iter(assets))}")
    return "".join(out) + text[cursor:]
