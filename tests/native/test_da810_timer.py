"""DA810 general-purpose timer behavior from SPRUH96B chapter 26."""
from pathlib import Path
import subprocess,tempfile,unittest
ROOT=Path(__file__).resolve().parents[2]
DEVICE=ROOT/'native/qemu/hw/cdj/boards/xdj1000mk2'
class TimerTests(unittest.TestCase):
    def test_64_bit_latch_period_interrupt_reload_and_fractional_clock(self):
        self.check(r'''
    Da810Timer t;da810_timer_init(&t,event,0);
    assert(R(0)==0x4472020c && R(0x24)==0);
    W(0x18,10);W(0x24,3);W(0x44,1);W(0x20,0x80);
    A(9);assert(R(0x10)==9 && !events);
    A(1);assert(R(0x10)==10 && events==1 && (R(0x44)&2));
    W(0x44,3);assert(R(0x44)==1);
    A(1);assert(R(0x10)==0 && events==1);
    A(10);assert(events==2 && R(0x10)==10);
    W(0x20,0);A(100);assert(R(0x10)==10);
    W(0x10,0);W(0x20,0x40);A(100);assert(R(0x10)==10 && events==3);
    A(100);assert(R(0x10)==10 && events==3);
    W(0x24,0);assert(R(0x10)==10 && R(0x14)==0);A(100);assert(R(0x10)==10);W(0x10,0);
    W(0x10,0xfffffffe);W(0x14,1);W(0x18,0xffffffff);W(0x1c,2);W(0x24,3);W(0x20,0x80);
    assert(R(0x10)==0xfffffffe);A(2);
    assert(R(0x14)==1); /* high half from low-half read, before carry */
    assert(R(0x10)==0 && R(0x14)==2);
    W(0x24,0);W(0x10,0);W(0x14,0);W(0x18,1000000);W(0x1c,0);W(0x24,3);W(0x20,0x80);
    for(unsigned i=0;i<1000;i++)da810_timer_advance(&t,10000,22579200);
    assert(R(0x10)==225792 && t.fraction==0);
    da810_timer_advance(&t,10000,0);assert(R(0x10)==225792);
    W(0x24,0);W(0x10,0);W(0x18,3);W(0x34,5);W(0x24,0x13);W(0x20,0xc0);
    A(3);assert(R(0x10)==3 && R(0x18)==3);
    A(1);assert(R(0x10)==0 && R(0x18)==5);
    A(5);assert(R(0x10)==5);
    W(0,0);assert(R(0)==0x4472020c);
''')

    def test_independent_32_bit_halves_prescaler_and_read_reset(self):
        self.check(r'''
    Da810Timer t;da810_timer_init(&t,event,0);
    W(0x24,0x3007); /* unchained, /4 on upper half */
    W(0x18,100);W(0x1c,2);W(0x44,0x10001);W(0x20,0x00800080);
    A(3);assert(R(0x10)==3 && R(0x14)==0 && ((R(0x24)>>8)&15)==3);
    A(1);assert(R(0x10)==4 && R(0x14)==1);
    A(4);assert(R(0x10)==8 && R(0x14)==2 && last==1 && events==1);
    W(0x44,0x00030001);assert(!(R(0x44)&0x20000));
    A(4);assert(R(0x14)==0);
    W(0x24,0x3017);W(0x20,0x04800480); /* timer-plus read-reset */
    assert(R(0x10)==12 && R(0x10)==0);
    A(4);assert(R(0x14)==1 && R(0x14)==0);
    W(0x24,0x3005);assert(R(0x14)==0);A(100);assert(R(0x14)==0);
    da810_timer_write(&t,0x1a,0x1234,2);assert(R(0x18)==0x12340064);
''')

    def test_backward_features_and_independent_external_clock(self):
        self.check(r'''
    Da810Timer t;da810_timer_init(&t,event,0);
    W(0x24,7);W(0x18,100);W(0x1c,100);W(0x20,0x04800480);
    A(5);assert(R(0x10)==5 && R(0x10)==5 && R(0x14)==5 && R(0x14)==5);
    W(0x20,0x00800180); /* external low clock must not stop internal upper */
    A(5);assert(R(0x10)==5 && R(0x14)==10);
    W(8,0x30033);assert(R(8)==0x30033);
''')

    def test_chained_prescaler_clocks_lower_half_on_following_cycle(self):
        self.check(r'''
    Da810Timer t;da810_timer_init(&t,event,0);
    W(0x24,15);W(0x18,2);W(0x1c,2);W(0x20,0x80);
    A(2);assert(R(0x10)==0 && R(0x14)==2 && !events);
    A(1);assert(R(0x10)==1 && R(0x14)==0 && !events);
    A(3);assert(R(0x10)==2 && R(0x14)==0 && events==1 && last==0);
    A(3);assert(R(0x10)==0 && R(0x14)==0 && events==1);
    W(0x10,0);W(0x20,0x40);A(100);
    assert(R(0x10)==2 && events==2);unsigned high=R(0x14);A(100);assert(R(0x14)==high);
    assert(!t.unsupported);
    W(0x24,31);W(0x10,0);W(0x14,0);W(0x34,5);W(0x38,3);W(0x20,0xc0);
    A(9);assert(R(0x10)==0 && R(0x18)==5 && R(0x1c)==3);
''')

    def check(self,body):
        code=r'''
#include <assert.h>
#include <stdint.h>
#include "da810_timer.h"
static unsigned events,last;
static void event(void *p,unsigned half) { events++;last=half; }
#define W(o,v) da810_timer_write(&t,o,v,4)
#define R(o) da810_timer_read(&t,o,4)
#define A(n) da810_timer_advance(&t,n,1000000000)
int main(void) {
''' + body + '\n}\n'
        with tempfile.TemporaryDirectory() as d:
            src=Path(d)/'test.c';src.write_text(code);binary=Path(d)/'test'
            build=subprocess.run(['cc','-std=c11','-Wall','-Werror','-I',str(DEVICE),str(src),str(DEVICE/'da810_timer.c'),'-o',str(binary)],capture_output=True,text=True)
            self.assertEqual(build.returncode,0,build.stderr)
            run=subprocess.run([str(binary)],capture_output=True,text=True)
            self.assertEqual(run.returncode,0,run.stderr)
