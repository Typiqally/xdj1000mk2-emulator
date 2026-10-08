from native_support import core_library
"""Whole-loop code preserves short-trip loads and pipeline state."""
import os
import shlex
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
DSP = ROOT/'native/qemu/hw/cdj/c6x'
LIB = core_library()


@unittest.skipUnless(LIB.exists(), 'requires the built DSP library')
class WholeLoopTests(unittest.TestCase):
    def test_short_trip_memory_loop_matches_interpreter_at_every_budget(self):
        # Original C674x memcpy loop: SPLOOP 2; LDNDW; four pipeline slots; STNDW.
        # Only instruction bytes are retained. Data/registers/counts are synthetic.
        code = bytes.fromhex('37569482b42a8884553714a8d80f2000753794c3a0e103c001808300763794c8a'
                             '43790030040000000000000014003007437940362a38c000000000000000000')
        self.check_loop(code, 0x11804720, 0x11804738,
            '173 2 6 -1 0 1000000 | 0x11804740 | | | | | 0x11804750',
            'c->cr[CR_ILC]=count; c->reg[4]=0x80000080; c->reg[5]=0x80000080; c->reg[36]=0x80000000;', 40)

    def test_delayed_fetch_loop_resumes_after_the_epilog(self):
        # SPLOOPD 1, MVC B5,ILC; signed samples -> float -> scaled samples.
        # SPKERNEL's encoded delay exceeds the 13-cycle epilog. Stop emitting
        # buffered instructions when program fetch resumes, as the CPU does.
        code = bytes.fromhex('004000007a001d02518f670cef dadc3e581914020040000000ce9001200093e1'
                             '6e2c6e0c675e350c62a38c0028e486016e0cf191a23d98025ac09704040060e2'.replace(' ',''))
        self.check_loop(code, 0xc003d140, 0xc003d14a,
            '174 1 14 -1 0 1000000 | 0xc003d14e | | | | | 0xc003d150 | | | | 0xc003d158 | | | | 0xc003d166',
            'c->reg[37]=count * 8; c->reg[5]=0x80000000; c->reg[36]=0x80000200; c->reg[6]=0x3f800000; c->reg[35]=0xc003d180;', 150)

    def check_loop(self, code, base, root, kernel, registers, max_budget):
        with tempfile.TemporaryDirectory() as d:
            folder = Path(d)
            (folder/'ram_0.bin').write_bytes(code+bytes(64))
            (folder/'profile.txt').write_text(
                f'idle_head 0\nmap {base:#x} 128 0\nkern {root:#x} {kernel}\n')
            env = {k:v for k,v in os.environ.items() if not k.startswith('C66X_')}
            result = subprocess.run([sys.executable,str(DSP/'tools/c14_jitgen.py'),str(folder),
                str(folder/'m'),'--lib',str(LIB),'--roots','0','--kernels','0','--loops','1',
                '--lmin','0','--loop-only',hex(root),'--round-batch'],env=env,capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stderr)
            runner = folder/'test.c'
            runner.write_text(r'''
#include <assert.h>
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "c66x_priv.h"
static const unsigned char bytes_[] = {'''+','.join(str(x) for x in code)+r'''};
static c66x_core *make(unsigned count, uint8_t code[128], uint8_t data[1024]) {
    c66x_core *c = c66x_new(NULL);
    memset(code,0,128); memcpy(code,bytes_,sizeof bytes_);
    for(unsigned i=0;i<1024;i++) data[i]=i;
    c66x_map_ram(c,'''+hex(base)+r''',128,code); c66x_map_ram(c,0x80000000,1024,data);
    c66x_reset(c,'''+hex(root)+r''');
    '''+registers+r'''
    return c;
}
int main(int argc,char **argv) {
    void *module=dlopen(argv[1],RTLD_NOW); assert(module);
    const c66x_jit_module *m=dlsym(module,"c66x_jit_exports"); assert(m && m->nloops==1);
    for(unsigned count=1;count<=8;count++) for(unsigned budget=1;budget<='''+str(max_budget)+r''';budget++) {
        uint8_t ca[128],cb[128],da[1024],db[1024];
        c66x_core *a=make(count,ca,da),*b=make(count,cb,db);
        uint64_t at=a->cycle,done=0;
        m->loops[0].fn(a,budget);
        if(a->cycle>at) {
            assert(c66x_step(b,a->cycle-at,&done)==C66X_STOP_BUDGET);
            if(a->jit_committed) commit_writes(b);
            if(c66x_state_hash(a)!=c66x_state_hash(b) || memcmp(da,db,1024)) {
                fprintf(stderr,"count %u budget %u cycles %llu pc %08x/%08x ilc %u/%u\n",count,budget,(unsigned long long)a->cycle,a->pc,b->pc,a->cr[CR_ILC],b->cr[CR_ILC]);
                for(unsigned i=0;i<64;i++) if(a->reg[i]!=b->reg[i])fprintf(stderr,"r%u %08x/%08x\n",i,a->reg[i],b->reg[i]);
                return 1;
            }
        }
        c66x_free(a);c66x_free(b);
    }
    dlclose(module);return 0;
}
''')
            build = subprocess.run(['cc','-O2','-std=gnu11','-I',str(DSP),str(runner),str(LIB),
                                    '-o',str(folder/'test')]+shlex.split(subprocess.check_output(['pkg-config','--cflags','gmodule-2.0'],text=True)),capture_output=True,text=True)
            self.assertEqual(build.returncode,0,build.stderr)
            run = subprocess.run([str(folder/'test'),str(folder/'m.so')],env=env,capture_output=True,text=True)
            self.assertEqual(run.returncode,0,run.stderr)
