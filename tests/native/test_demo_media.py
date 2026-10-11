"""Independent checks of the clean demo database and precomputed audio analysis."""
import importlib.util
import math
from pathlib import Path
import shutil
import struct
import tempfile
import unittest
import wave

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("demo", ROOT / "scripts/build_demo_media.py")
demo = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(demo)


def sections(path):
    data = path.read_bytes()
    assert data[:4] == b"PMAI" and struct.unpack_from(">I", data, 8)[0] == len(data)
    offset = struct.unpack_from(">I", data, 4)[0]
    result = {}
    while offset < len(data):
        name, head, total = struct.unpack_from(">4sII", data, offset)
        assert 12 <= head <= total <= len(data) - offset
        result[name] = (data[offset:offset + head], data[offset + head:offset + total])
        offset += total
    return result


class DemoMediaTests(unittest.TestCase):
    def test_fresh_database_resolves_only_the_requested_rows(self):
        track = {"title": "CC0 Demo", "audio": "/Contents/CC0/01.wav",
                 "analysis": "/PIONEER/USBANLZ/P000/00000001/ANLZ0000.DAT", "bpm": 140}
        data = demo.database({0: [demo.track_row(track, i, 123456, 123.4) for i in (1, 2, 3)]})
        self.assertEqual(struct.unpack_from("<II", data, 4), (4096, 20))
        kind, unused, first, last = struct.unpack_from("<4I", data, 28)
        self.assertEqual((kind, unused), (0, 41))
        self.assertEqual(data[first * 4096 + 27] & 0x40, 0x40)
        self.assertEqual(struct.unpack_from("<I", data, first * 4096 + 12)[0], last)
        base = last * 4096
        self.assertEqual(int.from_bytes(data[base + 24:base + 27], "little") >> 13, 3)
        self.assertEqual(struct.unpack_from("<H", data, base + 4092)[0], 7)
        for row in range(3):
            offset = struct.unpack_from("<H", data, base + 4090 - row * 2)[0]
            start = base + 40 + offset
            self.assertEqual(struct.unpack_from("<I", data, start + 72)[0], row + 1)
            self.assertEqual(struct.unpack_from("<H", data, start + 90)[0], 11)
            self.assertNotEqual(struct.unpack_from("<I", data, start + 4)[0], 0)
            for index, expected in [(14, track["analysis"]), (17, track["title"]), (20, track["audio"])]:
                string = start + struct.unpack_from("<H", data, start + 94 + index * 2)[0]
                length = (data[string] >> 1) - 1
                self.assertEqual(data[string + 1:string + 1 + length].decode(), expected)
        self.assertEqual(data[41 * 4096:], bytes(20 * 4096))
        # An empty table ends at its index page, not an empty data page.
        _, candidate, first, last = struct.unpack_from("<4I", data, 44)
        self.assertEqual(first, last)
        self.assertEqual(struct.unpack_from("<I", data, first * 4096 + 12)[0], candidate)

    def test_browser_categories_resolve_public_names_and_order(self):
        tables = demo.browse_tables()
        names = {}
        for row in tables[16]:
            ident, code = struct.unpack_from("<HH", row)
            self.assertIn(code, (0x81, 0x83, 0x84))
            self.assertEqual(row[4], 0x90)
            size = struct.unpack_from("<H", row, 5)[0]
            names[ident] = row[8:4 + size].decode("utf-16-le")
        categories = [struct.unpack("<4H", row) for row in tables[17]]
        self.assertEqual([names[c[0]] for c in sorted(categories, key=lambda c: c[3])],
                         ["\ufffaARTIST\ufffb", "\ufffaTRACK\ufffb", "\ufffaPLAYLIST\ufffb"])
        self.assertTrue(all(c[2] & 0x100 == 0 for c in categories))

    def test_analysis_lookup_matches_stock_player_path(self):
        self.assertEqual(demo.analysis_path("/Contents/CC0/01.wav"),
                         "/PIONEER/USBANLZ/P046/0001C174/ANLZ0000.DAT")

    def test_grid_phase_tracks_known_transients_and_has_no_end_overrun(self):
        low = [0.] * 1500
        for i in range(15, len(low), 75):
            low[i] = 1.
        times, phase = demo.beat_grid(low, 120, 10.)
        self.assertAlmostEqual(phase, .1)
        self.assertEqual(times[:3], [100, 600, 1100])
        self.assertLess(times[-1], 10000)

    @unittest.skipUnless(shutil.which("ffmpeg"), "Requires offline ffmpeg analysis")
    def test_real_measurements_have_matching_waveform_rates_and_paths(self):
        with tempfile.TemporaryDirectory() as directory:
            usb = Path(directory) / "USB"
            audio = usb / "Contents/CC0/01.wav"
            audio.parent.mkdir(parents=True)
            path = usb / "PIONEER/USBANLZ/P000/00000001/ANLZ0000.DAT"
            path.parent.mkdir(parents=True)
            with wave.open(str(audio), "wb") as output:
                output.setparams((2, 2, 44100, 0, "NONE", "not compressed"))
                for i in range(88200):
                    hz = 100 if i < 44100 else 5000
                    sample = round(12000 * math.sin(2 * math.pi * hz * i / 44100))
                    output.writeframesraw(struct.pack("<hh", sample, sample))
            report = demo.write_analysis(path, audio, 120, 2.)
            self.assertEqual(report["samples"], 300)
            dat = sections(path)
            self.assertEqual(dat[b"PPTH"][1].decode("utf-16-be").rstrip("\0"), "/Contents/CC0/01.wav")
            self.assertEqual(len(dat[b"PWAV"][1]), 400)
            self.assertEqual(len(dat[b"PVBR"][1]), 1604)
            self.assertEqual(struct.unpack_from(">I", dat[b"PQTZ"][0], 16)[0], 0x80000)
            self.assertEqual(struct.unpack_from(">I", dat[b"PWAV"][0], 16)[0], 0x10000)
            ext = sections(path.with_suffix(".EXT"))
            bands = sections(path.with_suffix(".2EX"))
            for name, width in [(b"PWV3", 1), (b"PWV5", 2)]:
                self.assertEqual(struct.unpack_from(">II", ext[name][0], 12), (width, 300))
                self.assertEqual(len(ext[name][1]), width * 300)
            self.assertEqual(len(bands[b"PWV6"][1]), 3600)
            self.assertEqual(len(bands[b"PWV7"][1]), 900)
            values = bands[b"PWV7"][1]
            self.assertGreater(sum(values[2:450:3]), sum(values[1:450:3]))
            self.assertGreater(sum(values[451:900:3]), sum(values[452:900:3]))


if __name__ == "__main__":
    unittest.main()
