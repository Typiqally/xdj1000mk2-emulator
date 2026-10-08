"""Exercise the native DSP bus, timer clock and interrupt routes together."""
from pathlib import Path
import subprocess, tempfile, unittest
ROOT = Path(__file__).resolve().parents[2]
DEVICE = ROOT / "native/qemu/hw/cdj/boards/xdj1000mk2"

class TimerBusTests(unittest.TestCase):
    def test_native_addresses_clock_gate_and_interrupt_mapping(self):
        source = (DEVICE / "dsp_da810.c").read_text()
        bus = source[source.index("static uint32_t dsp_bus_read("):source.index("static uint32_t dma_read(")]
        advance = source[source.index("static void dsp_advance("):source.index("static void dma_irq(")]
        routes = source[source.index("static void timer0_event("):source.index("static void dsp_advance(")]
        code = r'''
#include <assert.h>
#include <stdbool.h>
#include <stdint.h>
#include "da810_timer.h"
#include "da810_pll.h"
#include "da810_intc.h"
#include "da810_syscfg.h"
#include "hardware_profile.h"
#define UHPI_HPIC 0x01e10030u
#define SPI1_SPIBUF 0x01e12040u
#define SPIBUF_RXEMPTY 0
#define GPIO_DIR 0
#define GPIO_OUT 4
#define GPIO_IN 16
#define GPIO_CMD_PAIR 2
#define MCASP_BASE 0x01d00000u
#define MCASP_STRIDE 0x4000
static unsigned peripherals;
static unsigned gaps;
#define dsp_unmodeled(...) (++gaps)
typedef struct CdjC6747 {
    Da810Intc intc;Da810Pll pll;Da810Syscfg syscfg;Da810Timer timer[2];bool clock_override;
    struct { unsigned busr,busw,pin_reads; } host;
    unsigned edma,mcasp[3],gpio_dir[5],gpio_out[5];
} CdjC6747;
static void xdj_hardware_violation(const char *reason, uint64_t value) { assert(false); }
static void cdj_dsp_count(void *p,uint32_t a,uint32_t v) {}
static void cdj_dsp_host_clock(void *p,uint64_t hz) {}
static uint32_t cdj_dsp_host_hpic(void *p) {return 0;}
static void cdj_dsp_host_dsp_hpic(void *p,uint32_t v) {}
static uint32_t da810_edma_read(void *p,uint32_t a,unsigned n) {return 0;}
static void da810_edma_write(void *p,uint32_t a,uint32_t v,unsigned n) {}
static uint32_t da810_mcasp_read(void *p,uint32_t a,unsigned n) {return 0;}
static void da810_mcasp_write(void *p,uint32_t a,uint32_t v,unsigned n) {}
static void da810_mcasp_advance(void *p,uint64_t ns) {peripherals++;}
static bool gpio_reg(uint32_t a,unsigned *p,unsigned *r) {return false;}
static uint32_t gpio_in(void *p,unsigned n) {return 0;}
static void gpio_write(void *p,unsigned n,unsigned r,uint32_t v) {}
''' + bus + routes + advance + r'''
#define W(a,v) dsp_bus_write(&s,a,v,4)
#define R(a) dsp_bus_read(&s,a,4)
int main(void) {
    CdjC6747 s={0};da810_intc_init(&s.intc,0,0);da810_pll_init(&s.pll,22579200);
    da810_timer_init(&s.timer[0],timer0_event,&s);
    da810_timer_init(&s.timer[1],timer1_event,&s);
    assert(R(0x01c20000)==0x4472020c && R(0x01c21000)==0x4472020c);
    assert(R(0x01c22000)==0); /* DA810 has only two GP timers */
    assert(gaps==1);
    W(0x01c20024,7);W(0x01c20018,225);W(0x01c2001c,225);W(0x01c20020,0x00800080);
    W(0x01c21024,7);W(0x01c21018,225);W(0x01c2101c,225);W(0x01c21020,0x00800080);
    dsp_advance(&s,10000);
    assert(R(0x01c20010)==225 && R(0x01c21014)==225);
    assert(s.intc.flag[0]&(1u<<4));assert(s.intc.flag[2]&1);
    assert(s.intc.flag[1]&(1u<<8));assert(s.intc.flag[1]&(1u<<16));
    W(0x01c11000+0x148,2);dsp_advance(&s,10000);
    assert(R(0x01c20010)==225 && peripherals==6);
    W(0x01c11000+0x148,3);dsp_advance(&s,10000);
    assert(R(0x01c20010)==225); /* fractional AUXCLK carry supplies 226 clocks */
}
'''
        with tempfile.TemporaryDirectory() as d:
            src=Path(d)/"test.c";src.write_text(code);binary=Path(d)/"test"
            build=subprocess.run(["cc","-std=c11","-O2","-I",str(DEVICE),str(src),
                str(DEVICE/"da810_timer.c"),str(DEVICE/"da810_pll.c"),
                str(DEVICE/"da810_intc.c"),str(DEVICE/"da810_syscfg.c"),"-o",str(binary)],capture_output=True,text=True)
            self.assertEqual(build.returncode,0,build.stderr)
            run=subprocess.run([str(binary)],capture_output=True,text=True)
            self.assertEqual(run.returncode,0,run.stderr)
