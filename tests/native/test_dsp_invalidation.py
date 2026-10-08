"""Audio DMA must avoid decoded-packet walks on data-only pages."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / 'native/qemu/hw/cdj/c6x/c66x_mem.c'


class DspInvalidationTests(unittest.TestCase):
    def test_data_fast_path_and_code_aliases_across_page_boundary(self):
        source = SOURCE.read_text()
        body = source[source.index('void invalidate_code('):
                      source.index('static void idle_dma_store(')]
        # Count packet metadata reads without imposing a host-speed threshold.
        body = body.replace('fpblk **pg =', 'metadata_reads++; fpblk **pg =')
        code = r'''
#include <assert.h>
#include <stdint.h>
#include <string.h>
#include <stddef.h>
#define FP_PAGE_SHIFT 12
#define CP_CODE 1
typedef struct { unsigned pk_n; } insn;
typedef struct { unsigned valid; insn insn[16]; } fpblk;
typedef struct {
    uint32_t base,size; uint8_t *host; fpblk ***fpp; uint8_t *codepage;
} ramreg;
typedef struct {
    ramreg ram[2]; unsigned nram,last_ram,nxpk,code_gen; uint8_t pkc[16];
} c66x_core;
static unsigned metadata_reads;
static uint8_t *ram_ptr(c66x_core *c,uint32_t a,unsigned n) {
    for(unsigned i=0;i<c->nram;i++) {
        ramreg *r=&c->ram[i];
        if(r->size>=n && a-r->base<=r->size-n) {
            c->last_ram=i; return r->host+(a-r->base);
        }
    }
    return NULL;
}
''' + body + r'''
int main(void) {
    uint8_t ram[8192]={0},pages[2]={0};
    fpblk *slots[2][2][128]={0};
    fpblk **tables[2][2]={{slots[0][0],slots[0][1]},
                         {slots[1][0],slots[1][1]}};
    c66x_core c={.nram=2};
    c.ram[0]=(ramreg){0x10000,sizeof ram,ram,tables[0],pages};
    c.ram[1]=(ramreg){0x20000,sizeof ram,ram,tables[1],pages};
    /* The high-rate audio-data path needs no instruction packet reads. */
    invalidate_code(&c,0x20000+4096,4);
    assert(metadata_reads==0 && c.code_gen==0);
    /* Both addresses share page flags, but have separate decoded packets. */
    fpblk blocks[2][2]={0};
    pages[0]=pages[1]=CP_CODE;
    for(unsigned i=0;i<2;i++) for(unsigned j=0;j<2;j++) {
        blocks[i][j].valid=1; blocks[i][j].insn[0].pk_n=7;
        slots[i][j][j?0:127]=&blocks[i][j];
    }
    memset(c.pkc,1,sizeof c.pkc); c.nxpk=10;
    invalidate_code(&c,0x20000+4092,8);
    assert(c.code_gen==4 && c.nxpk==0);
    for(unsigned i=0;i<2;i++) for(unsigned j=0;j<2;j++)
        assert(!blocks[i][j].valid && !blocks[i][j].insn[0].pk_n);
    for(unsigned i=0;i<sizeof c.pkc;i++) assert(!c.pkc[i]);
    /* A repeated notification changes no generation; zero length is inert. */
    invalidate_code(&c,0x10000+4092,8);
    invalidate_code(&c,0x10000,0);
    assert(c.code_gen==4);
}
'''
        with tempfile.TemporaryDirectory() as d:
            src = Path(d) / 'invalidation.c'; src.write_text(code)
            binary = Path(d) / 'invalidation'
            subprocess.run(['cc', '-std=c11', '-O2', str(src), '-o', str(binary)],
                           check=True, capture_output=True)
            result = subprocess.run([str(binary)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
