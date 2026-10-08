"""DA810 DMA transfers and completion interrupts use guest-programmed PaRAM."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
DEVICE = ROOT / 'native/qemu/hw/cdj/boards/xdj1000mk2'

class EdmaTests(unittest.TestCase):
    def test_block_copies_leave_the_same_memory_as_word_copies(self):
        code = r'''
#include <assert.h>
#include <string.h>
#include "da810_edma.h"
static unsigned char ram[1024];
static unsigned blocks, block_len;
static uint32_t read_mem(void *p, uint32_t a, unsigned n) {
    uint32_t v=0; assert(a+n<=sizeof ram); memcpy(&v,ram+a,n); return v;
}
static void write_mem(void *p, uint32_t a, uint32_t v, unsigned n) {
    assert(a+n<=sizeof ram); memcpy(ram+a,&v,n);
}
static void irq(void *p, unsigned region, int level) {}
/* The board's rule: plain memory only, and never ranges that overlap. */
static bool copy_mem(void *p, uint32_t src, uint32_t dst, unsigned len) {
    if (src+len>sizeof ram || dst+len>sizeof ram || (src<dst+len && dst<src+len)) return false;
    memcpy(ram+dst,ram+src,len); blocks++; block_len=len; return true;
}
static void run(int block, uint32_t opt, uint32_t src, uint32_t dst, unsigned acnt, unsigned bcnt,
                int sbidx, int dbidx, unsigned char *out) {
    Da810Edma s;
    da810_edma_init(&s, read_mem, write_mem, irq, 0);
    if (block) s.copy_mem = copy_mem;
    for (unsigned i=0;i<sizeof ram;i++) ram[i]=(unsigned char)(i*7+3);
    uint32_t words[]={ opt|(3u<<12), src, acnt|(bcnt<<16), dst,
                       (uint16_t)sbidx|((uint32_t)(uint16_t)dbidx<<16), 0xffff, 0, 1 };
    da810_edma_write(&s,0x348,8,4);  /* DRAE1: region 1 owns channel 3 */
    for(unsigned i=0;i<8;i++) da810_edma_write(&s,0x4060+4*i,words[i],4);
    da810_edma_write(&s,0x2230,8,4);
    da810_edma_event(&s,3,1);
    /* the transfer ran: its first byte arrived (a fixed destination keeps the last) */
    assert((opt&2) || ram[dst]==(unsigned char)(src*7+3));
    memcpy(out,ram,sizeof ram);
}
static void same(uint32_t opt, uint32_t src, uint32_t dst, unsigned acnt, unsigned bcnt,
                 int sbidx, int dbidx, unsigned expected_blocks, unsigned expected_len) {
    unsigned char words[sizeof ram], block[sizeof ram];
    run(0,opt,src,dst,acnt,bcnt,sbidx,dbidx,words);
    blocks=block_len=0;
    run(1,opt,src,dst,acnt,bcnt,sbidx,dbidx,block);
    assert(!memcmp(words,block,sizeof ram));
    assert(blocks==expected_blocks && (!expected_blocks || block_len==expected_len));
}
int main(void) {
    enum { SAM=1, DAM=2, SYNCDIM=4 };
    same(0,       0, 512, 64, 1, 0, 0, 1, 64);       /* one array, one block */
    same(0,       0, 512, 62, 1, 0, 0, 1, 62);       /* any length, not only whole words */
    same(SYNCDIM, 0, 512, 16, 4, 16, 16, 1, 64);     /* arrays laid end to end: one frame */
    same(SYNCDIM, 0, 512, 16, 4, 32, 16, 4, 16);     /* gaps on one side: one block per array */
    same(0,       0, 4,   64, 1, 0, 0, 0, 0);        /* overlapping forward copy repeats: by words */
    same(0,       4, 0,   64, 1, 0, 0, 0, 0);
    same(SAM,     0, 512, 64, 1, 0, 0, 0, 0);        /* a fixed-address side is a register, not memory */
    same(DAM,     0, 512, 64, 1, 0, 0, 0, 0);
    same(SYNCDIM|DAM, 0, 512, 16, 4, 16, 16, 0, 0);
    return 0;
}
'''
        with tempfile.TemporaryDirectory() as d:
            src = Path(d)/'block.c'; src.write_text(code); binary = Path(d)/'block'
            build = subprocess.run(['cc','-std=c11','-O2','-I',str(DEVICE),str(src),str(DEVICE/'da810_edma.c'),
                                    '-o',str(binary)],capture_output=True,text=True)
            self.assertEqual(build.returncode,0,build.stderr)
            result = subprocess.run([str(binary)],capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stderr)

    def test_fixed_channel_mapping_transfer_link_and_shadow_interrupt(self):
        code = r'''
#include <assert.h>
#include <string.h>
#include "da810_edma.h"
static unsigned char ram[256];
static unsigned irq_bits, irq_rises;
static uint32_t read_mem(void *p, uint32_t a, unsigned n) {
    uint32_t v=0; assert(a+n<=256); memcpy(&v,ram+a,n); return v;
}
static void write_mem(void *p, uint32_t a, uint32_t v, unsigned n) {
    assert(a+n<=256); memcpy(ram+a,&v,n);
}
static void irq(void *p, unsigned region, int level) {
    if(level) { irq_bits |= 1u<<region; irq_rises++; }
    else irq_bits &= ~(1u<<region);
}
int main(void) {
    Da810Edma s;
    da810_edma_init(&s, read_mem, write_mem, irq, 0);
    assert(da810_edma_read(&s,4,4)==0x00213344);
    da810_edma_write(&s,0x348,8,4);  /* DRAE1: channel/TCC 3 */
    da810_edma_write(&s,0x2260,8,4); /* shadow IESR */
    uint32_t words[]={ (1u<<20)|(3u<<12), 0, 4|(2u<<16), 128,
                       4|(4u<<16), 0xffff, 0, 1 };
    for(unsigned i=0;i<8;i++) da810_edma_write(&s,0x4060+4*i,words[i],4);
    ram[0]=0x12; ram[4]=0x34;
    da810_edma_write(&s,0x2230,8,4); /* EESR enables peripheral events */
    da810_edma_event(&s,3,1);
    assert(ram[128]==0x12 && ram[132]==0 && irq_bits==0);
    da810_edma_event(&s,3,0); da810_edma_event(&s,3,1);
    assert(ram[132]==0x34 && irq_bits==(1u<<1));
    assert(da810_edma_read(&s,0x2268,4)==8);
    unsigned rises=irq_rises;
    /* PaRAM/register writes while IPR is already asserted must not create
     * another CPU edge. IEVAL explicitly requests re-evaluation. */
    da810_edma_write(&s,0x4080,0,4);
    da810_edma_write(&s,0x284,0,4);
    assert(irq_rises==rises && irq_bits==(1u<<1));
    da810_edma_write(&s,0x2278,1,4);
    assert(irq_rises==rises+1 && irq_bits==(1u<<1));
    da810_edma_write(&s,0x2270,8,4);
    assert(irq_bits==0);
    assert(da810_edma_read(&s,0x5000,4)==0);
    da810_edma_event(&s,50,1); /* only 32 channels on DA810 */
    assert(irq_bits==0);
    /* Completing channel3 reloads its linked PaRAM, but does not move
     * the next buffer until a new peripheral edge arrives. */
    uint32_t first[]={ (1u<<20)|(3u<<12), 8, 4|(1u<<16), 136,
                       0, 0x4080, 0, 1 };
    uint32_t next[]={ (1u<<20)|(3u<<12), 12, 4|(1u<<16), 140,
                      0, 0xffff, 0, 1 };
    for(unsigned i=0;i<8;i++) {
        da810_edma_write(&s,0x4060+4*i,first[i],4);
        da810_edma_write(&s,0x4080+4*i,next[i],4);
    }
    ram[8]=0x56; ram[12]=0x78;
    da810_edma_event(&s,3,0); da810_edma_event(&s,3,1);
    assert(ram[136]==0x56 && ram[140]==0);
    assert(da810_edma_read(&s,0x4064,4)==12);
    assert(irq_bits==(1u<<1));
    da810_edma_write(&s,0x2270,8,4);
    da810_edma_event(&s,3,0); da810_edma_event(&s,3,1);
    assert(ram[140]==0x78 && irq_bits==(1u<<1));
    return 0;
}
'''
        with tempfile.TemporaryDirectory() as d:
            src = Path(d) / 'test.c'; src.write_text(code)
            binary = Path(d) / 'test'
            subprocess.run(['cc', '-std=c11', '-Wall', '-Werror', '-I', str(DEVICE),
                            str(src), str(DEVICE / 'da810_edma.c'), '-o', str(binary)],
                           check=True, capture_output=True)
            subprocess.run([str(binary)], check=True, capture_output=True)
