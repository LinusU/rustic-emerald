"""Use rustc's place-expression spans to lower unaligned C field accesses."""
from collections import defaultdict
from functools import lru_cache
import json
from pathlib import Path
import re
import subprocess
import tempfile

from normalize_rust import statement_end

ROOT = Path(__file__).resolve().parent.parent


@lru_cache(maxsize=1)
def compiler_flags():
    # Use Cargo's exact locked dependency artifacts, even on a fresh checkout.
    result = subprocess.check_output([
        "cargo", "build", "--locked", "--quiet", "--message-format=json",
        "--target-dir", ROOT / "target/translate-deps", "-p", "libc", "-p", "c2rust-bitfields",
    ], cwd=ROOT, text=True)
    crates = {}
    for line in result.splitlines():
        artifact = json.loads(line)
        if artifact["reason"] == "compiler-artifact":
            name = artifact["target"]["name"]
            if name in {"libc", "c2rust_bitfields"}:
                crates[name] = next(Path(f) for f in artifact["filenames"] if f.endswith(".rmeta"))
    return ["-L", f"dependency={crates['libc'].parent}",
            "--extern", f"libc={crates['libc']}",
            "--extern", f"c2rust_bitfields={crates['c2rust_bitfields']}"]


def diagnosed_places(path):
    result = subprocess.run([
        "rustc", path, "--crate-name", "translate_unit", "--crate-type=lib", "--emit=metadata",
        "--edition=2024", "--error-format=json", "-A", "warnings", "-A", "static_mut_refs",
        *compiler_flags(), "-o", path.with_suffix(".rmeta"),
    ], stderr=subprocess.PIPE, text=True)
    if result.returncode < 0 or result.returncode and not result.stderr:
        raise RuntimeError(f"rustc terminated during packed-field analysis ({result.returncode})")
    places = set()
    for line in result.stderr.splitlines():
        diagnostic = json.loads(line)
        code = (diagnostic.get("code") or {}).get("code")
        if code == "E0463":
            raise RuntimeError(diagnostic["message"])
        if code not in {"E0793", "E0596"}:
            continue
        for span in diagnostic["spans"]:
            if span["is_primary"] and span["file_name"] == str(path):
                places.add((span["byte_start"], span["byte_end"]))
    return sorted(places)


def lower_aliases(function, aliases):
    # "&raw" evaluates the C place once. Every subsequent use of the alias
    # observes current storage, including reads after other pointers wrote it.
    function = function.decode()
    for name in aliases:
        name = name.decode()
        decl = f"let ref mut {name} = "
        deref = rf"\*{name}\b"
        stores = list(re.finditer(deref + r"\s*=(?!=)\s*", function))
        for store in reversed(stores):
            end = statement_end(function, store.end())
            if end is None:
                raise ValueError(f"unterminated assignment to {name}")
            rhs = function[store.end():end]
            function = function[:store.start()] + f"{name}.write_unaligned({rhs})" + function[end:]
        # The remaining dereferences here load Copy pointers or integers.
        # Reject changes to this C2Rust grammar that invoke a setter through
        # the alias or take a reference to its storage.
        if re.search(r"\(" + deref + r"\)\s*\.set_|&(?:raw (?:mut|const) |mut )?" + deref, function):
            raise ValueError(f"unhandled mutation through {name}")
        function = re.sub(deref, name + ".read_unaligned()", function)
        pointer = "&raw mut" if stores else "&raw const"
        if function.count(decl) != 1:
            raise ValueError(f"ambiguous C2Rust temporary {name}")
        function = function.replace(decl, f"let {name} = {pointer} ")
    return function.encode()


def apply_places(data, places):
    # C2Rust closes each top-level item with an unindented brace. Group
    # temporary names by item because their spelling is local to generated code.
    getters = set(re.findall(rb'#\[bitfield\(name = "(\w+)"', data)) | {b"is_some", b"is_none"}
    aliases = defaultdict(set)
    edits = []
    for start, end in places:
        expr = data[start:end]
        name = re.fullmatch(rb"ref mut (fresh\d+)", expr)
        if name:
            boundary = data.rfind(b"\n}\n", 0, start)
            boundary = boundary + 3 if boundary >= 0 else 0
            aliases[boundary].add(name[1])
            continue
        tail = data[end:]
        method = re.match(rb"\s*\.(\w+)\(\)", tail)
        if (re.match(rb"\s*(?:==|!=)", tail)
                or method and method[1] in getters):
            # A block expression yields a copy before the comparison or
            # bitfield getter or Option predicate takes a shared reference.
            edits.append((start, end, b"({ " + expr + b" })"))
            continue
        setter = re.match(rb"\s*\.set_(\w+)\(", tail)
        if setter and setter[1] in getters:
            call = tail.decode()
            end_call = statement_end(call, 0)
            if end_call is None or call[end_call - 1] != ")":
                raise ValueError(f"unterminated bitfield setter {setter[1]!r}")
            argument = call[setter.end():end_call - 1].encode()
            size = len(call[:end_call + 1].encode())
            # The generated bitfield setter takes &mut self. Read the record
            # after evaluating the argument, update it, then store it unaligned.
            edits.append((start, end + size, b"{ let field = &raw mut " + expr
                          + b"; let argument = " + argument + b"; let mut value = field.read_unaligned(); "
                          + b"value.set_" + setter[1] + b"(argument); field.write_unaligned(value); }"))
            continue
        raise ValueError(f"unhandled unaligned expression {expr[-100:]!r} before {tail[:50]!r}")
    # Apply edits to each affected item in original byte coordinates before
    # rewriting aliases, which can change the lengths of whole statements.
    chunks = []
    cursor = 0
    for boundary, names in sorted(aliases.items()):
        end = data.index(b"\n}\n", boundary) + 3
        chunks.append((cursor, boundary, ()))
        chunks.append((boundary, end, names))
        cursor = end
    chunks.append((cursor, len(data), ()))
    output = []
    for start, end, names in chunks:
        block = data[start:end]
        for lo, hi, value in reversed(edits):
            if start <= lo < end:
                block = block[:lo-start] + value + block[hi-start:]
        output.append(lower_aliases(block, sorted(names)) if names else block)
    return b"".join(output)


def normalize_packed(text):
    if "let ref mut fresh" not in text and "packed(" not in text:
        return text
    data = text.encode()
    with tempfile.TemporaryDirectory() as temp:
        source = Path(temp) / "unit.rs"
        for _ in range(4):
            source.write_bytes(data)
            places = diagnosed_places(source)
            if not places:
                return data.decode()
            data = apply_places(data, places)
    raise ValueError("packed field normalization did not converge")
