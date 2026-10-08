"""Full-width DSP DMA RAM bounds and little-endian unaligned transfers."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
DEVICE = ROOT/'native/qemu/hw/cdj/boards/xdj1000mk2'


class DspMemoryTests(unittest.TestCase):
    def check(self, code, extra=()):
        with tempfile.TemporaryDirectory() as d:
            src=Path(d)/'test.c';src.write_text(code);binary=Path(d)/'test'
            build=subprocess.run(['cc','-std=c11','-Wall','-Werror','-fsanitize=address,undefined',
                                  '-I',str(DEVICE),str(src),str(DEVICE/'dsp_memory.c'),
                                  *[str(DEVICE/name) for name in extra],'-o',str(binary)],
                                 capture_output=True,text=True)
            self.assertEqual(build.returncode,0,build.stderr)
            result=subprocess.run([str(binary)],capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stderr)

    def test_all_memory_aliases_exact_width_and_atomic_rejection(self):
        code = r'''
#include <assert.h>
#include <string.h>
#include "dsp_memory.h"
int main(void) {
    const uint32_t base[]={0x00800000,0x11800000,0x00e00000,0x11e00000,
        0x00f00000,0x11f00000,0x80000000,0xc0000000};
    const uint32_t size[]={0x40000,0x40000,0x8000,0x8000,0x8000,0x8000,0x20000,0x4000000};
    for(unsigned i=0;i<8;i++) {
        assert(xdj_dsp_ram_remaining(base[i])==size[i]);
        assert(xdj_dsp_ram_remaining(base[i]+size[i]-1)==1);
        assert(xdj_dsp_ram_remaining(base[i]+size[i])==0);
        assert(xdj_dsp_ram_remaining(base[i]-1)==0);
    }
    assert(xdj_dsp_ram_remaining(0xffffffff)==0);
    uint8_t bytes[8]={0x11,0x22,0x33,0x44,0x55,0x66,0x77,0x88};uint32_t value;
    assert(xdj_dsp_ram_read(bytes+1,7,4,&value) && value==0x55443322);
    assert(xdj_dsp_ram_read(bytes+3,5,2,&value) && value==0x5544);
    assert(xdj_dsp_ram_read(bytes+7,1,1,&value) && value==0x88);
    assert(!xdj_dsp_ram_read(bytes+5,3,4,&value) && value==0);
    assert(!xdj_dsp_ram_read(bytes,8,3,&value));
    assert(!xdj_dsp_ram_read(bytes,8,0,&value));
    assert(!xdj_dsp_ram_read(0,8,1,&value));
    uint8_t before[8];memcpy(before,bytes,8);
    assert(!xdj_dsp_ram_write(bytes+6,2,4,0xffffffff));
    assert(!memcmp(before,bytes,8)); /* reject before any partial write */
    assert(!xdj_dsp_ram_write(bytes,8,3,0));
    assert(!xdj_dsp_ram_write(0,8,1,0));
    assert(xdj_dsp_ram_write(bytes+1,7,4,0xf1e2d3c4));
    assert(bytes[0]==0x11 && bytes[1]==0xc4 && bytes[2]==0xd3 && bytes[3]==0xe2 && bytes[4]==0xf1 && bytes[5]==0x66);
    assert(xdj_dsp_ram_write(bytes+6,2,2,0x1234) && bytes[6]==0x34 && bytes[7]==0x12);
    assert(xdj_dsp_ram_write(bytes+7,1,1,0xab) && bytes[7]==0xab);
}
'''
        self.check(code)

    def test_edma_transfers_unaligned_words_and_flags_region_crossing(self):
        self.check(r'''
#include <assert.h>
#include <string.h>
#include "dsp_memory.h"
#include "da810_edma.h"
static uint8_t ram[0x20000];
static unsigned faults;
static uint8_t *ptr(uint32_t addr) {
    return addr-0x80000000u < sizeof ram ? ram+(addr-0x80000000u) : 0;
}
static uint32_t read_mem(void *opaque, uint32_t addr, unsigned size) {
    uint32_t value;
    if (!xdj_dsp_ram_read(ptr(addr),xdj_dsp_ram_remaining(addr),size,&value)) faults++;
    return value;
}
static void write_mem(void *opaque, uint32_t addr, uint32_t value, unsigned size) {
    if (!xdj_dsp_ram_write(ptr(addr),xdj_dsp_ram_remaining(addr),size,value)) faults++;
}
static void irq(void *opaque, unsigned region, int level) {}
static void transfer(Da810Edma *s, uint32_t src, uint32_t dst) {
    uint32_t words[]={0,src,4|(1u<<16),dst,0,0xffff,0,1};
    for(unsigned i=0;i<8;i++) da810_edma_write(s,0x4000+4*i,words[i],4);
    da810_edma_event(s,0,0);da810_edma_event(s,0,1);
}
int main(void) {
    Da810Edma s;da810_edma_init(&s,read_mem,write_mem,irq,0);
    da810_edma_write(&s,0x1030,1,4);
    memset(ram,0xaa,sizeof ram);
    ram[1]=0x12;ram[2]=0x34;ram[3]=0x56;ram[4]=0x78;
    transfer(&s,0x80000001,0x80000009);
    assert(!faults && !memcmp(ram+1,ram+9,4) && ram[8]==0xaa && ram[13]==0xaa);
    transfer(&s,0x80000001,0x8001fffc);
    assert(!faults && !memcmp(ram+1,ram+sizeof ram-4,4));
    uint8_t before[4];memcpy(before,ram+sizeof ram-4,4);
    transfer(&s,0x80000001,0x8001fffd);
    assert(faults==1 && !memcmp(before,ram+sizeof ram-4,4));
    transfer(&s,0x8001fffd,0x80000010);
    assert(faults==2 && ram[16]==0 && ram[17]==0 && ram[18]==0 && ram[19]==0);
}
''', ('da810_edma.c',))
