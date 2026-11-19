"""Prepare pinned Emerald C input for the freestanding C2Rust frontend."""
import json
from pathlib import Path
import re
import struct
import subprocess


PATCHES = {"main": "reset-keys", "m4a": "m4a", "sound": "cry", "malloc": "heap"}
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


def aligned_globals(declarations):
    """Extract the effective alignment, including attributes inherited from a header."""
    result = {}
    for decl in declarations:
        if decl["kind"] != "VarDecl" or decl.get("storageClass") == "extern":
            continue
        for attr in decl.get("inner", ()):
            if attr["kind"] == "AlignedAttr":
                name = decl["name"]
                value = int(attr["inner"][0]["value"])
                result[name] = max(result.get(name, 1), value)
    return result


# Rust sets a static's alignment from its type. Keep that storage private so
# pointer arithmetic, sizeof, and all existing uses see the original C type.
# On ELF .size also describes the C object exactly when its allocation has
# trailing padding, e.g. a one-byte message aligned to a four-byte boundary.
ALIGNED_STATIC = """macro_rules! aligned_game_static {
    ($(#[$attr:meta])* $vis:vis static mut $name:ident: $ty:ty;
     $symbol:literal, $aligned:ident, $storage:ident, $init:ident) => {
        unsafe extern "C" {
            #[link_name = $symbol]
            $vis static mut $name: $ty;
        }
        $(#[$attr])*
        #[used]
        #[cfg_attr(any(target_vendor = "apple", target_os = "windows"), unsafe(export_name = $symbol))]
        static mut $storage: $aligned<$ty> = $aligned($init);
        #[cfg(not(any(target_vendor = "apple", target_os = "windows")))]
        core::arch::global_asm!(
            concat!(".globl ", $symbol),
            concat!(".set ", $symbol, ", {storage}"),
            concat!(".size ", $symbol, ", {size}"),
            storage = sym $storage, size = const core::mem::size_of::<$ty>(),
        );
    }
}
"""


def preserve_static_alignment(text, unit, alignments):
    if not alignments:
        return text
    decl = re.compile(r"^((?:#\[[^\n]+\]\n)*)((?:pub )?)static mut (\w+): ([^=]+?) =", re.M)
    used = set()

    def replace(match):
        attrs, visibility, name, ty = match.groups()
        if name not in alignments:
            return match[0]
        alignment = alignments[name]
        used.add(alignment)
        symbol = name if visibility else f"__rustic_{unit}_{name}"
        if export := re.search(r'#\[unsafe\(export_name = "([^"]+)"\)\]', attrs):
            symbol = export[1]
        storage_attrs = re.sub(r"^#\[unsafe\((?:no_mangle|export_name = .*?)\)\]\n", "", attrs, flags=re.M)
        storage = f"__RUSTIC_STORAGE_{name}"
        initial = f"__RUSTIC_INIT_{name}"
        return (f"aligned_game_static!(\n{storage_attrs}{visibility}static mut {name}: {ty};\n"
                f'    "{symbol}", RusticAligned{alignment}, {storage}, {initial}\n);\n'
                f"const {initial}: {ty} =")

    text = decl.sub(replace, text)
    if not used:
        return text
    types = "".join(f"#[repr(C, align({n}))]\nstruct RusticAligned{n}<T>(T);\n"
                    for n in sorted(used))
    return types + ALIGNED_STATIC + text
