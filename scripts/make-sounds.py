#!/usr/bin/env python3
"""Writes the default theme's select, back, error and start sounds as WAV files.

Each sound is one or two synthesized tones: a pitch glide with an 8 ms attack and an
exponential decay to -80 dB, as the old launcher made them. The files are mono, 16 bit,
22050 Hz, as move.wav. Python 3 standard library only.

Usage: scripts/make-sounds.py [folder]   (default: plugins/themes/default/theme/sounds)
"""

import math
import struct
import sys
import wave
from pathlib import Path

RATE = 22050


def tone(out, f1, f2, dur, vol, shape, delay):
    """Adds a tone from f1 to f2 Hz, `dur` seconds long, `delay` seconds in, to `out`."""
    start = int(delay * RATE)
    length = int(dur * RATE)
    if len(out) < start + length:
        out.extend([0.0] * (start + length - len(out)))
    phase = 0.0
    for i in range(length):
        t = i / length
        f = f1 * (f2 / f1) ** t
        phase = (phase + f / RATE) % 1.0
        if shape == "sine":
            s = math.sin(phase * math.tau)
        elif shape == "triangle":
            s = 1.0 - 4.0 * abs(phase - 0.5)
        else:
            s = 0.6 if phase < 0.5 else -0.6
        attack = min(i / (0.008 * RATE), 1.0)
        envelope = attack * 0.0001**t
        out[start + i] += s * envelope * vol


def sounds():
    """Each sound's name and its samples, from -1 to 1."""
    select, back, error, start = [], [], [], []
    tone(select, 680.0, 1020.0, 0.12, 0.09, "sine", 0.0)
    tone(back, 880.0, 560.0, 0.1, 0.07, "sine", 0.0)
    tone(error, 230.0, 170.0, 0.22, 0.06, "square", 0.0)
    tone(start, 392.0, 784.0, 0.45, 0.09, "sine", 0.0)
    tone(start, 587.0, 1175.0, 0.6, 0.06, "sine", 0.12)
    return {"select": select, "back": back, "error": error, "start": start}


def write(path, samples):
    frames = b"".join(struct.pack("<h", int(max(-1.0, min(1.0, s)) * 32767)) for s in samples)
    with wave.open(str(path), "wb") as out:
        out.setnchannels(1)
        out.setsampwidth(2)
        out.setframerate(RATE)
        out.writeframes(frames)


def main():
    here = Path(__file__).resolve().parent.parent
    folder = Path(sys.argv[1]) if len(sys.argv) > 1 else here / "plugins/themes/default/theme/sounds"
    folder.mkdir(parents=True, exist_ok=True)
    for name, samples in sounds().items():
        path = folder / f"{name}.wav"
        write(path, samples)
        print(f"{path}: {path.stat().st_size} bytes")


if __name__ == "__main__":
    main()
