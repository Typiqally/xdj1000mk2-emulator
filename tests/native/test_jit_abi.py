"""Current generated modules use core state and reject incompatible profile ABIs."""
import importlib.util
from pathlib import Path
import re
import subprocess
import tempfile
import unittest
import sys
from native_support import core_library

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
        self.assertGreaterEqual(abi,15)
        self.assertIn('fprintf(f, "abi %d\\n", C66X_JIT_ABI);',(DSP/'c66x_jit.c').read_text())
        tail=header[header.index('uint8_t c674x_only;'):]
        for field in ('jit_hop_fn','jit_hop_entry','jit_round_mode'):
            self.assertIn(field,tail,'new core state must be appended, never inserted')

    def test_prelude_compiles_and_keeps_hop_and_rounding_state_in_the_core(self):
        module=generator()
        body=r'''
#include <assert.h>
int main(void) {
    c66x_core *c = calloc(1, sizeof(*c));
    fesetround(FE_UPWARD); c->jit_round_mode = fegetround(); jit_round_enter();
    assert(c->jit_round_mode == FE_UPWARD);
    jit_hop_fn = 7; jit_hop_entry = 9;
#undef jit_hop_fn
#undef jit_hop_entry
    assert(c->jit_hop_fn == 7 && c->jit_hop_entry == 9 && c->jit_round_mode == FE_UPWARD);
    jit_round_sync(); assert(fegetround() == FE_TONEAREST);
    free(c);
    return 0;
}
'''
        with tempfile.TemporaryDirectory() as d:
            code=module.make_prelude()+body
            src=Path(d)/'prelude.c';src.write_text(code);binary=Path(d)/'prelude'
            build=subprocess.run(['cc','-std=gnu11','-O2','-frounding-math','-ffp-contract=off','-I',str(DSP),str(src),'-o',str(binary)],capture_output=True,text=True)
            self.assertEqual(build.returncode,0,build.stderr[-2000:])
            result=subprocess.run([str(binary)],capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stderr)

    def test_generator_refuses_a_profile_for_an_older_core_layout(self):
        header=(DSP/'c66x_core_int.h').read_text()
        abi=int(re.search(r'#define C66X_JIT_ABI (\d+)',header).group(1))
        with tempfile.TemporaryDirectory() as d:
            folder=Path(d)
            (folder/'ram_0.bin').write_bytes(bytes(1024))
            (folder/'profile.txt').write_text(f'abi {abi-1}\nmap 0x11800000 0x400 0\n')
            run=subprocess.run([sys.executable,str(DSP/'tools/c14_jitgen.py'),str(folder),str(folder/'m'),
                                '--lib',str(core_library()),'--no-cc'],capture_output=True,text=True)
            self.assertNotEqual(run.returncode,0)
            self.assertIn(f'module ABI {abi-1}',run.stderr)
            self.assertFalse((folder/'m.main.c').exists())


if __name__=='__main__':unittest.main()
