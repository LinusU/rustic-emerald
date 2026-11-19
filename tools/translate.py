#!/usr/bin/env python3
"""Regenerate native game modules from the pinned C sources with C2Rust 0.22.1."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent
VENDOR = ROOT / "vendor/pokeemerald"
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
    raw = subprocess.check_output([*CLANG, *FLAGS, "-E", source], cwd=VENDOR)
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
    text = text.replace("#[no_mangle]", "#[unsafe(no_mangle)]")
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
    # Keep the C order of evaluation: compute the place, then evaluate the
    # new value. C2Rust introduces these adjacent temporaries for assignments.
    writes = r"let ref mut (fresh\d+) = ([^;]+);(\s*)\*\1 = ([^;]+);"
    def store(match):
        if match[1] in match[4] or re.search(r"\b" + match[1] + r"\b", text[match.end():]):
            return match[0]
        return f"let {match[1]} = &raw mut {match[2]};{match[3]}{match[1]}.write_unaligned({match[4]});"
    text = re.sub(writes, store, text)
    refs = re.findall(r"let ref mut (fresh\d+) = \(\*if [^;]+?\}\)\s*\.\w+;", text)
    for name in refs:
        # Only the null-tested pointer value is being copied. Preserve aliases
        # to records: methods on those records may update live game state.
        if not re.search(r"\(\*" + name + r"\)\s*\.is_null\(\)", text):
            continue
        if re.search(r"\*" + name + r"\s*[-|&+*/]?=(?!=)", text):
            continue
        text = text.replace(f"let ref mut {name} =", f"let {name} =")
        text = re.sub(r"\*" + name + r"\b", name, text)

    readonly = const_globals(source)
    def mark_state(match):
        # The native engine restores this linker section when creating a game.
        return ("" if match[2] in readonly else SECTION) + match[0]
    text = re.sub(r"^(pub )?static mut (\w+):", mark_state, text, flags=re.M)

    layouts = record_layouts(source)
    checks = []
    records = re.findall(r"^pub (?:struct|union) (\w+) \{(.*?)^\}", text, re.M | re.S)
    for name, body in records:
        if name not in layouts:
            continue
        size, alignment, offsets = layouts[name]
        if alignment <= 4:
            # C2Rust can discard pragma pack when it encounters an earlier
            # forward declaration of the same record.
            text = re.sub(r"#\[repr\(C\)\](\npub (?:struct|union) " + name + r" \{)",
                          rf"#[repr(C, packed({alignment}))]\1", text)
        if "#[bitfield(" in body and alignment > 1:
            # The derive stores C bitfields as byte arrays. Restore their C
            # alignment, including in records nested inside packed save data.
            text = re.sub(r"\b" + name + r" \{\n", lambda m: m[0] + (
                f"    pub _c_alignment: [u{alignment * 8}; 0],\n"
                if text[max(0, m.start() - 7):m.start()].endswith("struct ")
                else "    _c_alignment: [],\n"), text)
        if name == "Main":
            # The native renderer requests a 16-byte-aligned OAM array.
            text = text.replace("    pub oamBuffer: [OamData; 128],",
                                "    pub _oam_alignment: [u128; 0],\n    pub oamBuffer: [OamData; 128],")
        checks.extend([f'assert!(size_of::<{name}>() == {size}, "size of {name}");',
                       f'assert!(align_of::<{name}>() == {alignment}, "alignment of {name}");'])
        fields = re.findall(r"^    pub (\w+):", body, re.M)
        for field in fields:
            if field in offsets:
                checks.append(f'assert!(offset_of!({name}, {field}) == {offsets[field]}, "{name}.{field}");')
    if checks:
        text += "\n#[cfg(target_pointer_width = \"64\")]\nconst _: () = {\n"
        text += "    use core::mem::{align_of, offset_of, size_of};\n"
        text += "".join("    " + check + "\n" for check in checks) + "};\n"
    return "// Generated by tools/translate.py from " + source.name + ".\n\n" + text


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="verify generated files byte for byte")
    args = parser.parse_args()
    version = subprocess.check_output(["c2rust-transpile", "--version"], text=True).strip()
    if version.split()[-1] != "0.22.1":
        raise SystemExit(f"Expected C2Rust 0.22.1, found {version}")
    expected = {"mod.rs", *(unit + ".rs" for unit in UNITS)}
    unexpected = sorted(path.name for path in OUT.glob("*.rs") if path.name not in expected)
    if unexpected:
        raise SystemExit("Unexpected Rust modules in src/translated: " + ", ".join(unexpected))
    # build.rs uses the same file lock for the shared native assets and tools.
    lock_path = VENDOR / "build/native/rustic-build.lock"
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    with lock_path.open("a+b") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        subprocess.run(["make", "-s", "-f", "make_tools.mk", "tools"], cwd=VENDOR, check=True)
        subprocess.run(["make", "-s", "-f", "Makefile", "-f", "../../tools/build-core.mk",
                        "NODEP=1", "generated", "wasm-assets"], cwd=VENDOR, check=True)
    with tempfile.TemporaryDirectory() as directory:
        tmp = Path(directory).resolve()
        generated = {}
        for unit in UNITS:
            source = prepare(unit, tmp)
            db = tmp / "compile_commands.json"
            db.write_text(json.dumps([{"directory": str(tmp), "file": str(source),
                "arguments": [*CLANG, "-fwrapv", "-Wno-everything", "-c", str(source)]}]))
            subprocess.run(["c2rust-transpile", "--fail-on-error", "--disable-rustfmt", "--emit-modules",
                            "--translate-const-macros", "none", "--translate-fn-macros", "none", db],
                           check=True)
            generated[unit + ".rs"] = normalize(source.with_suffix(".rs").read_text(), source)
        generated["mod.rs"] = ("// Generated from the C2Rust units listed in tools/rust-units.txt.\n"
                               "#![allow(warnings, static_mut_refs, clippy::all)]\n\n"
                               + "".join(f"#[rustfmt::skip]\nmod {unit};\n" for unit in UNITS))
        stale = []
        for name, result in generated.items():
            path = OUT / name
            if args.check:
                if not path.is_file() or path.read_text() != result:
                    stale.append(name)
            else:
                OUT.mkdir(exist_ok=True)
                path.write_text(result)
        if stale:
            raise SystemExit("Regenerate " + ", ".join(stale))
        print(f"{'Verified' if args.check else 'Generated'} {len(UNITS)} C2Rust units.")


if __name__ == "__main__":
    main()
