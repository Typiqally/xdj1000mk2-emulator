"""Mac DSP rounding shim preserves IEEE results and unrelated FP state."""
from pathlib import Path
import importlib.util
import subprocess
import tempfile
import unittest
from native_support import core_library
ROOT = Path(__file__).resolve().parents[2]
DSP = ROOT / 'native/qemu/hw/cdj/c6x'
class RoundingTests(unittest.TestCase):
    def test_compiled_rounding_batches_match_mixed_modes_and_restore_callbacks(self):
        spec = importlib.util.spec_from_file_location('jitgen', DSP / 'tools/c14_jitgen.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        library = core_library()
        code = module.make_prelude() + r'''
#include <assert.h>
static uint32_t callback(void *opaque, uint32_t a, unsigned n) {
    assert(fegetround() == FE_TONEAREST);
    return 0x12345678;
}
int main(void) {
    c66x_bus bus = { .read = callback };
    c66x_core *c = c66x_new(&bus);
    uint32_t x[] = {0, 0x80000000, 0x3f800000, 0xbf800000, 0x33800000,
      0xb3800000, 0x3f800001, 0x7f7fffff, 0x00800000, 1, 0x7f800000, 0x7fc12345};
    for (int op = 0; op < 3; op++) for (int a = 0; a < 12; a++)
    for (int b = 0; b < 12; b++) for (int mode = 3; mode >= 0; mode--) {
        char kind = op == 0 ? '+' : op == 1 ? '-' : '*';
        fesetround(jit_fe_mode[mode]); feclearexcept(FE_ALL_EXCEPT);
        volatile float p = jit_u2f(x[a]), q = jit_u2f(x[b]);
        float want = op == 0 ? p + q : op == 1 ? p - q : p * q;
        uint32_t bits = jit_f2u(want);
        int flags = fetestexcept(FE_ALL_EXCEPT);
        /* Keep the preceding operation's host mode; the guest mode must
         * still be selected, including an explicit return to nearest. */
        fesetround(jit_fe_mode[(mode + 1) & 3]); feclearexcept(FE_ALL_EXCEPT);
        c->jit_round_mode = fegetround(); /* the c66x_step entry contract */
        jit_round_enter();
        c->cr[op == 2 ? CR_FMCR : CR_FADCR] = (unsigned)mode << 9;
        c66x_round_sync(c);
        assert(jit_fop(c, op == 2 ? 6 : 0, 1, x[a], x[b], kind) == bits);
        assert(fetestexcept(FE_ALL_EXCEPT) == flags);
    }
    fesetround(FE_DOWNWARD);
    c->jit_round_mode = fegetround();
    jit_round_enter();
    assert(jit_f_to_i32(1.5, 0) == 2);
    fesetround(FE_UPWARD);
    c->jit_round_mode = fegetround();
    jit_round_enter();
    /* The mode in use belongs to the core being run, not to the thread: no
     * thread-local lookup on the floating-point path. */
    assert(c->jit_round_mode == FE_UPWARD);
    assert((char *)&c->jit_round_mode > (char *)c && (char *)&c->jit_round_mode < (char *)(c + 1));
    c->ram[0].size = 0;
    assert(jit_load(c, 0x1234, 4) == 0x12345678);
    jit_round_sync(); assert(fegetround() == FE_TONEAREST);
    c66x_free(c);
}
'''
        with tempfile.TemporaryDirectory() as d:
            src = Path(d) / 'batch.c'; src.write_text(code)
            binary = Path(d) / 'batch'
            subprocess.run(['cc', '-std=gnu11', '-O2', '-frounding-math',
                            '-ffp-contract=off', '-I', str(DSP), str(src),
                            str(library), '-o', str(binary)], check=True, capture_output=True)
            result = subprocess.run([str(binary)], capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr.decode())

    def test_all_modes_results_and_exception_flags_match_libc(self):
        code = r'''
#include <assert.h>
#include <fenv.h>
#include <stdint.h>
#include <string.h>
#include "c66x_fenv.h"
static inline uint32_t calc(uint32_t x, uint32_t y, unsigned op) {
    float a,b,r; memcpy(&a,&x,4); memcpy(&b,&y,4);
    r=op==0?a+b:op==1?a-b:a*b;
    uint32_t out; memcpy(&out,&r,4); return out;
}
int main(void) {
    int modes[]={FE_TONEAREST,FE_TOWARDZERO,FE_UPWARD,FE_DOWNWARD};
    uint32_t values[]={0,0x80000000,0x3f800000,0xbf800000,0x33800000,
      0xb3800000,0x3f800001,0x7f7fffff,0x00800000,1,0x7f800000,0x7fc12345};
    for(unsigned m=0;m<4;m++)for(unsigned a=0;a<12;a++)for(unsigned b=0;b<12;b++)
    for(unsigned op=0;op<3;op++) {
        fesetround(modes[m]); feclearexcept(FE_ALL_EXCEPT);
        uint32_t want=calc(values[a],values[b],op); int flags=fetestexcept(FE_ALL_EXCEPT);
        fesetround(FE_TONEAREST); feclearexcept(FE_ALL_EXCEPT);
        assert(c66x_fesetround(modes[m])==0); assert(fegetround()==modes[m]);
        assert(calc(values[a],values[b],op)==want);
        assert(fetestexcept(FE_ALL_EXCEPT)==flags);
    }
    feraiseexcept(FE_DIVBYZERO);
    assert(c66x_fesetround(FE_TONEAREST)==0);
    assert(fetestexcept(FE_DIVBYZERO));
    return 0;
}
'''
        with tempfile.TemporaryDirectory() as d:
            src=Path(d)/'round.c'; src.write_text(code); binary=Path(d)/'round'
            subprocess.run(['cc','-std=c11','-O3','-frounding-math','-ffp-contract=off',
                            '-I',str(DSP),str(src),'-o',str(binary)],check=True,capture_output=True)
            subprocess.run([str(binary)],check=True,capture_output=True)
