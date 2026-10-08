"""SH7724 TPU register/timing contracts from Renesas chapter 21."""
from pathlib import Path
import subprocess
import tempfile
import unittest
import json
ROOT=Path(__file__).resolve().parents[2]
DEVICE=ROOT/'native/qemu/hw/cdj/boards/xdj1000mk2'

class TpuTests(unittest.TestCase):
    def check(self,body):
        code=r'''
#include <assert.h>
#include <stdint.h>
#include "sh7724_tpu.h"
#define R(o) (assert(sh7724_tpu_read(&s,o,2,&v)),v)
#define W(o,x) sh7724_tpu_write(&s,o,x,2)
#define A(ns) sh7724_tpu_advance(&s,ns,250000000,3)
int main(void) { Sh7724Tpu s; uint32_t v; (void)v; sh7724_tpu_init(&s);
'''+body+'\n}\n'
        with tempfile.TemporaryDirectory() as d:
            src=Path(d)/'test.c';src.write_text(code);binary=Path(d)/'test'
            r=subprocess.run(['cc','-std=c11','-O2','-Wall','-Werror','-fsanitize=address,undefined','-I',str(DEVICE),str(src),str(DEVICE/'sh7724_tpu.c'),'-o',str(binary)],capture_output=True,text=True)
            self.assertEqual(r.returncode,0,r.stderr)
            r=subprocess.run([str(binary)],capture_output=True,text=True,timeout=5)
            self.assertEqual(r.returncode,0,r.stderr)

    def test_bus_and_superhyway_clocks_are_separate(self):
        p=json.loads((ROOT/'native/hardware_profile.json').read_text())['main_cpu']
        self.assertEqual(p['bus_hz'],[250000000,3])
        self.assertEqual(p['superhyway_hz'],[500000000,3])
        self.assertIn('#define XDJ_TPU_CLOCK_NUM 250000000u',(DEVICE/'hardware_profile.h').read_text())

    def test_reset_register_masks_widths_and_holes(self):
        self.check(r'''
assert(R(0)==0);
for(unsigned n=0;n<4;n++) {
 unsigned b=0x10+0x40*n;
 for(unsigned o=0;o<24;o+=4) assert(R(b+o)==0);
 for(unsigned o=24;o<40;o+=4) assert(R(b+o)==65535);
 assert(W(b,0xffff) && R(b)==255);
 assert(W(b+4,0xffff) && R(b+4)==0x77);
 assert(W(b+8,0xffff) && R(b+8)==7);
 assert(W(b+12,0xffff) && R(b+12)==31);
 assert(W(b+16,0xffff) && R(b+16)==0);
}
assert(W(0,0xffff) && R(0)==15);
assert(!sh7724_tpu_read(&s,0,1,&v) && v==0);
assert(!sh7724_tpu_read(&s,0x28,4,&v) && v==0);
assert(!sh7724_tpu_read(&s,2,2,&v));
assert(!sh7724_tpu_read(&s,0x38,2,&v));
assert(!sh7724_tpu_write(&s,0x1000,1,2));
assert(!sh7724_tpu_write(&s,0x28,1,1) && R(0x28)==65535);
''')

    def test_rational_clock_dividers_stop_and_double_edges(self):
        self.check(r'''
const unsigned div[4]={1,4,16,64};
for(unsigned n=0;n<4;n++) {
 sh7724_tpu_init(&s);assert(W(0x10,n));assert(W(0,1));
 assert(A(12*div[n]-1));assert(R(0x24)==0);
 assert(A(1));assert(R(0x24)==1);
 assert(W(0,0));assert(A(1000000));assert(R(0x24)==1);
 assert(W(0,1));assert(A(12*div[n]));assert(R(0x24)==2);
}
sh7724_tpu_init(&s);assert(W(0x10,0x11));assert(W(0,1));
assert(A(24));assert(R(0x24)==1);
assert(W(0,0));assert(W(0x10,0x10));assert(W(0,1));
assert(!A(1000)); /* Bphi/1 both edges performs no operation */
assert(R(0x24)==1);
assert(W(0,0));assert(W(0x10,4));assert(W(0,1));assert(!A(1000));
assert(!sh7724_tpu_advance(&s,1000,0,3));
assert(!sh7724_tpu_advance(&s,1000,250000000,0));
''')

    def test_compare_clear_overflow_and_read_before_clear(self):
        self.check(r'''
assert(W(0x28,2));assert(W(0x2c,4));assert(W(0x10,0x40));assert(W(0,1));
assert(A(24));assert(R(0x24)==2 && R(0x20)==0);
assert(A(12));assert(R(0x20)==1 && R(0x24)==3);
assert(W(0x20,0));assert(R(0x20)==0);
assert(A(24));assert(R(0x24)==0 && R(0x20)==2);
assert(W(0x20,0xffff));assert(R(0x20)==2);assert(W(0x20,0));
/* Setting a flag via software is prohibited. */
assert(W(0x20,0x1f));assert(R(0x20)==0);
/* Clear only flags observed set by a preceding status read. */
assert(A(36));assert(W(0x20,0));assert(R(0x20)==1);
assert(W(0x20,0));assert(R(0x20)==0);
assert(W(0,0));assert(W(0x10,0));assert(W(0x24,65535));assert(W(0,1));
assert(A(12));assert((R(0x20)&16) && R(0x24)==0);
''')

    def test_stock_pwm_buffer_latches_at_period_and_large_time_is_bounded(self):
        self.check(r'''
assert(W(0x10,0x43));assert(W(0x14,0x52));assert(W(0x18,2));
assert(W(0x2c,3973));assert(W(0x30,100));assert(W(0,1));
assert(sh7724_tpu_buffer_pending(&s));
assert(A(3973ULL*768));assert(R(0x28)==65535);
assert(A(768));assert(R(0x24)==0 && R(0x28)==100 && R(0x30)==100);
assert(!sh7724_tpu_buffer_pending(&s));
assert(W(0x30,200));assert(R(0x28)==100);assert(A(3974ULL*768));assert(R(0x28)==200);
/* No callback or iteration per PWM edge even after a huge time jump. */
assert(A(UINT64_MAX));assert(R(0x24)<3974);assert((R(0x20)&3)==3);
/* Buffer-on-match updates A and B and preserves compare flags. */
sh7724_tpu_init(&s);assert(W(0x14,0x30));assert(W(0x28,2));assert(W(0x2c,4));
assert(W(0x30,6));assert(W(0x34,8));assert(W(0,1));assert(A(60));
assert(R(0x28)==6 && R(0x2c)==8 && (R(0x20)&3)==3);
''')

    def test_unreachable_buffer_and_unsupported_configuration_are_bounded(self):
        self.check(r'''
assert(W(0x10,0x40));assert(W(0x14,0x10));assert(W(0x28,10));assert(W(0x2c,4));
assert(W(0x30,1));assert(W(0,1));assert(A(UINT64_MAX));assert(R(0x28)==10);
assert(W(0,0));assert(W(0x10,0x60));assert(W(0,1));assert(!A(100));
assert(W(0,0));assert(W(0x10,0xa0));assert(W(0,1));assert(!A(100));
assert(W(0,0));assert(W(0x10,0));assert(W(0x14,1));assert(W(0,1));assert(!A(100));
''')

    def test_buffer_match_after_wrap_and_after_periodic_clear(self):
        self.check(r'''
assert(W(0x14,0x10));assert(W(0x28,2));assert(W(0x30,6));assert(W(0x24,4));assert(W(0,1));
assert(A(65535ULL*12));assert(R(0x28)==6);
sh7724_tpu_init(&s);assert(W(0x10,0x40));assert(W(0x14,0x10));
assert(W(0x28,2));assert(W(0x2c,10));assert(W(0x30,6));assert(W(0x24,4));assert(W(0,1));
assert(A(10*12));assert(R(0x28)==6 && R(0x24)==3);
''')

    def test_arithmetic_matches_independent_single_tick_reference(self):
        self.check(r'''
uint32_t seed=0x12345678;
#define RAND() (seed=seed*1664525u+1013904223u)
const unsigned clears[6]={0,1,2,4,5,6};
for(unsigned trial=0;trial<2048;trial++) {
 sh7724_tpu_init(&s);
 unsigned ch=trial%4;Sh7724TpuChannel *c=&s.channel[ch];
 c->tcr=clears[(RAND()>>16)%6]<<5;
 c->tmdr=((RAND()>>16)&0x70)|((trial&1)?2:0);
 for(unsigned i=0;i<4;i++) c->tgr[i]=(RAND()>>16)&63;
 c->count=(RAND()>>16)&65535;s.start=1u<<ch;
 if(!sh7724_tpu_configured(&s)) continue;
 Sh7724TpuChannel expected=*c;
 unsigned ticks=(RAND()>>16)&2047;
 for(unsigned n=0;n<ticks;n++) {
  bool matches[4];
  for(unsigned i=0;i<4;i++) {
   bool buffer=i>=2 && (expected.tmdr&(0x10u<<(i-2)));
   matches[i]=!buffer && expected.count==expected.tgr[i];
   if(matches[i]) expected.tsr|=1u<<i;
  }
  unsigned clear=expected.tcr>>5;
  bool clearing=(clear==1&&matches[0])||(clear==2&&matches[1])||
                (clear==5&&matches[2])||(clear==6&&matches[3]);
  if(expected.count==65535) expected.tsr|=16;
  expected.count=clearing?0:expected.count+1;
  for(unsigned i=0;i<2;i++)
   if((expected.tmdr&(0x10u<<i)) && ((expected.tmdr&0x40)?clearing:matches[i]))
    expected.tgr[i]=expected.tgr[i+2];
 }
 assert(A((uint64_t)ticks*12));
 assert(c->count==expected.count && c->tsr==expected.tsr);
 for(unsigned i=0;i<4;i++) assert(c->tgr[i]==expected.tgr[i]);
}
''')
