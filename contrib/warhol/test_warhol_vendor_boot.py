import hashlib
import struct
import unittest
from pathlib import Path

from warhol_vendor_boot import VendorBoot, Entry, align, avb_footer, build, read_cpio, write_cpio


def sample():
    h = bytearray(4096)
    h[:8] = b"VNDRBOOT"
    for at, value in [(8, 4), (12, 4096), (24, 12), (2096, 2128), (2100, 4),
                      (2112, 216), (2116, 2), (2120, 108), (2124, 7)]:
        struct.pack_into("<I", h, at, value)
    h[28:34] = b"keepme"
    table = bytearray()
    for size, offset, kind, name in [(5, 0, 1, b"platform"), (7, 5, 2, b"recovery")]:
        table.extend(struct.pack("<III32s16I", size, offset, kind, name, *range(16)))
    out = bytes(h)
    for section in (b"BASE!RECOVRY", b"DTB!", table, b"cfg=yes"):
        out += bytes(section)
        out += bytes(align(len(out)) - len(out))
    return out


class VendorBootTests(unittest.TestCase):
    def test_repack_preserves_platform_and_selection_metadata(self):
        before = VendorBoot(sample())
        after = VendorBoot(before.replace_recovery(b"R" * 6000))
        self.assertEqual(after.fragments, [b"BASE!", b"R" * 6000])
        self.assertEqual(before.dtb, after.dtb)
        self.assertEqual(before.bootconfig, after.bootconfig)
        self.assertEqual(before.header[:24], after.header[:24])
        self.assertEqual(before.header[28:], after.header[28:])
        for a, b in zip(before.entries, after.entries):
            self.assertEqual(a[8:], b[8:])

    def test_bad_layouts_are_rejected(self):
        for at, value in [(8, 3), (12, 2048), (2112, 1), (2116, 0), (2116, 1000), (2120, 107), (24, 0xffffffff)]:
            b = bytearray(sample())
            struct.pack_into("<I", b, at, value)
            with self.subTest(offset=at), self.assertRaises(ValueError):
                VendorBoot(b)
        b = bytearray(sample())
        # The second fragment overlaps the first.
        struct.pack_into("<I", b, 12288 + 108 + 4, 0)
        with self.assertRaises(ValueError):
            VendorBoot(b)
        with self.assertRaises(ValueError):
            VendorBoot(sample()[:13000])

    def test_other_phone_and_unpatched_binary_refused(self):
        with self.assertRaisesRegex(ValueError, "verified warhol"):
            build(sample(), b"", b"", {})

    def test_avb_matches_upstream_avbtool_fixtures(self):
        root = Path(__file__).resolve().parents[2]
        for size in [1000, 4096]:
            fixture = root / f"crates/skyward-pack/tests/avb-footer-{size}.bin"
            self.assertEqual(avb_footer(b"S" * size, 81920, "recovery", b"\x5a" * 32), fixture.read_bytes())

    def test_footer_and_capacity(self):
        data = b"firmware" * 900
        image = avb_footer(data, 16384)
        magic, major, minor, size, offset, vsize, _ = struct.unpack(">4sIIQQQ28s", image[-64:])
        self.assertEqual((magic, major, minor, size), (b"AVBf", 1, 0, len(data)))
        self.assertEqual(image[offset:offset + 4], b"AVB0")
        self.assertGreater(vsize, 256)
        self.assertEqual(image[:size], data)
        with self.assertRaises(ValueError):
            avb_footer(data, 8192)

    def test_cpio_metadata_and_links_roundtrip(self):
        files = [Entry("a", [42, 0o100640, 1000, 1001, 2, 123, 0, 1, 2, 3, 4, 0, 0], b"data"),
                 Entry("link", [43, 0o120777, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0], b"a")]
        got = read_cpio(write_cpio(files))
        for a, b in zip(files, got):
            self.assertEqual((a.name, a.data), (b.name, b.data))
            self.assertEqual(a.fields[:6], b.fields[:6])
            self.assertEqual(a.fields[7:11], b.fields[7:11])
        bad = Entry("../bad", [1, 0o100644, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0], b"bad")
        with self.assertRaises(ValueError):
            read_cpio(write_cpio([bad]))
        with self.assertRaises(ValueError):
            read_cpio(write_cpio(files) + b"unaccounted data")


if __name__ == "__main__":
    unittest.main()
