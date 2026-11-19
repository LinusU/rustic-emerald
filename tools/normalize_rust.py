"""Normalize the native ABI constructs which C2Rust cannot express directly."""
import re
from collections import Counter


# A C2Rust assignment may contain array types (`[T; N]`) and nested blocks.
# Only a semicolon outside all delimiters ends its Rust statement.
TOKENS = re.compile(r'''"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])'|//[^\n]*|/\*.*?\*/|[][(){};]''', re.S)


def statement_end(text, start):
    depth = 0
    for token in TOKENS.finditer(text, start):
        char = token[0]
        if char in ("[", "(", "{"):
            depth += 1
        elif char in ("]", ")", "}"):
            depth -= 1
            if depth < 0:
                break
        elif char == ";" and depth == 0:
            return token.start()
    return None


def packed_temporaries(function):
    """Preserve C evaluation order without borrowing a misaligned field."""
    if "let ref mut fresh" not in function:
        return function
    counts = Counter(re.findall(r"\bfresh\d+\b", function))
    parts = []
    last = 0
    for decl in re.finditer(r"let ref mut (fresh\d+) = ", function):
        name = decl[1]
        if counts[name] != 2 or decl.start() < last:
            continue
        end = statement_end(function, decl.end())
        if end is None:
            continue
        assignment = re.compile(r"(\s*)\*" + name + r" = ").match(function, end + 1)
        if assignment is None:
            continue
        rhs_end = statement_end(function, assignment.end())
        if rhs_end is None:
            continue
        place = function[decl.end():end]
        value = function[assignment.end():rhs_end]
        parts.extend((function[last:decl.start()], f"let {name} = &raw mut {place};",
                      f"{assignment[1]}{name}.write_unaligned({value});"))
        last = rhs_end + 1
    parts.append(function[last:])
    function = "".join(parts)
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


def align_dodrio_fields(text, source):
    """Preserve the Dodrio minigame's per-field alignment attributes."""
    records = re.finditer(r"^struct (DodrioGame\w*|\w+Packet)\s*\{([^}]*)\};", source, re.M)
    for record in records:
        fields = set(re.findall(r"__attribute__\(\(aligned\(4\)\)\)\s+(\w+)", record[2]))
        if not fields:
            continue
        def add_witnesses(body):
            def align(field):
                return (f"    pub _{field[1]}_alignment: [u32; 0],\n" if field[1] in fields else "") + field[0]
            return re.sub(r"^    pub (\w+):", align, body[0], flags=re.M)
        text = re.sub(rf"^pub struct {record[1]} \{{[^}}]*\}}", add_witnesses, text, flags=re.M)
        # Local RFU packet variables receive an explicit zero initializer.
        def init_witnesses(body):
            return re.sub(r"^([ ]+)(\w+):", lambda field:
                          (f"{field[1]}_{field[2]}_alignment: [],\n" if field[2] in fields else "")
                          + field[0], body[0], flags=re.M)
        text = re.sub(rf"(?<!struct )(?<!-> )\b{record[1]} \{{[^{{}}]*\}}", init_witnesses, text)
    return text


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
        if alignment is None or (prefix != "struct " and not re.compile(r"\s*\w+:(?!:)").match(text, match.end())):
            return match[0]
        if prefix == "struct ":
            return match[0] + f"    pub _c_alignment: [u{alignment * 8}; 0],\n"
        return match[0] + "    _c_alignment: [],\n"
    text = re.sub(r"(struct |-> )?\b(\w+) \{(?:\n|(?= [a-zA-Z_]\w*:))", align_record, text)
    # RFU names begin at a four-byte-aligned offset following a 14-byte
    # radio packet. Model the original aligned field without changing its type.
    text = text.replace("pub struct RfuPlayerData {\n    pub data: RfuGameData,",
                        "pub struct RfuPlayerData {\n    pub data: RfuGameData,\n"
                        "    pub _name_alignment: [u32; 0],")
    text = re.sub(r"(?<!struct )(?<!-> )\bRfuPlayerData \{(?=\s*data:)",
                  "RfuPlayerData { _name_alignment: [],", text)
    # The native renderer requests a 16-byte-aligned array in gMain.
    text = text.replace("    pub oamBuffer: [OamData; 128],",
                        "    pub _oam_alignment: [u128; 0],\n    pub oamBuffer: [OamData; 128],")
    text = re.sub(r"^( +)oamBuffer: (?=\[)", r"\1_oam_alignment: [],\n\1oamBuffer: ", text, flags=re.M)
    if checks:
        text += '\n#[cfg(target_pointer_width = "64")]\nconst _: () = {\n'
        text += "    use core::mem::{align_of, offset_of, size_of};\n"
        text += "".join("    " + check + "\n" for check in checks) + "};\n"
    return text
