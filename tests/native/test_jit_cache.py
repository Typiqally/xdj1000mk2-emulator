from native_support import core_library
"""Empty adaptive compilation batches must not exhaust the module cache."""
import ctypes
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
DSP = ROOT / 'native/qemu/hw/cdj/c6x'
LIB = core_library()

@unittest.skipUnless(LIB.exists(), 'requires the built DSP library')
class JitCacheTests(unittest.TestCase):
    def test_warm_cache_remembers_uncompilable_roots_and_queued_sites(self):
        import sys
        with tempfile.TemporaryDirectory() as d:
            folder = Path(d)
            batch = folder / 'cache' / 'batch0'; batch.mkdir(parents=True)
            source = folder / 'empty.c'
            source.write_text('#include "c66x_core_int.h"\n'
                'const c66x_jit_module c66x_jit_exports={.abi=C66X_JIT_ABI};\n')
            subprocess.run(['cc', '-shared', '-fPIC', '-I', str(DSP), str(source),
                            '-o', str(batch / 'm.so')], check=True, capture_output=True)
            (batch / 'm.tried').write_text('0x11800000\n')
            (batch / 'm.qtried').write_text('0x11800040 5 0x11800080\n')
            generator = folder / 'generator.py'
            generator.write_text('import pathlib,sys\n'
                'pathlib.Path(sys.argv[2]+".tried").write_text("0x11800000\\n")\n'
                'pathlib.Path(sys.argv[2]+".qtried").write_text("0x11800040 5 0x11800080\\n")\n')
            driver = folder / 'warm-tried.c'
            driver.write_text(r'''
#include <assert.h>
#include <stdlib.h>
#include "c66x_priv.h"
int main(int argc,char **argv) {
    c66x_core *c=c66x_new(NULL); uint8_t ram[1024]={0}; uint64_t done;
    c66x_map_ram(c,0x11800000,sizeof ram,ram); c66x_reset(c,0x11800000);
    for(unsigned i=0;i<1201;i++) {
        c->pc=0x11800000; c->nbr=0; c66x_step(c,1,&done);
        /* A later queued context at the same site inherits the attempted
         * site's admission result, just as it does within one live run. */
        c->nbr=1; c->br[0].remaining=4; c->br[0].target=0x118000c0;
        jit_qprof(c,0x11800040);
    }
    c->nbr=0; c->jit->auto_last=-c->jit->auto_interval; jit_auto_poll(c);
    while(c->jit->auto_busy) { g_usleep(1000); jit_auto_poll(c); }
    unsigned batches=c->jit->auto_batch;
    c66x_free(c);
    assert(batches==(unsigned)atoi(argv[1]));
}
''')
            flags = subprocess.check_output(['pkg-config', '--cflags', '--libs', 'gmodule-2.0'], text=True).split()
            binary = folder / 'warm-tried'
            subprocess.run(['cc', '-std=gnu11', '-I', str(DSP), str(driver),
                            str(LIB), *flags, '-o', str(binary)], check=True, capture_output=True)
            env = {k: v for k, v in os.environ.items() if not k.startswith('C66X_')}
            env.update(C66X_JIT_AUTO=str(folder / 'cache'), C66X_JIT_AUTO_MIN='1000',
                       C66X_JIT_AUTO_S='1000000', C66X_JIT_GEN=str(generator),
                       C66X_JIT_PYTHON=sys.executable)
            for version, expected in ((2, 1), (1, 2)):
                (batch / 'm.cache').write_text(str(version)+'\n')
                result = subprocess.run([str(binary), str(expected)], env=env,
                                        capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)

    def test_mapped_region_that_fails_verification_does_not_schedule_empty_batches(self):
        import sys
        with tempfile.TemporaryDirectory() as d:
            folder = Path(d)
            source = folder / 'mapped.c'
            source.write_text('#include "c66x_core_int.h"\n'
                'static const uint32_t addr[]={0x11800000};\n'
                'static const uint8_t bytes[32]={1};\n'
                'static const c66x_jit_region r[]={ {.pc=0x11800000,.ndeps=1,.dep_addr=addr,.dep_bytes=bytes} };\n'
                'const c66x_jit_module c66x_jit_exports={.abi=C66X_JIT_ABI,.nregions=1,.regions=r};\n')
            subprocess.run(['cc', '-shared', '-fPIC', '-I', str(DSP), str(source),
                            '-o', str(folder / 'mapped.so')], check=True, capture_output=True)
            generator = folder / 'generator.py'
            generator.write_text('import pathlib,sys\n'
                'pathlib.Path(sys.argv[2]+".tried").write_text("")\n'
                'pathlib.Path(sys.argv[2]+".qtried").write_text("")\n')
            driver = folder / 'mapped-test.c'
            driver.write_text(r'''
#include <assert.h>
#include "c66x_priv.h"
int main(void) {
    c66x_core *c=c66x_new(NULL); uint8_t ram[1024]={0}; uint64_t done;
    c66x_map_ram(c,0x11800000,sizeof ram,ram); c66x_reset(c,0x11800000);
    assert(!jit_lookup(c,0x11800000));
    for(unsigned i=0;i<1201;i++) {
        c->pc=0x11800000; assert(c66x_step(c,1,&done)==C66X_STOP_BUDGET);
    }
    c->jit->auto_last=-c->jit->auto_interval; jit_auto_poll(c);
    while(c->jit->auto_busy) { g_usleep(1000); jit_auto_poll(c); }
    unsigned batches=c->jit->auto_batch;
    c66x_free(c);
    /* The generator's have list already excludes this mapped PC. Failed
     * verification must keep interpretation, without offering it forever. */
    assert(batches==0);
}
''')
            flags = subprocess.check_output(['pkg-config', '--cflags', '--libs', 'gmodule-2.0'], text=True).split()
            binary = folder / 'mapped-test'
            subprocess.run(['cc', '-std=gnu11', '-I', str(DSP), str(driver),
                            str(LIB), *flags, '-o', str(binary)], check=True, capture_output=True)
            env = {k: v for k, v in os.environ.items() if not k.startswith('C66X_')}
            env.update(C66X_JIT=str(folder / 'mapped.so'), C66X_JIT_AUTO=str(folder / 'cache'),
                       C66X_JIT_AUTO_MIN='1000', C66X_JIT_AUTO_S='1000000',
                       C66X_JIT_GEN=str(generator), C66X_JIT_PYTHON=sys.executable)
            result = subprocess.run([str(binary)], env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_already_mapped_kernel_does_not_schedule_an_empty_batch(self):
        import sys
        with tempfile.TemporaryDirectory() as d:
            folder = Path(d)
            generator = folder / 'generator.py'
            generator.write_text('import pathlib,sys\n'
                'pathlib.Path(sys.argv[2]+".tried").write_text("")\n'
                'pathlib.Path(sys.argv[2]+".qtried").write_text("")\n')
            driver = folder / 'kernel-test.c'
            driver.write_text(r'''
#include <assert.h>
#include "c66x_priv.h"
int main(void) {
    c66x_core *c=c66x_new(NULL);
    const c66x_jit_kernel mapped={.addr=0x11800000,.ii=1,.dynlen=2};
    c->jit->kern[0].k=&mapped; c->jit->nkern=1;
    c->jit->nkprof=1;
    c->jit->kprof[0].addr=mapped.addr;
    c->jit->kprof[0].ii=mapped.ii;
    c->jit->kprof[0].dynlen=mapped.dynlen;
    c->jit->kprof[0].cycles=50000;
    c->jit->auto_last=-c->jit->auto_interval; jit_auto_poll(c);
    while(c->jit->auto_busy) { g_usleep(1000); jit_auto_poll(c); }
    unsigned batches=c->jit->auto_batch;
    c66x_free(c);
    assert(batches==0);
}
''')
            flags = subprocess.check_output(['pkg-config', '--cflags', '--libs', 'gmodule-2.0'], text=True).split()
            binary = folder / 'kernel-test'
            subprocess.run(['cc', '-std=gnu11', '-I', str(DSP), str(driver),
                            str(LIB), *flags, '-o', str(binary)], check=True, capture_output=True)
            env = {k: v for k, v in os.environ.items() if not k.startswith('C66X_')}
            env.update(C66X_JIT_AUTO=str(folder / 'cache'), C66X_JIT_AUTO_MIN='1000',
                       C66X_JIT_AUTO_S='1000000', C66X_JIT_GEN=str(generator),
                       C66X_JIT_PYTHON=sys.executable)
            result = subprocess.run([str(binary)], env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_warm_cache_loads_later_batches_after_earlier_batches(self):
        with tempfile.TemporaryDirectory() as d:
            folder = Path(d)
            for i in (10, 2):
                batch = folder / f'batch{i}'; batch.mkdir()
                source = batch / 'm.c'
                source.write_text('#include "c66x_core_int.h"\n'
                    'static void run(c66x_core *c,uint64_t end){c->reg[0]=%d;}\n'
                    'static const c66x_jit_region r[]={ {.pc=0x11800000,.fn=run} };\n'
                    'const c66x_jit_module c66x_jit_exports={.abi=C66X_JIT_ABI,.nregions=1,.regions=r};\n' % i)
                subprocess.run(['cc', '-shared', '-fPIC', '-I', str(DSP),
                                str(source), '-o', str(batch / 'm.so')], check=True, capture_output=True)
                (batch / 'm.cache').write_text('2\n')
            driver = folder / 'order.c'
            driver.write_text('#include <assert.h>\n#include "c66x_priv.h"\n'
                'int main(void){c66x_core *c=c66x_new(NULL);'
                'const c66x_jit_region *r=jit_lookup(c,0x11800000);assert(r);'
                'r->fn(c,0);assert(c->reg[0]==10);c66x_free(c);}\n')
            flags = subprocess.check_output(['pkg-config', '--cflags', '--libs', 'gmodule-2.0'], text=True).split()
            binary = folder / 'order'
            subprocess.run(['cc', '-std=gnu11', '-I', str(DSP), str(driver),
                            str(LIB), *flags, '-o', str(binary)], check=True, capture_output=True)
            env = {k: v for k, v in os.environ.items() if not k.startswith('C66X_')}
            env['C66X_JIT_AUTO'] = str(folder)
            result = subprocess.run([str(binary)], env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_new_adaptive_batch_replaces_cached_entry_pointers(self):
        with tempfile.TemporaryDirectory() as d:
            folder = Path(d)
            for name in ('old', 'new'):
                source = folder / (name + '.c')
                source.write_text('#include "c66x_core_int.h"\n'
                    'static const c66x_jit_region r[]={ {.pc=0x11800000} };\n'
                    'static const c66x_jit_qregion q[]={ {.r={.pc=0x11800000},.rem=5,.target=0x11800080} };\n'
                    'const c66x_jit_module c66x_jit_exports={.abi=C66X_JIT_ABI,'
                    '.nregions=1,.regions=r,.nregions0=1,.regions0=r,.nqregions=1,.qregions=q};\n')
                subprocess.run(['cc', '-shared', '-fPIC', '-I', str(DSP),
                                str(source), '-o', str(folder / (name + '.so'))],
                               check=True, capture_output=True)
            driver = folder / 'refresh.c'
            driver.write_text(r'''
#include <assert.h>
#include <stdio.h>
#include "c66x_priv.h"
static gpointer completed(gpointer data) { return NULL; }
int main(int argc,char **argv) {
    c66x_core *c=c66x_new(NULL);
    pkc_ent *e=&c->pkc[0];
    e->pc=0x11800000; e->first=(c66x_insn *)1;
    e->region=jit_lookup(c,e->pc); e->region0=jit_lookup0(c,e->pc);
    e->qregion=jit_lookupq(c,e->pc);
    const c66x_jit_region *old=e->region;
    assert(old && e->region0 && e->qregion);
    snprintf(c->jit->auto_out,sizeof c->jit->auto_out,"%s",argv[1]);
    c->jit->auto_busy=1; c->jit->auto_result=1;
    c->jit->auto_thr=g_thread_new("completed",completed,NULL);
    jit_auto_poll(c);
    assert(jit_lookup(c,e->pc)!=old);
    assert(e->region==jit_lookup(c,e->pc));
    assert(e->region0==jit_lookup0(c,e->pc));
    assert(e->qregion==jit_lookupq(c,e->pc));
    e->first=NULL;
    c66x_free(c);
}
''')
            flags = subprocess.check_output(['pkg-config', '--cflags', '--libs', 'gmodule-2.0'], text=True).split()
            binary = folder / 'refresh'
            subprocess.run(['cc', '-std=gnu11', '-I', str(DSP), str(driver),
                            str(LIB), *flags, '-o', str(binary)], check=True, capture_output=True)
            env = {k: v for k, v in os.environ.items() if not k.startswith('C66X_')}
            env.update(C66X_JIT=str(folder / 'old.so'), C66X_JIT_AUTO=str(folder / 'cache'))
            result = subprocess.run([str(binary), str(folder / 'new')], env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_old_compiler_cache_is_rebuilt_without_changing_explicit_module_abi(self):
        with tempfile.TemporaryDirectory() as d:
            folder = Path(d)
            for i in range(3):
                batch = folder / f'batch{i}'; batch.mkdir()
                source = batch / 'm.c'
                source.write_text('#include "c66x_core_int.h"\n'
                                  'static const c66x_jit_region r[]={{.pc=%du}};\n'
                                  'const c66x_jit_module c66x_jit_exports={'
                                  '.abi=C66X_JIT_ABI,.nregions=1,.regions=r};\n'
                                  % (0x11800000 + i * 32))
                subprocess.run(['cc', '-shared', '-fPIC', '-I', str(DSP),
                                str(source), '-o', str(batch / 'm.so')],
                               check=True, capture_output=True)
            (folder / 'batch0/m.cache').write_text('1\n')
            (folder / 'batch1/m.cache').write_text('2\n')
            (folder / 'batch2/m.cache').write_text('2\n')
            script = '''import ctypes,sys
lib=ctypes.CDLL(sys.argv[1]);lib.c66x_new.restype=ctypes.c_void_p
lib.c66x_new.argtypes=[ctypes.c_void_p]
lib.c66x_jit_report.argtypes=[ctypes.c_void_p,ctypes.c_char_p,ctypes.c_size_t]
lib.c66x_free.argtypes=[ctypes.c_void_p]
c=lib.c66x_new(None);b=ctypes.create_string_buffer(1024)
lib.c66x_jit_report(c,b,len(b));print(b.value.decode());lib.c66x_free(c)
'''
            import sys
            env = {k: v for k, v in os.environ.items() if not k.startswith('C66X_')}
            env['C66X_JIT_AUTO'] = str(folder)
            result = subprocess.run([sys.executable, '-c', script, str(LIB)],
                                    env=env, capture_output=True, text=True, check=True)
            self.assertIn('2 regions', result.stdout, result.stderr)
            self.assertIn('auto 2 cached 3 batches', result.stdout, result.stderr)

    def test_empty_batches_do_not_prevent_a_later_native_region_loading(self):
        with tempfile.TemporaryDirectory() as d:
            folder=Path(d)
            empty=folder/'empty.so'; live=folder/'live.so'
            for name, content in (
                ('empty', 'const c66x_jit_module c66x_jit_exports={.abi=C66X_JIT_ABI};'),
                ('live', 'static const c66x_jit_region regions[]={{.pc=0x11800000}};\n'
                         'const c66x_jit_module c66x_jit_exports={.abi=C66X_JIT_ABI,'
                         '.nregions=1,.regions=regions};')):
                source=folder/(name+'.c')
                source.write_text('#include "c66x_core_int.h"\n'+content)
                subprocess.run(['cc','-shared','-fPIC','-I',str(DSP),str(source),
                                '-o',str(folder/(name+'.so'))],check=True,capture_output=True)
            # Use a child so adaptive-JIT environment and GLib module handles
            # cannot affect any other component test in this process.
            script='''import ctypes,sys
lib=ctypes.CDLL(sys.argv[1]);lib.c66x_new.restype=ctypes.c_void_p
lib.c66x_new.argtypes=[ctypes.c_void_p]
lib.c66x_jit_report.argtypes=[ctypes.c_void_p,ctypes.c_char_p,ctypes.c_size_t]
lib.c66x_free.argtypes=[ctypes.c_void_p]
c=lib.c66x_new(None);b=ctypes.create_string_buffer(1024)
lib.c66x_jit_report(c,b,len(b));print(b.value.decode());lib.c66x_free(c)
'''
            import sys
            env={k:v for k,v in os.environ.items() if not k.startswith('C66X_')}
            env['C66X_JIT']=os.pathsep.join([str(empty)]*256+[str(live)])
            result=subprocess.run([sys.executable,'-c',script,str(LIB)],env=env,
                                  capture_output=True,text=True,check=True)
            self.assertIn('1 regions',result.stdout,result.stderr)
