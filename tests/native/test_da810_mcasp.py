"""McASP generates real DMA requests and serializes guest sample words."""
from pathlib import Path
import subprocess
import tempfile
import unittest
ROOT = Path(__file__).resolve().parents[2]
DEVICE = ROOT / 'native/qemu/hw/cdj/boards/xdj1000mk2'
class McaspTests(unittest.TestCase):
    def test_reset_ready_dma_slots_pcm_and_clock(self):
        code = r'''
#include <assert.h>
#include "da810_mcasp.h"
static unsigned events, samples;
static int32_t pcm[8];
static void event(void *p, unsigned ch) { assert(ch==3); events++; }
static void sample(void *p, unsigned port, unsigned slot, int32_t v) {
    assert(port==1 && slot==(samples&1)); pcm[samples++]=v;
}
int main(void) {
    Da810Mcasp s;
    da810_mcasp_init(&s,1,event,sample,0);
    da810_mcasp_write(&s,0xa4,0xffffffff,4);
    da810_mcasp_write(&s,0xa8,0x180f0,4);
    da810_mcasp_write(&s,0xac,0x113,4);
    da810_mcasp_write(&s,0xb0,0xe7,4);
    da810_mcasp_write(&s,0xb4,0x8000,4);
    da810_mcasp_write(&s,0xb8,3,4);
    da810_mcasp_write(&s,0x180,0xd,4);
    da810_mcasp_write(&s,0xa0,0x400,4);
    assert(da810_mcasp_read(&s,0x44,4)==0x400);
    assert(da810_mcasp_read(&s,0xc0,4)&0x20);
    assert(events==1);
    da810_mcasp_write(&s,0x2000,0x12340000,4);
    assert(!(da810_mcasp_read(&s,0xc0,4)&0x20));
    da810_mcasp_write(&s,0xa0,0x1f00,4);
    da810_mcasp_advance(&s,11000);
    assert(samples==0);
    da810_mcasp_advance(&s,1000);
    assert(samples==1 && pcm[0]==0x12340000 && events==2);
    da810_mcasp_write(&s,0x2000,0xfedc0000,4);
    da810_mcasp_advance(&s,11000);
    assert(samples==2 && pcm[1]==(int32_t)0xfedc0000 && events==3);
    da810_mcasp_write(&s,0xa0,0,4);
    da810_mcasp_advance(&s,1000000);
    assert(samples==2 && da810_mcasp_read(&s,0x44,4)==0);
    return 0;
}
'''
        with tempfile.TemporaryDirectory() as d:
            src = Path(d) / 'test.c'; src.write_text(code)
            binary = Path(d) / 'test'
            subprocess.run(['cc','-std=c11','-Wall','-Werror','-I',str(DEVICE),
                            str(src),str(DEVICE/'da810_mcasp.c'),'-o',str(binary)],
                           check=True,capture_output=True)
            subprocess.run([str(binary)],check=True,capture_output=True)
