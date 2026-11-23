from io import BytesIO
from pathlib import Path
import tempfile
import unittest

from import_string_journal import MAX_BULK, MAX_INPUT, import_file, write_frame


def frames(*commands):
    result = BytesIO()
    for command in commands:
        write_frame(result, list(command))
    return result.getvalue()


class ImportStringJournalTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.source = self.root / "source.resp"
        self.destination = self.root / "appendonly.aof"

    def test_binary_values_and_absolute_expiry(self):
        data = frames((b"SELECT", b"0"), (b"set", b"key\0", b"\xff\r\n", b"pxat", b"4102444800000"),
                      (b"SET", b"persistent", b""))
        self.source.write_bytes(data)
        self.assertEqual(import_file(self.source, self.destination), 2)
        self.assertEqual(self.destination.read_bytes(), frames(
            (b"SET", b"key\0", b"\xff\r\n"), (b"PEXPIREAT", b"key\0", b"4102444800000"),
            (b"SET", b"persistent", b"")))
        self.assertEqual(self.source.read_bytes(), data)
        self.assertEqual(list(self.root.glob(".rillcache-import-*")), [])

    def test_existing_output_is_preserved(self):
        self.source.write_bytes(frames((b"SET", b"k", b"v")))
        self.destination.write_bytes(b"previous journal")
        with self.assertRaises(FileExistsError):
            import_file(self.source, self.destination)
        self.assertEqual(self.destination.read_bytes(), b"previous journal")
        self.assertEqual(list(self.root.glob(".rillcache-import-*")), [])

    def test_invalid_suffix_never_publishes_a_valid_prefix(self):
        prefix = frames((b"SET", b"k", b"v"))
        for suffix in (frames((b"SELECT", b"1")), frames((b"SELECT", b"0")),
                       frames((b"SET", b"k", b"v", b"PX", b"100")),
                       frames((b"SET", b"k", b"v", b"PXAT", b"0")),
                       frames((b"SET", b"k", b"v", b"PXAT", b"9223372036854775808")),
                       frames((b"MULTI",)), b"REDIS0009", b"*3\r\n$3\r\nSET\r\n$2\r\nk",
                       b"*3\r\n$%d\r\n" % (MAX_BULK + 1),
                       frames((b"SET", b"k" * MAX_BULK, b"v" * MAX_BULK))):
            with self.subTest(prefix=suffix[:24]):
                self.source.write_bytes(prefix + suffix)
                with self.assertRaises(ValueError):
                    import_file(self.source, self.destination)
                self.assertFalse(self.destination.exists())
                self.assertEqual(list(self.root.glob(".rillcache-import-*")), [])
                self.assertEqual(self.source.read_bytes(), prefix + suffix)

    def test_oversize_source_and_empty_dataset_are_refused(self):
        with self.source.open("wb") as source:
            source.truncate(MAX_INPUT + 1)
        with self.assertRaisesRegex(ValueError, "exceeds 16 MiB"):
            import_file(self.source, self.destination)
        self.source.write_bytes(frames((b"SELECT", b"0")))
        with self.assertRaisesRegex(ValueError, "at least one SET"):
            import_file(self.source, self.destination)
        self.assertFalse(self.destination.exists())


if __name__ == "__main__":
    unittest.main()
