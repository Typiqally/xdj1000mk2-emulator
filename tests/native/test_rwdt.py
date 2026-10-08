"""SH7724 RWDT contracts: Renesas R01UH0174JJ0200 chapters 17 and 19."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
DEVICE = ROOT/'native/qemu/hw/cdj/boards/xdj1000mk2'


class RwdtTests(unittest.TestCase):
    def check(self, body):
        code = r'''
#include <assert.h>
#include <stdint.h>
#include "sh7724_rwdt.h"
#define R(o) (assert(sh7724_rwdt_read(&s,o,1,&v)),v)
#define W(o,x) sh7724_rwdt_write(&s,o,x,2)
#define A(ns) sh7724_rwdt_advance(&s,ns,100000000,3072)
int main(void) { Sh7724Rwdt s; uint32_t v; (void)v; sh7724_rwdt_init(&s);
''' + body + '\n}\n'
        with tempfile.TemporaryDirectory() as d:
            src = Path(d)/'test.c'; src.write_text(code)
            binary = Path(d)/'test'
            build = subprocess.run(['cc','-std=c11','-Wall','-Werror','-I',str(DEVICE),
                                    str(src),str(DEVICE/'sh7724_rwdt.c'),'-o',str(binary)],
                                   capture_output=True,text=True)
            self.assertEqual(build.returncode,0,build.stderr)
            result = subprocess.run([str(binary)],capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stderr)

    def test_power_on_keys_widths_and_reserved_bits(self):
        self.check(r'''
    assert(R(0)==0 && R(4)==0x87);
    assert(W(4,0x5a07) && R(4)==0x87); /* wrong key ignored */
    assert(sh7724_rwdt_write(&s,4,0x07,1) && R(4)==0x87);
    assert(sh7724_rwdt_write(&s,4,0xa507,4) && R(4)==0x87);
    assert(W(4,0xa507) && R(4)==7);
    assert(W(0,0xa5ff) && R(0)==0);
    assert(!W(0,0x5a42) && R(0)==0x42); /* write-sync timing unqualified */
    assert(W(4,0xa57f) && R(4)==0x17); /* reserved/WRFLG/SRSTF cannot be set */
    assert(W(4,0xa50f) && R(4)==7); /* WOVF R/W, SRSTF clear-only */
    s.csr |= 8;
    assert(W(4,0xa50f) && R(4)==15);
    assert(W(4,0xa507) && R(4)==7);
    assert(!sh7724_rwdt_read(&s,4,2,&v));
    assert(!sh7724_rwdt_read(&s,1,1,&v));
    assert(!sh7724_rwdt_write(&s,8,0,2));
''')

    def test_disable_retains_count_and_fractional_time_does_not_drift(self):
        self.check(r'''
    assert(W(4,0xa507)); /* disable before changing divider */
    assert(W(4,0xa500));
    assert(W(4,0xa580));
    for(unsigned n=0;n<3071;n++) assert(!A(10));
    assert(R(0)==0);
    assert(!A(10) && R(0)==1); /* one EXTAL/1024 edge */
    assert(W(4,0xa500));
    assert(!A(10000000000ULL) && R(0)==1);
    assert(sh7724_rwdt_until_overflow(&s,100000000,3072)==-1);
    assert(W(4,0xa580));
    assert(!A(30720) && R(0)==2);
    assert(!sh7724_rwdt_advance(&s,1000000,0,3072) && R(0)==2);
''')

    def test_all_dividers_exact_overflow_and_feed(self):
        self.check(r'''
    const unsigned div[8]={1,4,16,32,64,128,1024,4096};
    for(unsigned i=0;i<8;i++) {
        sh7724_rwdt_init(&s);
        assert(W(4,0xa507));
        assert(W(4,0xa500|i));
        assert(!W(0,0x5aff));
        assert(W(4,0xa580|i));
        uint64_t period=(uint64_t)div[i]*30720;
        assert(sh7724_rwdt_until_overflow(&s,100000000,3072)==(int64_t)period);
        assert(!A(period-1) && R(0)==255);
        assert(A(1) && R(0)==0 && (R(4)&0x10) && s.pending_reset);
        assert(!A(period) && R(0)==0); /* reset request only once */
        assert(sh7724_rwdt_until_overflow(&s,100000000,3072)==-1);
    }
    sh7724_rwdt_init(&s);
    assert(W(4,0xa507));assert(!W(0,0x5aff));assert(W(4,0xa587));
    assert(!A(100000000));
    assert(!W(0,0x5a00)); /* feed clears counter before overflow */
    assert(!A(1000000000) && R(0)==8);
''')

    def test_active_divider_change_and_missing_clock_remain_unqualified(self):
        self.check(r'''
    assert(!W(4,0xa586)); /* changing divider while running is prohibited */
    assert(sh7724_rwdt_until_overflow(&s,0,3072)==-1);
    assert(!sh7724_rwdt_advance(&s,UINT64_MAX,0,3072));
    assert(sh7724_rwdt_until_overflow(&s,100000000,0)==-1);
    assert(!sh7724_rwdt_advance(&s,UINT64_MAX,100000000,0));
    assert(A(UINT64_MAX) && s.pending_reset); /* no arithmetic wrap hides overflow */
    sh7724_rwdt_init(&s);
    assert(sh7724_rwdt_until_overflow(&s,1,UINT64_MAX)==INT64_MAX);
''')

    def test_divider_change_does_not_reuse_fraction_from_a_different_period(self):
        self.check(r'''
    assert(W(4,0xa507));assert(!W(0,0x5aff));assert(W(4,0xa587));
    assert(!A(100000000) && R(0)==255);
    assert(W(4,0xa507));assert(W(4,0xa500));assert(W(4,0xa580));
    assert(sh7724_rwdt_until_overflow(&s,100000000,3072)==30720);
    assert(!A(30719) && R(0)==255);
    assert(A(1));
''')
