#!/usr/bin/env python3
"""Generate original launcher art and PCM cues without any input assets."""
import math
from pathlib import Path
import struct
import wave
import zlib

ROOT = Path(__file__).resolve().parents[1] / 'crates'
CUES = {
    'microphone-mute': (620, 455),
    'microphone-unmute': (455, 620),
    'headphones-deafen': (530, 400, 300),
    'headphones-undeafen': (300, 400, 530),
    'voice-connect': (385, 510, 690),
    'voice-disconnect': (690, 510, 385),
    'message-notification': (740,),
    'screen-share-start': (420, 565, 565),
    'screen-share-stop': (565, 565, 420),
    'screen-viewer-join': (555, 720),
    'screen-viewer-leave': (720, 555),
}


def cue(path, notes):
    rate = 24000
    length = 1800
    path.parent.mkdir(parents=True, exist_ok=True)
    with wave.open(str(path), 'wb') as output:
        output.setparams((1, 2, rate, 0, 'NONE', 'not compressed'))
        samples = []
        for hz in notes:
            for i in range(length):
                phase = math.tau * hz * i / rate
                envelope = min(i / 120, (length - 1 - i) / 480, 1)
                samples.append(round(5500 * envelope * (math.sin(phase) + 0.16 * math.sin(2 * phase))))
            samples.extend([0] * 240)
        output.writeframes(struct.pack(f'<{len(samples)}h', *samples))


def icon(path, size):
    # A mint speech bubble containing three rising bars on a navy tile.
    def pixel(x, y):
        if not (0.06 <= x < 0.94 and 0.06 <= y < 0.94):
            return (0, 0, 0, 0)
        bubble = 0.19 <= x < 0.81 and 0.23 <= y < 0.69
        tail = 0.30 <= x < 0.46 and 0.69 <= y < 0.69 + (0.46 - x)
        bars = any(a <= x < a + 0.08 and top <= y < 0.60
                   for a, top in ((0.31, 0.50), (0.46, 0.42), (0.61, 0.34)))
        return (124, 240, 190, 255) if (bubble or tail) and not bars else (25, 35, 59, 255)

    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))

    pixels = b''.join(b'\0' + bytes(c for x in range(size)
                                       for c in pixel((x + 0.5) / size, (y + 0.5) / size))
                      for y in range(size))
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', size, size, 8, 6, 0, 0, 0))
                     + chunk(b'IDAT', zlib.compress(pixels, 9)) + chunk(b'IEND', b''))


if __name__ == '__main__':
    for name, notes in CUES.items():
        cue(ROOT / 'stonks-core/assets/cues' / f'{name}.wav', notes)
    icon(ROOT / 'stonks-app/launcher/com.rdxlabs.stonks.gpui.png', 128)
    icon(ROOT / 'stonks-app/assets/branding/32x32.png', 32)
