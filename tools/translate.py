#!/usr/bin/env python3
"""Regenerate native game modules from the pinned C sources with C2Rust 0.22.1."""
import argparse
from concurrent.futures import ProcessPoolExecutor
import fcntl
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

from prepare_port import preprocess, restore_assets
from translation_crates import manifests

ROOT = Path(__file__).resolve().parent.parent
VENDOR = ROOT / "vendor/pokeemerald"
C2RUST = os.environ.get("C2RUST_TRANSPILE", str(ROOT / "target/c2rust/bin/c2rust-transpile"))
OUT = ROOT / "src/translated"
UNITS = (ROOT / "tools/rust-units.txt").read_text().splitlines()
# A fixed freestanding LP64 target keeps system headers out of generated Rust.
CLANG = [os.environ.get("CLANG", "clang"), "--target=aarch64-none-elf", "-ffreestanding"]
FLAGS = ["-DMODERN=1", "-DWASM=1", "-DNATIVE=1", "-DNDEBUG", "-fwrapv",
         "-fno-strict-aliasing", "-Wno-everything", "-Ibuild/wasm", "-Iinclude/wasm", "-Iinclude"]
SECTION = """#[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,native_game_data"))]
#[cfg_attr(not(target_vendor = "apple"), unsafe(link_section = "native_game_data"))]
"""


def prepare(unit, tmp):
    source = f"src/{unit}.c"
    raw = preprocess(unit, tmp, VENDOR, CLANG, FLAGS)
    # These two headers declare exactly the same global callback. C2Rust's
    # AST pruning otherwise removes the typedef needed by the second spelling.
    if b"extern void (*gPostMenuFieldCallback)(void);" in raw:
        raw = raw.replace(b"extern MainCallback gPostMenuFieldCallback;", b"")
    # The natural C layout of this radio packet is already 14 bytes aligned
    # to two. The redundant attributes make C2Rust invent an inner/outer
    # wrapper pair and then confuse their types when passing packet pointers.
    raw = raw.replace(b"struct __attribute__((packed, aligned(2))) RfuGameData",
                      b"struct RfuGameData")
    # The Magnitude animation writes and branches on slot 15. Upstream's
    # eight-element declaration makes that script use an out-of-bounds slot.
    raw = raw.replace(b"s16 gBattleAnimArgs[8]", b"s16 gBattleAnimArgs[16]")
    if unit == "sprite":
        # C permits tentative array definitions before a sized initializer.
        # C2Rust keeps an incomplete bound even after seeing the initializer.
        for table in (b"gDummySpriteAnimTable", b"gDummySpriteAffineAnimTable"):
            raw = raw.replace(table + b"[]", table + b"[1]")
    if unit == "item_menu":
        # Make the nested bitfield read explicit before C2Rust lowers the
        # outer Sprite bitfield assignment. Both values are read once.
        raw = re.sub(rb"gSprites\[(gBagMenu->spriteIds\[[^;]*?itemIconSlot[^;]*?\])\]\.invisible = ",
                     rb"u8 iconSprite = \1; gSprites[iconSprite].invisible = ", raw)
    path = tmp / f"{unit}.i"
    path.write_bytes(subprocess.check_output(
        ["tools/preproc/preproc", "-i", "-g", "build/assets", source, "charmap.txt"],
        cwd=VENDOR, input=raw))
    prepared = tmp / f"{unit}.c"
    subprocess.run(["python3", "tools/native/prepare_source.py", path, prepared,
                    "elf", *CLANG], cwd=VENDOR, check=True)
    return prepared


def record_layouts(source):
    dump = subprocess.check_output([*CLANG, "-Xclang", "-fdump-record-layouts-complete",
                                    "-Wno-everything", "-fsyntax-only", source], text=True)
    records = {}
    for record in dump.split("*** Dumping AST Record Layout"):
        lines = record.strip().splitlines()
        if not lines:
            continue
        name = re.fullmatch(r"0 \| (?:struct|union) (\w+)", lines[0])
        if not name:
            continue
        size, alignment = re.search(r"\[sizeof=(\d+), align=(\d+)\]", lines[-1]).groups()
        fields = {}
        for line in lines[1:-1]:
            field = re.fullmatch(r"\s*(\d+) \|   (?! ).+ (\w+)", line)
            if field:
                fields[field[2]] = int(field[1])
        records[name[1]] = (int(size), int(alignment), fields)
    return records


def const_globals(source):
    ast = subprocess.check_output([*CLANG, "-Xclang", "-ast-dump=json", "-Wno-everything",
                                   "-fsyntax-only", source], text=True)
    result = set()
    for decl in json.loads(ast)["inner"]:
        if decl["kind"] != "VarDecl":
            continue
        ty = re.sub(r"\[[^\]]*\]", "", decl["type"]["qualType"])
        # For pointer tables the final pointer's qualifiers control writability.
        if "const" in ty.rsplit("*", 1)[-1].split():
            result.add(decl["name"])
    return result


def normalize(text, source):
    from normalize_rust import align_dodrio_fields, normalize_records, packed_temporaries
    from packed_access import normalize_packed
    text = text.replace("#[no_mangle]", "#[unsafe(no_mangle)]")
    text = re.sub(r'#\[export_name = ("[^"]+")\]', r'#[unsafe(export_name = \1)]', text)
    text = text.replace('extern "C" {', 'unsafe extern "C" {')
    text = text.replace("use ::c2rust_bitfields;", "use c2rust_bitfields::BitfieldStruct;")
    text = re.sub(r', link_section = ("[^"]+")\)\]', r', unsafe(link_section = \1))]', text)
    # C2Rust sometimes emits an extra usize-to-usize cast for the DMA helpers'
    # sizeof operand. Normalize both equivalent spellings and their line wraps.
    text = re.sub(r"size = \(size as usize\)\s*\.wrapping_div\("
                  r"(::core::mem::size_of::<\w+>\(\))(?:\s+as usize)*\)"
                  r"(?:\s+as u32_0)+;",
                  r"size = (size as usize).wrapping_div(\1) as u32_0;", text)

    # C2Rust takes references during comparisons and when expanding GNU ?:.
    # A packed C pointer member can be copied, but cannot be borrowed by Rust.
    text = text.replace("gTasks[i as usize].func == func", "{ gTasks[i as usize].func } == func")
    text = re.sub(r"(gSprites\[[^;\]]+\]\s*\.callback)(\s*==)", r"{ \1 }\2", text)
    text = "\n}\n".join(packed_temporaries(block) for block in text.split("\n}\n"))

    readonly = const_globals(source)
    def mark_state(match):
        # The native engine restores this linker section when creating a game.
        return ("" if match[2] in readonly else SECTION) + match[0]
    text = re.sub(r"^(pub )?static mut (\w+):", mark_state, text, flags=re.M)

    if source.stem == "apprentice":
        # C2Rust loses the getter on the bitfield index of the two
        # suggestedChange assignments in TrySetApprenticeHeldItem.
        text = text.replace(".playerApprentice as ::core::ffi::c_int",
                            ".playerApprentice.questionsAnswered() as ::core::ffi::c_int")
    if source.stem == "dodrio_berry_picking":
        text = align_dodrio_fields(text, source.read_text())
    text = normalize_packed(normalize_records(text, record_layouts(source)))
    text = restore_assets(text, source)
    return ("// Generated by tools/translate.py from " + source.name + ".\n\n" + text).rstrip() + "\n"


def translate_unit(unit, directory):
    tmp = Path(directory) / unit
    tmp.mkdir()
    source = prepare(unit, tmp)
    db = tmp / "compile_commands.json"
    db.write_text(json.dumps([{"directory": str(tmp), "file": str(source),
        "arguments": [*CLANG, "-fwrapv", "-Wno-everything", "-c", str(source)]}]))
    subprocess.run([C2RUST, "--fail-on-error", "--disable-rustfmt", "--emit-modules",
                    "--translate-const-macros", "none", "--translate-fn-macros", "none", db],
                   check=True)
    result = normalize(source.with_suffix(".rs").read_text(), source)
    (tmp / "normalized.rs").write_text(result)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify generated files byte for byte")
    parser.add_argument("--jobs", type=int, default=3, help="parallel translation processes")
    parser.add_argument("units", nargs="*", help="regenerate only these registered game files")
    args = parser.parse_args()
    if args.jobs < 1:
        parser.error("--jobs must be positive")
    units = args.units or UNITS
    if unknown := set(units).difference(UNITS):
        parser.error("unknown unit(s): " + ", ".join(sorted(unknown)))
    if not Path(C2RUST).is_file():
        raise SystemExit("Build the pinned translator with python3 tools/c2rust/bootstrap.py")
    version = subprocess.check_output([C2RUST, "--version"], text=True).strip()
    if version.split()[-1] != "0.22.1":
        raise SystemExit(f"Expected C2Rust 0.22.1, found {version}")

    generated = manifests(UNITS)
    expected = set(generated) | {unit + ".rs" for unit in UNITS}
    unexpected = sorted(str(path.relative_to(OUT)) for path in OUT.rglob("*.rs")
                        if str(path.relative_to(OUT)) not in expected)
    if unexpected:
        raise SystemExit("Unexpected Rust modules: " + ", ".join(unexpected))
    lock_path = VENDOR / "build/native/rustic-build.lock"
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    with lock_path.open("a+b") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        subprocess.run(["make", "-s", "-f", "make_tools.mk", "tools"], cwd=VENDOR, check=True)
        subprocess.run(["make", "-s", "-f", "Makefile", "-f", "../../tools/build-core.mk",
                        "NODEP=1", "generated", "wasm-assets"], cwd=VENDOR, check=True)
    stale = []
    def emit(name, result):
        path = OUT / name
        if path.is_file() and path.read_text() == result:
            return
        if args.check:
            stale.append(name)
        else:
            path.parent.mkdir(exist_ok=True, parents=True)
            path.write_text(result)

    # Stage the full batch before replacing committed output. One failed
    # conversion must leave the last working target intact.
    with tempfile.TemporaryDirectory() as directory:
        with ProcessPoolExecutor(max_workers=args.jobs) as pool:
            jobs = [pool.submit(translate_unit, unit, directory) for unit in units]
            for job in jobs:
                job.result()
        for unit in units:
            emit(unit + ".rs", (Path(directory) / unit / "normalized.rs").read_text())
        for name, result in generated.items():
            emit(name, result)
    if stale:
        raise SystemExit("Regenerate " + ", ".join(stale))
    print(f"{'Verified' if args.check else 'Generated'} {len(units)} C2Rust units.")


if __name__ == "__main__":
    main()
