"""Documented DA810 PLL registers drive clocks without a firmware profile."""
from pathlib import Path
import subprocess
import tempfile
import unittest
ROOT=Path(__file__).resolve().parents[2]
DEVICE=ROOT/'native/qemu/hw/cdj/boards/xdj1000mk2'

class PllTests(unittest.TestCase):
    def test_reset_bypass_native_setup_and_divider_commit(self):
        code=r'''
#include <assert.h>
#include "da810_pll.h"
int main(void) {
    Da810Pll p; da810_pll_init(&p,22579200);
    assert(da810_pll_read(&p,0,4)==0x44813c00);
    assert(da810_pll_read(&p,0xe4,4)==1);
    assert(da810_pll_read(&p,0x110,4)==19);
    assert(da810_pll_read(&p,0x128,4)==0x8001);
    assert(da810_pll_read(&p,0x13c,4)==4);
    assert(da810_pll_sysclk(&p,1)==22579200);
    assert(da810_pll_auxclk(&p)==22579200);
    da810_pll_write(&p,0x148,2,4);assert(da810_pll_auxclk(&p)==0);
    da810_pll_write(&p,0x148,3,4);assert(da810_pll_auxclk(&p)==22579200);
    /* Firmware's documented sequence: bypass, reset, multiplier/dividers, GO,
       release reset, enable. No stock update hash or instruction address. */
    da810_pll_write(&p,0x100,0x50,4);
    da810_pll_write(&p,0x110,19,4);
    da810_pll_write(&p,0x128,0x8000,4);
    da810_pll_write(&p,0x120,0x8002,4);
    da810_pll_write(&p,0x138,1,4);
    da810_pll_write(&p,0x100,0x58,4);
    da810_pll_write(&p,0x100,0x59,4);
    assert(da810_pll_read(&p,0x100,4)==0x59);
    assert(da810_pll_sysclk(&p,1)==451584000);
    assert(da810_pll_sysclk(&p,2)==225792000);
    da810_pll_write(&p,0x118,0x8001,4);
    assert(da810_pll_sysclk(&p,1)==451584000); /* programmed vs active divider */
    assert(da810_pll_read(&p,0x144,4)&1);
    da810_pll_write(&p,0x138,1,4);
    assert(da810_pll_sysclk(&p,1)==225792000);
    assert(da810_pll_read(&p,0x144,4)==0);
    da810_pll_write(&p,0x110,9,1); /* byte lane preserves register width */
    assert(da810_pll_sysclk(&p,1)==112896000);
    da810_pll_write(&p,0x100,0x58,4);
    assert(da810_pll_sysclk(&p,1)==11289600); /* bypass still uses SYSCLK divider */
    da810_pll_write(&p,0x118,0,4);da810_pll_write(&p,0x138,1,4);
    assert(da810_pll_sysclk(&p,1)==0);assert(!(da810_pll_read(&p,0x150,4)&1));
    da810_pll_write(&p,0,0,4);da810_pll_write(&p,0x13c,0,4);
    assert(da810_pll_read(&p,0,4)==0x44813c00);assert(da810_pll_read(&p,0x13c,4)==4);
    assert(da810_pll_read(&p,0x11a,2)==0);
    return 0;
}
'''
        with tempfile.TemporaryDirectory() as d:
            src=Path(d)/'test.c';src.write_text(code);binary=Path(d)/'test'
            build=subprocess.run(['cc','-std=c11','-Wall','-Werror','-I',str(DEVICE),str(src),
                                  str(DEVICE/'da810_pll.c'),'-o',str(binary)],capture_output=True,text=True)
            self.assertEqual(build.returncode,0,build.stderr)
            run=subprocess.run([str(binary)],capture_output=True,text=True)
            self.assertEqual(run.returncode,0,run.stderr)
