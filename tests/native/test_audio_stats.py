"""Host callback deficits are separate from DSP serializer underruns."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
COMMON = ROOT / 'native/qemu/hw/cdj/common'


class AudioStatsTests(unittest.TestCase):
    def test_callback_bins_count_only_frames_the_host_could_not_receive(self):
        code = r'''
#include <assert.h>
#include <stdlib.h>
#include "xdj_audio_stats.h"
int main(void) {
    XdjAudioStats *s = calloc(1, sizeof(*s));
    xdj_audio_stats_add(s, 0, 512, 512);
    xdj_audio_stats_add(s, 999999, 512, 2048);
    xdj_audio_stats_add(s, 1000000, 512, 511);
    assert(s->bins[0].callbacks == 2 && s->bins[0].frames == 1024);
    assert(s->bins[0].underruns == 0 && s->bins[0].missed_frames == 0);
    /* QEMU's callback returns the entire silent buffer when short; it
     * does not deliver the 511 pending frames in this invocation. */
    assert(s->bins[1].underruns == 1 && s->bins[1].missed_frames == 512);
    xdj_audio_stats_add(s, (uint64_t)XDJ_AUDIO_STATS_BINS * 1000000, 512, 0);
    assert(s->outside_bins == 1);
    free(s);
}
'''
        with tempfile.TemporaryDirectory() as d:
            source = Path(d) / 'stats.c'; source.write_text(code)
            binary = Path(d) / 'stats'
            subprocess.run(['cc', '-std=c11', '-I', str(COMMON), str(source),
                            '-o', str(binary)], check=True, capture_output=True)
            subprocess.run([str(binary)], check=True, capture_output=True)
