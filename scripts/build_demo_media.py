"""Build a local three-track CC0 USB with precomputed analysis, not a player host.

Requires ffmpeg and the Rust backend. No personal library or firmware is read.
File layouts: https://djl-analysis.deepsymmetry.org/rekordbox-export-analysis/
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import struct
import subprocess
import urllib.request
import wave


RATE = 150
PAGE = 4096


def sql_string(value):
    if not value.isascii():
        payload = value.encode("utf-16-le")
        return b"\x90" + struct.pack("<H", len(payload) + 4) + b"\0" + payload
    payload = value.encode("ascii")
    if len(payload) <= 126:
        return bytes([(len(payload) + 1) * 2 + 1]) + payload
    return b"\x40" + struct.pack("<H", len(payload) + 4) + b"\0" + payload


def track_row(track, number, size, seconds):
    row = bytearray(136)
    struct.pack_into("<HH", row, 0, 0x24, (number - 1) * 32)
    for offset, value in {4: 0xc0700, 8: 44100, 16: size, 20: number + 20,
                          48: 1411, 52: number, 56: track["bpm"] * 100,
                          60: 1, 64: 1, 68: 1, 72: number}.items():
        struct.pack_into("<I", row, offset, value)
    struct.pack_into("<HH", row, 24, 19048, 30967)
    struct.pack_into("<HHHHHH", row, 76, 1, 0, 2026, 16, math.ceil(seconds), 41)
    struct.pack_into("<HH", row, 90, 11, 3)  # DeviceSQL file type 11 = WAV.
    strings = {10: "2026-10-10", 14: track["analysis"], 15: "2026-10-10",
               16: "CC0 demo; approximate fixture analysis", 17: track["title"],
               19: Path(track["audio"]).name, 20: track["audio"]}
    for i in range(21):
        struct.pack_into("<H", row, 94 + i * 2, len(row))
        row.extend(sql_string(strings.get(i, "")))
    return bytes(row)


def database(tables):
    """Fresh DeviceSQL database: no copied template, deleted rows or private data."""
    result = bytearray(PAGE * 61)
    struct.pack_into("<7I", result, 0, 0, PAGE, 20, 61, 1, 41, 0)
    for kind in range(20):
        index, data = 1 + kind * 2, 2 + kind * 2
        rows = tables.get(kind, [])
        candidate = 41 + kind
        struct.pack_into("<4I", result, 28 + kind * 16,
                         kind, candidate, index, data if rows else index)
        base = index * PAGE
        struct.pack_into("<6I", result, base, 0, index, kind, data if rows else candidate, 1, 0)
        result[base + 27] = 0x64
        struct.pack_into("<4H", result, base + 32, 0x1fff, 0x1fff, 0x3ec, 0)
        struct.pack_into("<5I", result, base + 40,
                         index, data if rows else 0x3ffffff, 0x3ffffff, 0, 0x1fff0000)
        for offset in range(60, PAGE - 20, 4):
            struct.pack_into("<I", result, base + offset, 0x1ffffff8)
        if not rows:
            continue
        assert len(rows) <= 16
        base = data * PAGE
        struct.pack_into("<6I", result, base, 0, data, kind, candidate, data, 0)
        count = len(rows) | len(rows) << 13
        result[base + 24:base + 27] = count.to_bytes(3, "little")
        result[base + 27] = 0x24
        heap = 0
        for i, row in enumerate(rows):
            assert heap + len(row) < PAGE - 40 - 36
            result[base + 40 + heap:base + 40 + heap + len(row)] = row
            struct.pack_into("<H", result, base + PAGE - 6 - i * 2, heap)
            heap += (len(row) + 3) & ~3
        directory_size = len(rows) * 2 + 4
        written, previous = (len(rows), 0) if kind in (6, 16) else (1, len(rows) - 1)
        struct.pack_into("<4H", result, base + 28,
                         PAGE - 40 - directory_size - heap, heap, written, previous)
        mask = (1 << len(rows)) - 1
        struct.pack_into("<HH", result, base + PAGE - 4, mask, mask)
    return bytes(result)


def browse_tables():
    """Public DeviceSQL category IDs/codes; only three useful demo categories.

    Layout references: rekordcrate MenuRow / ColumnRow and baken-export pdb/fixed.
    These are generated configuration records, not rows from a personal export.
    """
    columns = [(2, 0x81, "ARTIST"), (4, 0x83, "TRACK"), (17, 0x84, "PLAYLIST")]
    return {
        16: [struct.pack("<HH", ident, code) + sql_string("\ufffa" + name + "\ufffb")
             for ident, code, name in columns],
        17: [struct.pack("<4H", 2, 2, 2, 0), struct.pack("<4H", 4, 4, 1, 1),
             struct.pack("<4H", 17, 5, 99, 2)],
        18: [struct.pack("<4H", 2, 2, 0x300, 0), struct.pack("<4H", 4, 4, 0x400, 0)],
    }


def analysis_path(audio_path):
    """Player-compatible path hash, verified against stock 1.45's CC0 overlay."""
    value = 0
    for char in audio_path:
        value = ((value * 0x5bc9 + ord(char)) * 0x93b5 + ord(char)) & 0xffffffff
    value %= 200003
    folder = sum(((value >> bit) & 1) << i for i, bit in enumerate((0, 2, 6, 7, 9, 13, 16)))
    return f"/PIONEER/USBANLZ/P{folder:03X}/{value:08X}/ANLZ0000.DAT"


def tag(name, extra, payload=b""):
    head = 12 + len(extra)
    return name.encode() + struct.pack(">II", head, head + len(payload)) + extra + payload


def analysis(tags):
    content = b"".join(tags)
    return b"PMAI" + struct.pack(">II4I", 28, 28 + len(content), 1, 0, 0, 0) + content


def peaks(path, filters="anull"):
    chain = (filters + ",asetnsamples=n=294:p=1,astats=metadata=1:reset=1,"
             "ametadata=mode=print:key=lavfi.astats.Overall.Peak_level:file=-")
    output = subprocess.run(
        ["ffmpeg", "-nostdin", "-hide_banner", "-loglevel", "error", "-i", str(path),
         "-ac", "1", "-ar", "44100", "-af", chain, "-f", "null", "-"],
        check=True, capture_output=True, text=True).stdout
    values = [float(v) for v in re.findall(r"Peak_level=([^\s]+)", output)]
    if not values:
        raise ValueError("ffmpeg did not produce waveform measurements")
    return [10 ** (v / 20) if math.isfinite(v) else 0 for v in values]


def reduce(values, width):
    return [max(values[i * len(values) // width:
                       max(i * len(values) // width + 1, (i + 1) * len(values) // width)], default=0)
            for i in range(width)]


def beat_grid(low, bpm, seconds):
    period = 60 / bpm
    # Known creator BPM; estimate phase from positive low-frequency transients.
    flux = [0] + [max(0, low[i] - low[i - 1]) for i in range(1, len(low))]
    phases = range(max(1, round(period * RATE)))
    def score(phase):
        return sum(flux[min(len(flux) - 1, round((beat * period + phase / RATE) * RATE))]
                   for beat in range(4, min(100, int(seconds / period) - 1)))
    phase = max(phases, key=score) / RATE
    times = [round((phase + i * period) * 1000)
             for i in range(math.ceil((seconds - phase) / period))]
    return times, phase


def write_analysis(path, audio, bpm, seconds):
    full = peaks(audio)
    low = peaks(audio, "lowpass=f=200")
    mid = peaks(audio, "highpass=f=200,lowpass=f=2000")
    high = peaks(audio, "highpass=f=2000")
    count = math.ceil(seconds * RATE)
    if not all(len(v) == count for v in (full, low, mid, high)):
        raise ValueError("Waveform length disagrees with decoded audio")
    maximum = max(max(v) for v in (full, low, mid, high)) or 1
    def height(value, limit=31):
        return min(limit, round(limit * math.sqrt(value / maximum)))
    blue = bytes(height(v) for v in full)
    rgb = bytearray()
    bands = bytearray()
    for lo, mi, hi, amp in zip(low, mid, high, full):
        r, g, b = (height(v, 7) for v in (hi, mi, lo))
        rgb.extend(struct.pack(">H", (r << 13) | (g << 10) | (b << 7) | (height(amp) << 2)))
        bands.extend(height(v, 127) for v in (mi, hi, lo))
    preview = bytearray()
    overview_bands = bytearray()
    for lo, mi, hi in zip(reduce(low, 1200), reduce(mid, 1200), reduce(high, 1200)):
        levels = [height(v) for v in (lo, mi, hi)]
        preview.extend([max(levels), 0, 0, *levels])
        overview_bands.extend(height(v, 127) for v in (mi, hi, lo))
    times, phase = beat_grid(low, bpm, seconds)
    grid = b"".join(struct.pack(">HHI", i % 4 + 1, bpm * 100, t) for i, t in enumerate(times))
    pth = audio.as_posix().split("/USB/", 1)[1]
    pth = ("/" + pth + "\0").encode("utf-16-be")
    shared = [tag("PPTH", struct.pack(">I", len(pth)), pth),
              tag("PQTZ", struct.pack(">III", 0, 0x80000, len(times)), grid)]
    cues = [tag("PCOB", struct.pack(">IHHI", kind, 0, 0, 0xffffffff)) for kind in (1, 0)]
    path.with_suffix(".DAT").write_bytes(analysis([
        shared[0], tag("PVBR", struct.pack(">I", 0), bytes(1604)), shared[1],
        tag("PWAV", struct.pack(">II", 400, 0x10000), bytes(height(v) for v in reduce(full, 400))),
        tag("PWV2", struct.pack(">II", 100, 0x10000), bytes(height(v, 15) for v in reduce(full, 100)))] + cues))
    path.with_suffix(".EXT").write_bytes(analysis(shared + cues + [
        tag("PWV3", struct.pack(">III", 1, count, 0x960000), blue),
        tag("PWV4", struct.pack(">III", 6, 1200, 0), preview),
        tag("PWV5", struct.pack(">III", 2, count, 0x960305), rgb)]))
    path.with_suffix(".2EX").write_bytes(analysis(shared + [
        tag("PWV6", struct.pack(">II", 3, 1200), overview_bands),
        tag("PWV7", struct.pack(">III", 3, count, 0x960000), bands)]))
    return {"samples": count, "beats": len(times), "beat_phase_seconds": phase}


def build(out, backend, manifest):
    out.mkdir(parents=True, exist_ok=False)
    usb = out / "USB"
    source = out / "source"
    source.mkdir()
    (usb / "Contents/CC0").mkdir(parents=True)
    (usb / "PIONEER/rekordbox").mkdir(parents=True)
    rows, report = [], []
    for number, original in enumerate(manifest, 1):
        track = original.copy()
        if track["license"] != "CC0-1.0":
            raise ValueError("Demo manifest must contain only CC0 tracks")
        downloaded = source / track["file"]
        with urllib.request.urlopen(track["download"], timeout=60) as response:
            data = response.read(32 * 1024 * 1024 + 1)
        if len(data) > 32 * 1024 * 1024 or hashlib.sha256(data).hexdigest() != track["sha256"]:
            raise ValueError("Licensed source changed or exceeds download limit")
        downloaded.write_bytes(data)
        track["audio"] = f"/Contents/CC0/{number:02}.wav"
        track["analysis"] = analysis_path(track["audio"])
        audio = usb / track["audio"].lstrip("/")
        subprocess.run(["ffmpeg", "-nostdin", "-v", "error", "-i", str(downloaded),
                        "-map_metadata", "-1", "-ac", "2", "-ar", "44100",
                        "-c:a", "pcm_s16le", "-fflags", "+bitexact", str(audio)], check=True)
        with wave.open(str(audio)) as decoded:
            seconds = decoded.getnframes() / decoded.getframerate()
        path = usb / track["analysis"].lstrip("/")
        path.parent.mkdir(parents=True)
        measurements = write_analysis(path, audio, track["bpm"], seconds)
        rows.append(track_row(track, number, audio.stat().st_size, seconds))
        report.append({**track, **measurements, "seconds": seconds})
        print(f"Analyzed {track['title']}: {seconds:.1f}s, {measurements['samples']} samples", flush=True)
    tables = {0: rows, 1: [struct.pack("<I", 1) + sql_string("Electronic")],
              2: [struct.pack("<HHIBB", 0x60, 0, 1, 3, 10) + sql_string("MintoDog")],
              3: [struct.pack("<HHIIIIBB", 0x80, 0, 0, 1, 1, 0, 3, 22) + sql_string("CC0 Demo")],
              7: [struct.pack("<5I", 0, 0, 0, 1, 0) + sql_string("CC0 Demo")],
              8: [struct.pack("<3I", i, i, 1) for i in range(1, len(rows) + 1)]}
    tables.update(browse_tables())
    (usb / "PIONEER/rekordbox/export.pdb").write_bytes(database(tables))
    subprocess.run([str(backend), "export", "--out", str(out / "cc0-demo.img"), str(usb)], check=True)
    (out / "analysis.json").write_text(json.dumps(report, indent=2) + "\n")
    return out / "cc0-demo.img"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--backend", type=Path, required=True)
    parser.add_argument("--manifest", type=Path, default=Path(__file__).with_name("demo-tracks.json"))
    args = parser.parse_args()
    print(build(args.out.resolve(), args.backend.resolve(), json.loads(args.manifest.read_text())))
