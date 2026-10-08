"""MAIN-only UHPI registers and reads of DSP memory must not stop the DSP worker."""
from pathlib import Path
import subprocess,tempfile,unittest
ROOT=Path(__file__).resolve().parents[2]
DEVICE=ROOT/'native/qemu/hw/cdj/boards/xdj1000mk2/dsp_da810.c'

class HpiTests(unittest.TestCase):
    def test_registers_and_ram_reads_do_not_arbitrate_stores_still_do(self):
        source=DEVICE.read_text()
        read=source[source.index('static uint64_t hpi_read('):source.index('/* The channel transfers synchronously')]
        write=source[source.index('static void hpi_write('):source.index('static const MemoryRegionOps hpi_ops')]
        code=r'''
#include <assert.h>
#include <stdbool.h>
#include <stdint.h>
#include <string.h>
typedef uint64_t hwaddr;
#define HPIC_DSPINT 2
#define HPIC_HINT 4
static unsigned guards,locked;
static void unlock(unsigned *p) { assert(locked);locked=0; }
#define CDJ_DSP_HOST_GUARD(h) unsigned guard __attribute__((cleanup(unlock))) = (++guards,locked=1)
typedef struct CdjC6747 {
    struct { uint32_t hpic;void *core;bool enabled; } host;
    uint32_t hpia;uint64_t reads,writes,dspints,dropped;unsigned polls,intc;
} CdjC6747;
static uint32_t ram[4];
/* The memory map is fixed once the machine exists: looking a word up needs no lock. */
static uint8_t *dsp_ram(CdjC6747 *s,uint32_t addr) { return addr<16?(uint8_t *)ram+addr:0; }
static unsigned word_loads;
/* One whole aligned word, as the board's host port latches it. */
static uint32_t load_word(const uint32_t *p) { assert(!locked && !((uintptr_t)p&3));word_loads++;return *p; }
#define qatomic_read__nocheck(p) load_word(p)
#define le32_to_cpu(v) (v)
#define qatomic_set(p,v) (*(p)=(v))
static void stl_le_p(void *p,uint32_t v) { assert(locked);memcpy(p,&v,4); }
static uint32_t cdj_dsp_host_hpic(const void *h) { return 4; }
static bool cdj_dsp_host_main_hpic(void *h,uint32_t val) { assert(locked);return val&2; }
static void cdj_dsp_count(void *p,uint32_t a,uint32_t v) { (void)p; }
static void c66x_invalidate(void *p,uint32_t a,uint32_t n) { assert(locked); }
static void hpi_note_write(CdjC6747 *s,uint32_t a) { assert(locked); }
static void dsp_unmodeled(CdjC6747 *s,uint32_t a,unsigned n,uint32_t v,bool write) { assert(locked); }
static void dsp_start(CdjC6747 *s) { assert(locked); }
static void da810_intc_event(void *s,unsigned e) { assert(locked && e==34); }
''' + read + write + r'''
int main(void) {
    CdjC6747 s={0};
    assert(hpi_read(&s,0,4)==4 && guards==0);
    hpi_write(&s,0x40000,8,4); assert(s.hpia==8 && guards==0);
    assert(hpi_read(&s,0x40000,4)==8 && guards==0);
    hpi_write(&s,0x80000,0x1234abcd,4);
    assert(guards==1 && !locked && s.hpia==12 && s.writes==1);
    /* A read of DSP memory takes the word without stopping the core... */
    s.hpia=8;assert(hpi_read(&s,0x80000,4)==0x1234abcd);
    assert(guards==1 && word_loads==1 && s.hpia==12 && s.reads==1);
    /* ...at the address rounded down to its word, and without the increment
     * through the other data register. */
    s.hpia=10;assert(hpi_read(&s,0xc0000,4)==0x1234abcd);
    assert(guards==1 && word_loads==2 && s.hpia==10 && s.reads==2);
    /* An address that is not memory is the chip's business: under the lock. */
    s.hpia=0x100;assert(hpi_read(&s,0x80000,4)==0);
    assert(guards==2 && !locked && word_loads==2 && s.hpia==0x104 && s.reads==3);
    hpi_write(&s,0,2,4);assert(guards==3 && !locked && s.dspints==1);
}
'''
        with tempfile.TemporaryDirectory() as d:
            src=Path(d)/'test.c';src.write_text(code);binary=Path(d)/'test'
            build=subprocess.run(['cc','-std=gnu11','-O2',str(src),'-o',str(binary)],capture_output=True,text=True)
            self.assertEqual(build.returncode,0,build.stderr)
            run=subprocess.run([str(binary)],capture_output=True,text=True)
            self.assertEqual(run.returncode,0,run.stderr)
