"""Align a Mach-O string pool to 8 bytes.

Xcode 27's ld (ld-27037.1) emits dylibs whose string pool starts at an offset that is 4 mod 8 when
the indirect symbol count is odd, then rejects its own output with "mis-aligned LINKEDIT string
pool". The pool is followed by the code signature, 16-aligned, so shifting it right by the padding
needs no other offset to move. The caller re-signs afterwards.

    python3 -m tools.plan.macho_align <dylib>
"""

import os
import struct
import sys
import tempfile
from pathlib import Path

MH_MAGIC_64 = 0xFEEDFACF
LC_SYMTAB = 0x2
LC_CODE_SIGNATURE = 0x1D
LINKEDIT_DATA_COMMANDS = {0x1D, 0x1E, 0x26, 0x29, 0x2B, 0x2C, 0x33, 0x34}
COMMAND_HEADER_SIZE = 8
HEADER_SIZE = 32
STRING_POOL_ALIGNMENT = 8
SYMTAB_STROFF_FIELD = 16
CODE_SIGNATURE_DATAOFF_FIELD = 8


class AlignError(Exception):
    pass


def align_string_pool(data):
    if len(data) < HEADER_SIZE or struct.unpack_from("<I", data, 0)[0] != MH_MAGIC_64:
        raise AlignError("not a 64-bit little-endian Mach-O")

    try:
        return _align(data)
    except struct.error as error:
        raise AlignError(f"truncated Mach-O: {error}") from error


def _align(data):
    ncmds = struct.unpack_from("<I", data, 16)[0]
    symtab = signature = None
    data_offsets = []
    offset = HEADER_SIZE
    for _ in range(ncmds):
        command, size = struct.unpack_from("<II", data, offset)
        if size < COMMAND_HEADER_SIZE or offset + size > len(data):
            raise AlignError("malformed load command")
        if command == LC_SYMTAB:
            symtab = offset
        elif command == LC_CODE_SIGNATURE:
            signature = offset
        if command in LINKEDIT_DATA_COMMANDS:
            data_offsets.append(
                struct.unpack_from("<I", data, offset + CODE_SIGNATURE_DATAOFF_FIELD)[0]
            )
        offset += size
    if symtab is None:
        raise AlignError("no LC_SYMTAB")

    stroff, strsize = struct.unpack_from("<II", data, symtab + SYMTAB_STROFF_FIELD)
    shift = -stroff % STRING_POOL_ALIGNMENT
    if shift == 0:
        return data, 0

    end = stroff + strsize
    limit = (
        struct.unpack_from("<I", data, signature + CODE_SIGNATURE_DATAOFF_FIELD)[0]
        if signature is not None
        else len(data)
    )
    if end + shift > limit:
        raise AlignError("no room after the string pool to shift it")
    if any(end <= start < end + shift for start in data_offsets):
        raise AlignError("another LINKEDIT blob starts in the shift window")
    if any(data[end : end + shift]):
        raise AlignError("bytes after the string pool are not padding")

    fixed = bytearray(data)
    fixed[stroff + shift : end + shift] = data[stroff:end]
    fixed[stroff : stroff + shift] = bytes(shift)
    struct.pack_into("<I", fixed, symtab + SYMTAB_STROFF_FIELD, stroff + shift)
    return bytes(fixed), shift


def write_atomically(path, data):
    descriptor, temporary = tempfile.mkstemp(dir=path.parent)
    try:
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(data)
        os.chmod(temporary, path.stat().st_mode)
        os.replace(temporary, path)
    except BaseException:
        os.unlink(temporary)
        raise


def main(argv):
    if len(argv) != 1:
        print("usage: python3 -m tools.plan.macho_align <dylib>", file=sys.stderr)
        return 2
    path = Path(argv[0])
    try:
        fixed, shift = align_string_pool(path.read_bytes())
    except (AlignError, OSError) as error:
        print(f"macho_align: {path}: {error}", file=sys.stderr)
        return 1
    if shift:
        write_atomically(path, fixed)
        print(f"macho_align: shifted string pool of {path.name} by {shift}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
