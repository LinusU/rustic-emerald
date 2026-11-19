"""Normalize the native ABI constructs which C2Rust cannot express directly."""
import re
from collections import Counter


def packed_temporaries(function):
    """Preserve C evaluation order without borrowing a misaligned field."""
    if "let ref mut fresh" not in function:
        return function
    counts = Counter(re.findall(r"\bfresh\d+\b", function))
    writes = r"let ref mut (fresh\d+) = ([^;]+);(\s*)\*\1 = ([^;]+);"

    def store(match):
        name, place, indent, value = match.groups()
        if counts[name] != 2:
            return match[0]
        return f"let {name} = &raw mut {place};{indent}{name}.write_unaligned({value});"

    function = re.sub(writes, store, function)
    # GNU's ?: pointer fallback can be evaluated using a local value. Record
    # aliases must remain live, as a method call may mutate the original record.
    fields = set(re.findall(r"let ref mut (fresh\d+) = \(\*if [^;]+?\}\)\s*\.\w+;", function))
    checked = set(re.findall(r"\(\*(fresh\d+)\)\s*\.is_null\(\)", function))
    written = set(re.findall(r"\*(fresh\d+)\s*[-|&+*/]?=(?!=)", function))
    reads = (fields & checked) - written
    if reads:
        function = re.sub(r"let ref mut (fresh\d+) =", lambda m:
                          f"let {m[1]} =" if m[1] in reads else m[0], function)
        function = re.sub(r"\*(fresh\d+)\b", lambda m: m[1] if m[1] in reads else m[0], function)
    return function


def normalize_records(text, layouts):
    """Apply native record alignments and assert the Clang ABI for named fields."""
    checks = []
    aligns = {}
    records = re.findall(r"^pub (?:struct|union) (\w+) \{(.*?)^\}", text, re.M | re.S)
    for name, body in records:
        if name not in layouts:
            continue
        size, alignment, offsets = layouts[name]
        if "#[bitfield(" in body and alignment > 1:
            aligns[name] = alignment
        checks.extend([f'assert!(size_of::<{name}>() == {size}, "size of {name}");',
                       f'assert!(align_of::<{name}>() == {alignment}, "alignment of {name}");'])
        for field in re.findall(r"^    pub (\w+):", body, re.M):
            if field in offsets:
                checks.append(f'assert!(offset_of!({name}, {field}) == {offsets[field]}, "{name}.{field}");')

    # An earlier forward declaration can cause C2Rust to lose pragma pack.
    def with_packing(match):
        layout = layouts.get(match[2])
        if layout and layout[1] <= 4:
            return f"#[repr(C, packed({layout[1]}))]" + match[1]
        return match[0]
    text = re.sub(r"#\[repr\(C\)\](\npub (?:struct|union) (\w+) \{)", with_packing, text)

    # The bitfield derive stores values in byte arrays. A zero-length field
    # restores their alignment without consuming space in the C record.
    def align_record(match):
        prefix, name = match.groups()
        alignment = aligns.get(name)
        if alignment is None or prefix == "-> ":
            return match[0]
        if prefix == "struct ":
            return match[0] + f"    pub _c_alignment: [u{alignment * 8}; 0],\n"
        return match[0] + "    _c_alignment: [],\n"
    text = re.sub(r"(struct |-> )?\b(\w+) \{\n", align_record, text)
    # The native renderer requests a 16-byte-aligned array in gMain.
    text = text.replace("    pub oamBuffer: [OamData; 128],",
                        "    pub _oam_alignment: [u128; 0],\n    pub oamBuffer: [OamData; 128],")
    text = re.sub(r"^( +)oamBuffer: ", r"\1_oam_alignment: [],\n\1oamBuffer: ", text, flags=re.M)
    if checks:
        text += '\n#[cfg(target_pointer_width = "64")]\nconst _: () = {\n'
        text += "    use core::mem::{align_of, offset_of, size_of};\n"
        text += "".join("    " + check + "\n" for check in checks) + "};\n"
    return text
