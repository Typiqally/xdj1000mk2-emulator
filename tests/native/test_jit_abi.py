"""One generator serves cores built before and after generated code's state moved into the core."""
import importlib.util
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[2]
DSP=ROOT/'native/qemu/hw/cdj/c6x'


def generator():
    spec=importlib.util.spec_from_file_location('jitgen',DSP/'tools/c14_jitgen.py')
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    return module


class JitAbiTests(unittest.TestCase):
    def test_core_announces_its_module_abi_and_the_generator_follows_it(self):
        header=(DSP/'c66x_core_int.h').read_text()
        abi=int(re.search(r'#define C66X_JIT_ABI (\d+)',header).group(1))
        module=generator()
        # Appended fields a core of an earlier ABI does not have: its modules must not touch them.
        self.assertEqual(module.CORE_STATE_ABI,13)
        self.assertGreaterEqual(abi,module.CORE_STATE_ABI)
        self.assertIn('fprintf(f, "abi %d\\n", C66X_JIT_ABI);',(DSP/'c66x_jit.c').read_text())
        tail=header[header.index('uint8_t c674x_only;'):]
        for field in ('jit_hop_fn','jit_hop_entry','jit_round_mode'):
            self.assertIn(field,tail,'new core state must be appended, never inserted')

    def test_both_preludes_compile_and_keep_their_state_where_the_core_expects_it(self):
        module=generator()
        body=r'''
#include <assert.h>
int main(void) {
    c66x_core *c = calloc(1, sizeof(*c));
    fesetround(FE_UPWARD); jit_round_enter();
    assert(jit_round_mode == FE_UPWARD);
    jit_hop_fn = 7; jit_hop_entry = 9;
#ifdef JIT_CORE_STATE
    assert(c->jit_hop_fn == 7 && c->jit_hop_entry == 9 && c->jit_round_mode == FE_UPWARD);
#else
    /* A core of the earlier ABI ends before these fields: nothing may be written there. */
    assert(!c->jit_hop_fn && !c->jit_hop_entry && !c->jit_round_mode);
#endif
    jit_round_sync(); assert(fegetround() == FE_TONEAREST);
    return 0;
}
'''
        legacy_state='_Thread_local int jit_hop_fn = -1;\n_Thread_local unsigned jit_hop_entry;\n_Thread_local int jit_round_mode = FE_TONEAREST;\n'
        for core_state in (True,False):
            with self.subTest(core_state=core_state),tempfile.TemporaryDirectory() as d:
                prelude=module.make_prelude(round_batch=True,core_state=core_state)
                self.assertEqual('#define JIT_CORE_STATE 1' in prelude,core_state)
                # undo the accessor macros only for the direct field checks
                code=prelude+('' if core_state else legacy_state)+body.replace('c->jit_hop_fn','(*(int *)&c->jit_hop_fn_)').replace('c->jit_hop_entry','(*(unsigned *)&c->jit_hop_entry_)').replace('c->jit_round_mode','(*(int *)&c->jit_round_mode_)')
                code=code.replace('#include "c66x_core_int.h"','#define jit_hop_fn jit_hop_fn_\n#define jit_hop_entry jit_hop_entry_\n#define jit_round_mode jit_round_mode_\n#include "c66x_core_int.h"\n#undef jit_hop_fn\n#undef jit_hop_entry\n#undef jit_round_mode',1)
                if core_state:
                    code=code.replace('#define jit_hop_fn (c->jit_hop_fn)','#define jit_hop_fn (c->jit_hop_fn_)').replace('#define jit_hop_entry (c->jit_hop_entry)','#define jit_hop_entry (c->jit_hop_entry_)').replace('#define jit_round_mode (c->jit_round_mode)','#define jit_round_mode (c->jit_round_mode_)')
                src=Path(d)/'prelude.c';src.write_text(code);binary=Path(d)/'prelude'
                build=subprocess.run(['cc','-std=gnu11','-O2','-frounding-math','-ffp-contract=off','-I',str(DSP),str(src),'-o',str(binary)],capture_output=True,text=True)
                self.assertEqual(build.returncode,0,build.stderr[-2000:])
                result=subprocess.run([str(binary)],capture_output=True,text=True)
                self.assertEqual(result.returncode,0,result.stderr)


if __name__=='__main__':unittest.main()
