#!/usr/bin/env python3
"""Generate original PCM cues without any input assets."""
import math
from pathlib import Path
import struct
import wave

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


if __name__ == '__main__':
    for name, notes in CUES.items():
        cue(ROOT / 'stonks-core/assets/cues' / f'{name}.wav', notes)
