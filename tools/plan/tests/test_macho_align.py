import struct
import tempfile
import unittest
from pathlib import Path

from tools.plan import macho_align

MH_MAGIC_64 = 0xFEEDFACF
LC_SYMTAB = 0x2
LC_CODE_SIGNATURE = 0x1D


def build(stroff, strsize, sig_off, with_signature=True, gap=None):
    strings = bytes(range(1, 1 + strsize))
    commands = struct.pack("<IIIIII", LC_SYMTAB, 24, 0, 0, stroff, strsize)
    ncmds = 1
    if with_signature:
        commands += struct.pack("<IIII", LC_CODE_SIGNATURE, 16, sig_off, 8)
        ncmds += 1
    header = struct.pack("<IiiIIIII", MH_MAGIC_64, 0, 0, 6, ncmds, len(commands), 0, 0)
    body = bytearray(sig_off + 8)
    body[: len(header) + len(commands)] = header + commands
    body[stroff : stroff + strsize] = strings
    return bytes(body), strings


def stroff_of(data):
    return struct.unpack_from("<I", data, 32 + 16)[0]


class AlignStringPoolTests(unittest.TestCase):
    def test_aligned_pool_is_untouched(self):
        data, _ = build(stroff=128, strsize=40, sig_off=176)
        self.assertEqual(macho_align.align_string_pool(data), (data, 0))

    def test_misaligned_pool_moves_to_next_eight_byte_boundary(self):
        data, strings = build(stroff=132, strsize=40, sig_off=176)
        fixed, shift = macho_align.align_string_pool(data)
        self.assertEqual(shift, 4)
        self.assertEqual(stroff_of(fixed), 136)
        self.assertEqual(fixed[136:176], strings)
        self.assertEqual(len(fixed), len(data))

    def test_refuses_when_signature_leaves_no_room(self):
        data, _ = build(stroff=132, strsize=40, sig_off=172)
        with self.assertRaises(macho_align.AlignError):
            macho_align.align_string_pool(data)

    def test_refuses_when_padding_is_not_zero(self):
        data, _ = build(stroff=132, strsize=40, sig_off=176)
        corrupted = bytearray(data)
        corrupted[172] = 0xFF
        with self.assertRaises(macho_align.AlignError):
            macho_align.align_string_pool(bytes(corrupted))

    def test_refuses_a_file_that_is_not_a_64_bit_mach_o(self):
        with self.assertRaises(macho_align.AlignError):
            macho_align.align_string_pool(b"\x7fELF" + bytes(64))

    def test_without_a_signature_the_pad_must_fit_in_the_file(self):
        data, _ = build(stroff=132, strsize=40, sig_off=164, with_signature=False)
        with self.assertRaises(macho_align.AlignError):
            macho_align.align_string_pool(data)

    def test_refuses_a_truncated_load_command_table(self):
        data, _ = build(stroff=132, strsize=40, sig_off=176)
        with self.assertRaises(macho_align.AlignError):
            macho_align.align_string_pool(data[:40])

    def test_refuses_a_zero_sized_load_command(self):
        data, _ = build(stroff=132, strsize=40, sig_off=176)
        broken = bytearray(data)
        struct.pack_into("<I", broken, 32 + 4, 0)
        with self.assertRaises(macho_align.AlignError):
            macho_align.align_string_pool(bytes(broken))

    def test_refuses_when_another_blob_starts_in_the_shift_window(self):
        data, _ = build(stroff=132, strsize=40, sig_off=176)
        overlapping = bytearray(data)
        struct.pack_into("<I", overlapping, 32 + 24 + 8, 172)
        with self.assertRaises(macho_align.AlignError):
            macho_align.align_string_pool(bytes(overlapping))

    def test_cli_reports_a_missing_file_without_a_traceback(self):
        self.assertEqual(macho_align.main(["/nonexistent/lib.dylib"]), 1)

    def test_cli_rewrites_the_file_in_place(self):
        data, strings = build(stroff=132, strsize=40, sig_off=176)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "lib.dylib"
            path.write_bytes(data)
            self.assertEqual(macho_align.main([str(path)]), 0)
            fixed = path.read_bytes()
        self.assertEqual(stroff_of(fixed), 136)
        self.assertEqual(fixed[136:176], strings)


if __name__ == "__main__":
    unittest.main()
