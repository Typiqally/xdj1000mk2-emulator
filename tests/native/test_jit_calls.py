from native_support import core_library
"""Compile native call/return instructions through their delay slots."""
import os,shlex,subprocess,sys,tempfile,unittest
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
DSP=ROOT/'native/qemu/hw/cdj/c6x';LIB=core_library()

@unittest.skipUnless(LIB.exists(),'requires the built DSP library')
class CallTests(unittest.TestCase):
    def test_known_call_link_survives_delay_slots_and_return(self):
        self.check_calls(False)

    def test_delayed_load_of_a_new_link_invalidates_the_known_return(self):
        self.check_calls(True)

    def check_calls(self, overwrite):
        with tempfile.TemporaryDirectory() as d:
            p=Path(d);raw=bytearray(0x6000)
            # Original full-width C674x CALLP and B B3 instruction encodings;
            # addresses are their relative operands, all other code is NOP.
            raw[0x5d30:0x5d34]=bytes.fromhex('12aaf51f')
            raw[0xa70:0xa74]=bytes.fromhex('62030c00')
            if overwrite:
                raw[0xa70:0xa74]=bytes.fromhex('e6d2bc01')  # LDW *+B15(24),B3
                raw[0xa80:0xa84]=bytes.fromhex('62030c00')
                raw[0x5008:0x500c]=(0xc004ed38).to_bytes(4,'little')
            (p/'ram_0.bin').write_bytes(raw)
            (p/'profile.txt').write_text('idle_head 0\nmap 0xc0049000 0x6000 0\nprof 0xc004ed30 1000000 1000000\n')
            env={k:v for k,v in os.environ.items() if not k.startswith('C66X_')}
            gen=subprocess.run([sys.executable,str(DSP/'tools/c14_jitgen.py'),str(p),str(p/'m'),
                '--lib',str(LIB),'--roots','1','--min','0','--kernels','0','--cold','0','--nodes','128',
                '--fn-nodes','128','--total','256'],env=env,capture_output=True,text=True)
            self.assertEqual(gen.returncode,0,gen.stderr)
            source=p/'test.c';source.write_text(r'''
#include <assert.h>
#include <dlfcn.h>
#include <stdio.h>
#include <string.h>
#include "c66x_priv.h"
static c66x_core *make(uint8_t *ram) {
    memset(ram,0,0x6000);
    memcpy(ram+0x5d30,"\x12\xaa\xf5\x1f",4);
    memcpy(ram+0xa70,"\x62\x03\x0c\x00",4);
    c66x_core *c=c66x_new(NULL);c66x_map_ram(c,0xc0049000,0x6000,ram);
    c66x_reset(c,0xc004ed30);c->reg[35]=0xdeadbeef;
    '''+(r'''memcpy(ram+0xa70,"\xe6\xd2\xbc\x01",4);memcpy(ram+0xa80,"\x62\x03\x0c\x00",4);
    memcpy(ram+0x5008,"\x38\xed\x04\xc0",4);c->reg[47]=0xc004dff0;''' if overwrite else '')+r'''
    return c;
}
int main(int argc,char **argv) {
    void *module=dlopen(argv[1],RTLD_NOW);assert(module);
    const c66x_jit_module *m=dlsym(module,"c66x_jit_exports");assert(m && m->nregions0);
    unsigned root=0;while(root<m->nregions0 && m->regions0[root].pc!=0xc004ed30)root++;assert(root<m->nregions0);
    for(unsigned budget=1;budget<=48;budget++) {
        uint8_t ra[0x6000],rb[0x6000];c66x_core *a=make(ra),*b=make(rb);uint64_t done;
        m->regions0[root].fn(a,budget);
        assert(c66x_step(b,a->cycle,&done)==C66X_STOP_BUDGET);
        if(a->jit_committed)commit_writes(b);
        if(c66x_state_hash(a)!=c66x_state_hash(b)) {
            fprintf(stderr,"budget %u cycle %llu pc %08x/%08x\n",budget,(unsigned long long)a->cycle,a->pc,b->pc);return 1;
        }
        if('''+('0' if overwrite else '1')+r''' && a->cycle!=budget) {
            fprintf(stderr,"call returned to the dispatcher at cycle %llu instead of staying compiled through budget %u\n",(unsigned long long)a->cycle,budget);return 2;
        }
        c66x_free(a);c66x_free(b);
    }
    dlclose(module);return 0;
}
''')
            flags=shlex.split(subprocess.check_output(['pkg-config','--cflags','gmodule-2.0'],text=True))
            build=subprocess.run(['cc','-O2','-std=gnu11','-I',str(DSP),str(source),str(LIB),'-o',str(p/'test'),*flags],capture_output=True,text=True)
            self.assertEqual(build.returncode,0,build.stderr)
            run=subprocess.run([str(p/'test'),str(p/'m.so')],env=env,capture_output=True,text=True)
            self.assertEqual(run.returncode,0,run.stderr)
