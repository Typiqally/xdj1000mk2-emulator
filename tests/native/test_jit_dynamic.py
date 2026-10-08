"""Indirect clean branches stay compiled with byte guards and exact cycle bounds."""
from native_support import core_library
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
CORE = ROOT / 'native/qemu/hw/cdj/c6x'
LIB = core_library()


class DynamicTests(unittest.TestCase):
    def test_indirect_clean_target_stays_compiled_through_the_budget(self):
        self.check(False)

    def test_unknown_indirect_target_returns_exact_pending_state(self):
        self.check(True)

    def check(self, unknown):
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory)
            raw = bytearray(1024)
            raw[0:4] = raw[32:36] = bytes.fromhex('62030c00')  # B B3, five delay slots
            (folder / 'ram_0.bin').write_bytes(raw)
            (folder / 'profile.txt').write_text(
                'map 0x11800000 0x400 0\nprof 0x11800000 1000000 1000000\n'
                'prof 0x11800020 1000000 1000000\n')
            env = {k: v for k, v in os.environ.items() if not k.startswith('C66X_')}
            command = [sys.executable, str(CORE / 'tools/c14_jitgen.py'), str(folder), str(folder / 'm'),
                       '--lib', str(LIB), '--min', '0', '--roots', '2', '--kernels', '0',
                       '--cold', '0', '--nodes', '64', '--total', '256', '--fn-nodes', '32']
            generation = subprocess.run(command, env=env, capture_output=True, text=True)
            self.assertEqual(generation.returncode, 0, generation.stderr)
            source = folder / 'test.c'
            source.write_text(r'''
#include <assert.h>
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "c66x_priv.h"
static c66x_core *create(uint8_t *ram, uint32_t target) {
    memset(ram,0,1024); memcpy(ram,"\x62\x03\x0c\x00",4);
    memcpy(ram+32,"\x62\x03\x0c\x00",4);
    c66x_core *c=c66x_new(NULL); c66x_map_ram(c,0x11800000,1024,ram);
    c66x_reset(c,0x11800000); c->reg[35]=target; return c;
}
int main(int argc,char **argv) {
    void *module=dlopen(argv[1],RTLD_NOW); assert(module);
    const c66x_jit_module *m=dlsym(module,"c66x_jit_exports"); assert(m);
    unsigned root=0; while(root<m->nregions0 && m->regions0[root].pc!=0x11800000) root++;
    assert(root<m->nregions0);
    for(unsigned budget=1;budget<=64;budget++) {
        uint8_t ar[1024],br[1024]; uint64_t done;
        uint32_t target=''' + ('0x11800300' if unknown else '0x11800020') + r''';
        c66x_core *a=create(ar,target),*b=create(br,target);
        m->regions0[root].fn(a,budget);
        assert(c66x_step(b,a->cycle,&done)==C66X_STOP_BUDGET);
        if(a->jit_committed) commit_writes(b);
        assert(c66x_state_hash(a)==c66x_state_hash(b));
        if(''' + ('0' if unknown else '1') + r''' && a->cycle!=budget) {
            fprintf(stderr,"indirect target returned at cycle %llu before budget %u\n",
                    (unsigned long long)a->cycle,budget); return 2;
        }
        if(''' + ('1' if unknown else '0') + r''' && budget>6)
            assert(a->cycle==6 && a->pc==target);
        c66x_free(a); c66x_free(b);
    }
    /* The local target is covered by the same function byte guard. A code
     * notification must stop a verified function before it uses stale bytes. */
    if (''' + ('0' if unknown else '1') + r''') {
        uint8_t ar[1024],br[1024]; uint64_t done;
        setenv("C66X_JIT",argv[1],1);
        c66x_core *a=create(ar,0x11800020);
        unsetenv("C66X_JIT");
        c66x_core *b=create(br,0x11800020);
        assert(c66x_step(a,64,&done)==C66X_STOP_BUDGET);
        assert(c66x_step(b,64,&done)==C66X_STOP_BUDGET);
        assert(c66x_state_hash(a)==c66x_state_hash(b));
        uint64_t gen=a->code_gen;
        memset(ar+32,0,4); memset(br+32,0,4);
        c66x_invalidate(a,0x11800020,4); c66x_invalidate(b,0x11800020,4);
        assert(a->code_gen>gen);
        assert(c66x_step(a,64,&done)==C66X_STOP_BUDGET);
        assert(c66x_step(b,64,&done)==C66X_STOP_BUDGET);
        assert(c66x_state_hash(a)==c66x_state_hash(b));
        c66x_free(a); c66x_free(b);
    }
    dlclose(module);
}
''')
            flags = subprocess.check_output(['pkg-config', '--cflags', '--libs', 'gmodule-2.0'], text=True).split()
            binary = folder / 'test'
            build = subprocess.run(['cc', '-O2', '-std=gnu11', '-I', str(CORE), str(source),
                                    str(LIB), *flags, '-o', str(binary)], capture_output=True, text=True)
            self.assertEqual(build.returncode, 0, build.stderr)
            result = subprocess.run([str(binary), str(folder / 'm.so')], env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == '__main__':
    unittest.main()
