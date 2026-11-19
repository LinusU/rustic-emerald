"""Split the translated game into small crates so Cargo can compile it in parallel."""
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "src/translated"
ALL_UNITS = sorted(source.stem for source in (ROOT / "vendor/pokeemerald/src").glob("*.c"))
# Base the partition on the pinned C tree, so ports never move between crates
# when another C file is translated. A crate contains at most twenty units.
GROUPS = [ALL_UNITS[i:i + 20] for i in range(0, len(ALL_UNITS), 20)]
HEADER = "// Generated from the C2Rust units listed in tools/rust-units.txt.\n"
PACKAGE = """[package]
name = "{name}"
version = "0.1.0"
edition = "2024"
rust-version = "1.89"
publish = false

[lib]
path = "mod.rs"

[dependencies]
"""


def manifests(units):
    linked = set(units)
    unknown = linked.difference(ALL_UNITS)
    if unknown:
        raise ValueError(f"Unknown Emerald source files: {sorted(unknown)}")
    files = {}
    deps = []
    imports = []
    for index, group in enumerate(GROUPS, start=1):
        name = f"emerald-game-{index:02}"
        folder = f"groups/{index:02}"
        deps.append(f'{name} = {{ path = "{folder}" }}\n')
        imports.append(f'use {name.replace("-", "_")} as _;\n')
        files[f"{folder}/Cargo.toml"] = PACKAGE.format(name=name) + 'c2rust-bitfields = "=0.22.1"\nlibc = "0.2"\n'
        files[f"{folder}/mod.rs"] = (
            HEADER + "#![allow(warnings, static_mut_refs, clippy::all)]\n\n"
            + "".join(f'#[rustfmt::skip]\n#[path = "../../{unit}.rs"]\nmod {unit};\n'
                      for unit in group if unit in linked)
        )
    files["Cargo.toml"] = PACKAGE.format(name="emerald-game") + "".join(deps)
    files["mod.rs"] = HEADER + "".join(imports)
    return files


def write(units):
    for filename, text in manifests(units).items():
        path = OUT / filename
        path.parent.mkdir(parents=True, exist_ok=True)
        if not path.exists() or path.read_text() != text:
            path.write_text(text)
