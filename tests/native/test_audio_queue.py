"""Catch-up must not leave stale PCM blocking current firmware audio."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / 'native/qemu/hw/cdj/boards/xdj1000mk2/dsp_da810.c'


def queue_constants(source):
    # Single source of truth: the queue geometry defined beside the device.
    start=source.index('#define PCM_QUEUE_FRAMES')
    return source[start:source.index('\n\n',start)]+'\n'


class AudioQueueTests(unittest.TestCase):
    def test_overrun_recovers_recent_frames_without_moving_an_inflight_read(self):
        source = SOURCE.read_text()
        start = 'static void pcm_recover(' if 'static void pcm_recover(' in source else 'static void audio_sample('
        body = source[source.index(start):source.index('static void timer0_event(')]
        code = r'''
#include <assert.h>
#include <stdint.h>
#include <stdbool.h>
#include <stdio.h>
#include <string.h>
#define pcm_timeline(...) ((void)0)
#define MIN(a,b) ((a)<(b)?(a):(b))
#define MAX(a,b) ((a)>(b)?(a):(b))
#define CDJ_DSP_HOST_GUARD(h) assert(!locked); bool guard __attribute__((cleanup(unlock))) = (locked=true)
static bool locked;
static void unlock(bool *guard) { assert(locked); locked=false; }
''' + queue_constants(source) + r'''
enum { Q=PCM_QUEUE_FRAMES, KEEP=PCM_RECOVER_FRAMES, CUT=PCM_QUEUE_FRAMES-PCM_RECOVER_FRAMES };
typedef struct CdjC6747 {
    unsigned pcm_lock; uint8_t pcm[PCM_QUEUE_FRAMES*8]; unsigned pcm_head,pcm_length;
    int32_t left_sample; uint64_t pcm_frames,pcm_consumed_frames,pcm_overflows,pcm_nonzero,pcm_peak,pcm_resyncs;
    bool pcm_reading,pcm_resync_pending;
    int64_t pcm_last_overflow_ns; struct { int64_t dsp_ns; } host;
    FILE *pcm_capture,*pcm_consumed; void *voice;
} CdjC6747;
static void qemu_mutex_lock(unsigned *p) { assert(!*p); *p=1; }
static void qemu_mutex_unlock(unsigned *p) { assert(*p); *p=0; }
static void stl_le_p(void *p,uint32_t v) { memcpy(p,&v,4); }
static uint32_t word(const uint8_t *p) { uint32_t v; memcpy(&v,p,4); return v; }
static unsigned AUD_write(void *,const void *,unsigned);
''' + body + r'''
static CdjC6747 *active;
static unsigned inject,accepted,output_count;
static uint8_t output[PCM_QUEUE_FRAMES*8];
static void frame(CdjC6747 *s,unsigned n) {
    audio_sample(s,1,0,n<<16); audio_sample(s,1,1,-(int32_t)(n<<16));
}
static unsigned AUD_write(void *voice,const void *data,unsigned length) {
    assert(!locked);
    if(inject) {
        unsigned head=active->pcm_head;
        frame(active,Q);
        /* A producer cannot invalidate bytes already copied for AUD_write. */
        assert(active->pcm_head==head && active->pcm_length==Q*8);
        inject=0;
    }
    unsigned n=MIN(length,accepted);
    memcpy(output+output_count,data,n); output_count+=n; return n;
}
static void fill(CdjC6747 *s) {
    memset(s,0,sizeof *s); active=s; output_count=0;
    for(unsigned i=0;i<Q;i++) frame(s,i);
}
int main(void) {
    CdjC6747 s;
    /* The cushion must cover a burst, and survive the overflow it follows. */
    assert(KEEP*1000/44100>=250 && CUT>=KEEP);
    fill(&s); frame(&s,Q);
    assert(s.pcm_length==(KEEP+1)*8 && s.pcm_head==CUT*8);
    assert(s.pcm_overflows==CUT && s.pcm_resyncs==1 && s.pcm_frames==Q+1);
    accepted=4096; audio_callback(&s,(KEEP+1)*8);
    assert(s.pcm_length==0 && !s.pcm_reading);
    for(unsigned i=0;i<KEEP+1;i++) {
        assert(word(output+i*8)==(CUT+i)<<16);
        assert(word(output+i*8+4)==-(uint32_t)((CUT+i)<<16));
    }
    /* Inject an overrun while the audio core owns a copied buffer. Both a
     * short write and a zero write must release it before queue recovery. */
    for(unsigned pass=0;pass<2;pass++) {
        fill(&s); accepted=pass?128*8:0; inject=1;
        audio_callback(&s,128*8);
        assert(!s.pcm_reading && !s.pcm_resync_pending);
        assert(s.pcm_length==KEEP*8 && s.pcm_head==CUT*8);
        assert(s.pcm_overflows==CUT+1-(pass?128:0) && s.pcm_resyncs==1);
        assert(output_count==(pass?128*8:0));
        for(unsigned i=0;i<output_count/8;i++) assert(word(output+i*8)==i<<16);
        output_count=0; accepted=4096; audio_callback(&s,KEEP*8);
        assert(s.pcm_length==0);
        for(unsigned i=0;i<KEEP;i++) assert(word(output+i*8)==(CUT+i)<<16);
    }
    /* Ordinary partial writes preserve every sample and do not resync. */
    memset(&s,0,sizeof s); active=&s; output_count=0;
    for(unsigned i=0;i<600;i++) frame(&s,i);
    accepted=128*8; audio_callback(&s,600*8);
    assert(output_count==600*8 && s.pcm_length==0 && s.pcm_overflows==0);
    for(unsigned i=0;i<600;i++) assert(word(output+i*8)==i<<16);
    audio_callback(&s,4096); assert(!s.pcm_reading);
}
'''
        with tempfile.TemporaryDirectory() as d:
            src = Path(d) / 'queue.c'; src.write_text(code)
            binary = Path(d) / 'queue'
            build = subprocess.run(['cc', '-std=c11', '-O2', str(src), '-o', str(binary)],
                                   capture_output=True, text=True)
            self.assertEqual(build.returncode, 0, build.stderr)
            result = subprocess.run([str(binary)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_audio_feeder_can_drain_while_dsp_execution_is_busy(self):
        source = SOURCE.read_text()
        body = source[source.index('static void pcm_recover('):source.index('static void timer0_event(')]
        code = r"""
#include <assert.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#define pcm_timeline(...) ((void)0)
#define MIN(a,b) ((a)<(b)?(a):(b))
#define MAX(a,b) ((a)>(b)?(a):(b))
/* The native DSP is busy: output must never try to acquire its lock. */
#define CDJ_DSP_HOST_GUARD(h) assert(!"audio feeder waited for DSP execution")
""" + queue_constants(source) + r"""
typedef struct CdjC6747 {
    unsigned pcm_lock; uint8_t pcm[PCM_QUEUE_FRAMES*8]; unsigned pcm_head,pcm_length;
    int32_t left_sample; uint64_t pcm_frames,pcm_consumed_frames,pcm_overflows,pcm_nonzero,pcm_peak,pcm_resyncs;
    bool pcm_reading,pcm_resync_pending;
    int64_t pcm_last_overflow_ns; struct { int64_t dsp_ns; } host;
    FILE *pcm_capture,*pcm_consumed; void *voice;
} CdjC6747;
static CdjC6747 *active;
static void qemu_mutex_lock(unsigned *p) { assert(!*p); *p=1; }
static void qemu_mutex_unlock(unsigned *p) { assert(*p); *p=0; }
static void stl_le_p(void *p,uint32_t v) { memcpy(p,&v,4); }
static unsigned delivered;
static unsigned AUD_write(void *voice,const void *data,unsigned length) {
    assert(!active->pcm_lock); delivered+=length; return length;
}
""" + body + r"""
int main(void) {
    CdjC6747 s={0}; active=&s;
    for(unsigned i=0;i<2048;i++) {
        audio_sample(&s,1,0,i<<16); audio_sample(&s,1,1,-(int32_t)(i<<16));
    }
    audio_callback(&s,2048*8);
    assert(delivered==2048*8 && !s.pcm_length && !s.pcm_reading && !s.pcm_lock);
}
"""
        with tempfile.TemporaryDirectory() as d:
            src=Path(d)/'feeder.c';src.write_text(code);binary=Path(d)/'feeder'
            build=subprocess.run(['cc','-std=c11','-O2',str(src),'-o',str(binary)],capture_output=True,text=True)
            self.assertEqual(build.returncode,0,build.stderr)
            result=subprocess.run([str(binary)],capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stderr)
