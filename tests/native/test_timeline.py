"""A bounded timeline retains the first failure without allocating or doing I/O."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[2]


class TimelineTests(unittest.TestCase):
    def test_pcm_events_correlate_host_and_guest_clock_without_execution_lock(self):
        root=Path(__file__).resolve().parents[2]
        directory=root/'native/qemu/hw/cdj/boards/cdj2000'
        source=(root/'native/qemu/hw/cdj/boards/xdj1000mk2/dsp_da810.c').read_text()
        body=source[source.index('static void pcm_timeline('):source.index('/* Called under the PCM lock.')]
        code=r'''
#include <assert.h>
#include <stdbool.h>
#include "dsp_timeline.h"
#define qatomic_read(p) (*(p))
#define qatomic_set(p,v) (*(p)=(v))
typedef struct { CdjTimeline pcm_timeline; unsigned pcm_length;
    struct { uint64_t dsp_ns; unsigned timeline_freeze; } host; } CdjC6747;
static uint64_t g_get_monotonic_time(void) { return 900; }
''' + body + r'''
int main(void) {
    CdjC6747 s={.pcm_length=8,.host.dsp_ns=123};
    cdj_timeline_reset(&s.pcm_timeline,true);
    pcm_timeline(&s,7,0,true);
    assert(s.pcm_timeline.event[0].host_us==900);
    assert(s.pcm_timeline.event[0].guest_ns==123);
    assert(s.pcm_timeline.frozen && s.host.timeline_freeze==1);
}
'''
        with tempfile.TemporaryDirectory() as d:
            src=Path(d)/'test.c';src.write_text(code);binary=Path(d)/'test'
            build=subprocess.run(['cc','-I',str(directory),str(src),'-o',str(binary)],capture_output=True,text=True)
            self.assertEqual(build.returncode,0,build.stderr)
            result=subprocess.run([str(binary)],capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stderr)

    def test_wrap_freeze_disable_and_reset(self):
        code=r'''
#include <assert.h>
#include "dsp_timeline.h"
int main(void) {
    CdjTimeline t={0};
    cdj_timeline_add(&t,1,2,3,4,5,6,0);assert(!t.total);
    cdj_timeline_reset(&t,1);
    for(unsigned n=0;n<CDJ_TIMELINE_CAP+3;n++) cdj_timeline_add(&t,n,n*10,3,4,5,6,0);
    assert(t.total==CDJ_TIMELINE_CAP+3 && t.count==CDJ_TIMELINE_CAP);
    assert(t.event[t.next].host_us==3);
    cdj_timeline_add(&t,9999,10000,7,8,9,10,1);
    assert(t.frozen && t.event[(t.next+CDJ_TIMELINE_CAP-1)%CDJ_TIMELINE_CAP].kind==7);
    uint64_t total=t.total;cdj_timeline_add(&t,0,0,0,0,0,0,0);assert(t.total==total);
    cdj_timeline_reset(&t,0);assert(!t.enabled && !t.frozen && !t.total && !t.count);
}
'''
        with tempfile.TemporaryDirectory() as d:
            source=Path(d)/'test.c';source.write_text(code);binary=Path(d)/'test'
            result=subprocess.run(['cc','-std=c11','-I',str(ROOT/'native/qemu/hw/cdj/boards/cdj2000'),str(source),'-o',str(binary)],capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stderr)
            run=subprocess.run([str(binary)],capture_output=True,text=True)
            self.assertEqual(run.returncode,0,run.stderr)
