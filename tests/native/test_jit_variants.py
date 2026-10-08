from native_support import core_library
"""Compiled entries must survive native code overlays and format changes."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
ROOT=Path(__file__).resolve().parents[2]
DSP=ROOT/'native/qemu/hw/cdj/c6x'
LIB=core_library()

@unittest.skipUnless(LIB.exists(),'requires built DSP library')
class VariantTests(unittest.TestCase):
    def test_shorter_valid_footprint_survives_until_later_dma_completes_newer_region(self):
        with tempfile.TemporaryDirectory(prefix='xdj-jit-footprint-') as d:
            folder=Path(d);modules=[]
            for name,count,tag in [('short',1,1),('long',2,2)]:
                source=folder/(name+'.c');module=folder/(name+'.so')
                source.write_text('''#include "c66x_core_int.h"
const char c66x_jit_isa[]="c674x";
static void run(c66x_core *c,uint64_t end){c->reg[0]=%d;}
static const uint32_t addr[]={0x11800000,0x11800100};
static const uint8_t bytes[64]={0};
static const c66x_jit_region r[]={ {.pc=0x11800000,.fn=run,.ndeps=%d,.dep_addr=addr,.dep_bytes=bytes} };
const c66x_jit_module c66x_jit_exports={.abi=C66X_JIT_ABI,.nregions0=1,.regions0=r};
'''%(tag,count))
                subprocess.run(['cc','-shared','-fPIC','-I',str(DSP),str(source),'-o',str(module)],check=True,capture_output=True)
                modules.append(module)
            driver=folder/'driver.c';driver.write_text(r'''
#include <assert.h>
#include <string.h>
#include "c66x_priv.h"
int main(void) {
    c66x_core *c=c66x_new_c674x(NULL);uint8_t ram[512]={0};ram[0x100]=1;
    c66x_map_ram(c,0x11800000,sizeof ram,ram);c66x_reset(c,0x11800000);
    const c66x_jit_region *r=jit_lookup0(c,0x11800000);assert(r);r->fn(c,0);assert(c->reg[0]==1);
    uint64_t generation=c->code_gen;
    /* The head is unchanged. The previously missing distant dependency now
     * arrives; failed checks must be retried even without a new generation. */
    ram[0x100]=0;r=jit_lookup0(c,0x11800000);assert(r);r->fn(c,0);assert(c->reg[0]==2);
    assert(c->code_gen==generation);c66x_free(c);
}
''')
            flags=subprocess.check_output(['pkg-config','--cflags','--libs','gmodule-2.0'],text=True).split()
            binary=folder/'driver'
            subprocess.run(['cc','-std=gnu11','-I',str(DSP),str(driver),str(LIB),*flags,'-o',str(binary)],check=True,capture_output=True)
            env={k:v for k,v in os.environ.items() if not k.startswith('C66X_')}
            env['C66X_JIT']=os.pathsep.join(map(str,modules))
            result=subprocess.run([str(binary)],env=env,capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stderr)

    def test_byte_verified_variants_survive_batch_replacement_and_late_dma(self):
        with tempfile.TemporaryDirectory(prefix='xdj-jit-variants-') as d:
            folder=Path(d)
            modules=[]
            for name,byte,tag in [('old',0,1),('overlay',1,2),('improved',0,3)]:
                source=folder/(name+'.c');module=folder/(name+'.so')
                source.write_text('''#include "c66x_core_int.h"
const char c66x_jit_isa[]="c674x";
static void run(c66x_core *c,uint64_t end){c->reg[0]=%d;}
static const uint32_t addr[]={0x11800000};
static const uint8_t bytes[32]={%d};
static c66x_jit_verified memo;
static const c66x_jit_region r[]={ {.pc=0x11800000,.fn=run,.ndeps=1,.dep_addr=addr,.dep_bytes=bytes,.verified=&memo} };
static const c66x_jit_qregion q[]={ {.r={.pc=0x11800000,.fn=run,.ndeps=1,.dep_addr=addr,.dep_bytes=bytes},.rem=5,.target=0x11800080} };
const c66x_jit_module c66x_jit_exports={.abi=C66X_JIT_ABI,.nregions=1,.regions=r,.nregions0=1,.regions0=r,.nqregions=1,.qregions=q};
'''%(tag,byte))
                subprocess.run(['cc','-shared','-fPIC','-I',str(DSP),str(source),'-o',str(module)],check=True,capture_output=True)
                modules.append(module)
            driver=folder/'driver.c';driver.write_text(r'''
#include <assert.h>
#include <string.h>
#include "c66x_priv.h"
static void check(c66x_core *c,unsigned tag) {
    const c66x_jit_region *r=jit_lookup(c,0x11800000);assert(r);r->fn(c,0);assert(c->reg[0]==tag);
    r=jit_lookup0(c,0x11800000);assert(r);r->fn(c,0);assert(c->reg[0]==tag);
    const c66x_jit_qregion *q=jit_lookupq(c,0x11800000);assert(q);q->r.fn(c,0);assert(c->reg[0]==tag);
    assert(q->rem==5 && q->target==0x11800080);
}
int main(int argc,char **argv) {
    for(unsigned iteration=0;iteration<5;iteration++) {
        c66x_core *c=argc>1?c66x_new_c674x(NULL):c66x_new(NULL);
        uint8_t ram[512];memset(ram,0xff,sizeof ram);
        c66x_map_ram(c,0x11800000,sizeof ram,ram);c66x_reset(c,0x11800000);
        assert(!jit_lookup(c,0x11800000));assert(!jit_lookup0(c,0x11800000));assert(!jit_lookupq(c,0x11800000));
        /* Neither variant matched, so DMA can bring bytes without dropping a
         * decoded block or changing code_gen. A failed check cannot be frozen. */
        uint64_t gen=c->code_gen;memset(ram,0,32);check(c,3);assert(c->code_gen==gen);
        ram[0]=1;c66x_invalidate(c,0x11800000,32);check(c,2);
        ram[0]=0;c66x_invalidate(c,0x11800000,32);check(c,3);
        memset(ram,0xff,32);c66x_invalidate(c,0x11800000,32);
        assert(!jit_lookup(c,0x11800000));assert(!jit_lookup0(c,0x11800000));assert(!jit_lookupq(c,0x11800000));
        assert(c->jit->nregions==1 && c->jit->nregions0==1 && c->jit->nqregions==1);
        c66x_free(c);
    }
}
''')
            flags=subprocess.check_output(['pkg-config','--cflags','--libs','gmodule-2.0'],text=True).split()
            binary=folder/'driver'
            subprocess.run(['cc','-std=gnu11','-I',str(DSP),str(driver),str(LIB),*flags,'-o',str(binary)],check=True,capture_output=True)
            env={k:v for k,v in os.environ.items() if not k.startswith('C66X_')}
            env['C66X_JIT']=os.pathsep.join(map(str,modules))
            for args in ([],['native']):
                run=subprocess.run([str(binary),*args],env=env,capture_output=True,text=True)
                self.assertEqual(run.returncode,0,run.stderr)

if __name__=='__main__':unittest.main()
