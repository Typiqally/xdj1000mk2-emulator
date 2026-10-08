from native_support import core_library
"""Compile separate hot call sites even when their queued target is shared."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
DSP = ROOT / 'native/qemu/hw/cdj/c6x'
LIB = core_library()


@unittest.skipUnless(LIB.exists(), 'requires the built DSP library')
class QueuedJitTests(unittest.TestCase):
    def test_static_region_can_cross_an_existing_short_region(self):
        with tempfile.TemporaryDirectory() as d:
            folder = Path(d)
            (folder / 'ram_0.bin').write_bytes(bytes(1024))
            (folder / 'profile.txt').write_text(
                'idle_head 0x00000000\nmap 0x11800000 0x00000400 0\n'
                'prof 0x11800000 2000 2000\nprof 0x11800004 1500 1500\n'
                'have 0x11800004\n')
            env = {k: v for k, v in os.environ.items() if not k.startswith('C66X_')}
            result = subprocess.run(
                [sys.executable, str(DSP / 'tools/c14_jitgen.py'), str(folder),
                 str(folder / 'm'), '--lib', str(LIB), '--roots', '2', '--min', '1000',
                 '--kernels', '0', '--qroots', '0', '--nodes', '4', '--total', '16',
                 '--cold', '0', '--no-chain', '--no-cc'],
                env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            source = (folder / 'm.main.c').read_text()
            self.assertIn('static void clean_11800004(', source)
            # The existing root stays in the have set: only its short boundary
            # is crossed, rather than offering it as a redundant new root.
            self.assertEqual((folder / 'm.tried').read_text(), '0x11800000\n')

    def test_shared_destination_keeps_both_call_sites_and_hottest_context(self):
        with tempfile.TemporaryDirectory() as d:
            folder = Path(d)
            # Uncompressed NOP instructions; no firmware or prebuilt cache is
            # needed to exercise the generator's queued-entry selection.
            (folder / 'ram_0.bin').write_bytes(bytes(1024))
            (folder / 'profile.txt').write_text(
                'idle_head 0x00000000\nmap 0x11800000 0x00000400 0\n'
                'qprof 0x11800000 5 0x11800080 2000\n'
                'qprof 0x11800020 5 0x11800080 1500\n'
                'qprof 0x11800000 4 0x118000a0 1200\n')
            env = {k: v for k, v in os.environ.items()
                   if not k.startswith('C66X_')}
            result = subprocess.run(
                [sys.executable, str(DSP / 'tools/c14_jitgen.py'), str(folder),
                 str(folder / 'm'), '--lib', str(LIB), '--roots', '0',
                 '--kernels', '0', '--qroots', '2', '--qmin', '1000',
                 '--nodes', '16', '--total', '32', '--cold', '0', '--no-cc'],
                env=env, capture_output=True, text=True, check=True)
            source = (folder / 'm.main.c').read_text()
            self.assertIn('qroot_11800000', source, result.stderr)
            self.assertIn('qroot_11800020', source, result.stderr)
            self.assertEqual(source.count('5, 0x11800080u },'), 2)
            self.assertNotIn('4, 0x118000a0u },', source)

    def test_adaptive_profiler_offers_branch_delay_entries_once(self):
        self.check_adaptive_profiler(True)

    def test_adaptive_profiler_compiles_ranked_loops_without_addresses(self):
        self.check_adaptive_profiler(False)

    def check_adaptive_profiler(self, specific):
        with tempfile.TemporaryDirectory() as d:
            folder = Path(d)
            source = folder / 'profile.c'
            source.write_text(r'''
#include <assert.h>
#include <string.h>
#include "c66x_priv.h"
int main(void) {
    c66x_core *c = c66x_new(NULL);
    uint8_t ram[1024] = {0};
    c66x_map_ram(c, 0x11800000, sizeof ram, ram);
    /* Warm the decoded packet, then enter it with a branch already pending.
     * These visits have zero free-root hits and need their own profile. */
    c66x_reset(c, 0x11800000);
    uint64_t n;
    assert(c66x_step(c, 1, &n) == C66X_STOP_BUDGET);
    for (int i = 0; i < 1201; i++) {
        c->pc = 0x11800000;
        c->nbr = 1;
        c->br[0] = (pend_branch){5, 0x11800080};
        assert(c66x_step(c, 1, &n) == C66X_STOP_BUDGET);
    }
    c->jit->auto_last = -c->jit->auto_interval;
    jit_auto_poll(c);
    assert(c->jit->auto_busy);
    while (c->jit->auto_busy) {
        g_usleep(1000);
        jit_auto_poll(c);
    }
    unsigned batch = c->jit->auto_batch;
    /* A newly observed colder context at the same site must not replace
     * the selected context or restart unsuccessful compilation. */
    for (int i = 0; i < 1201; i++) {
        c->pc = 0x11800000;
        c->nbr = 1;
        c->br[0] = (pend_branch){4, 0x118000a0};
        assert(c66x_step(c, 1, &n) == C66X_STOP_BUDGET);
    }
    c->jit->auto_last = -c->jit->auto_interval;
    jit_auto_poll(c);
    assert(c->jit->auto_batch == batch);
    c66x_free(c);
    return 0;
}
''')
            flags = subprocess.check_output(
                ['pkg-config', '--cflags', '--libs', 'gmodule-2.0'],
                text=True).split()
            binary = folder / 'profile'
            subprocess.run(['cc', '-std=gnu11', '-I', str(DSP), str(source),
                            str(LIB), *flags, '-o', str(binary)],
                           check=True, capture_output=True)
            # A deliberately unsuccessful generator is still offered only
            # once. Capture the complete profile/argv without invoking a shell.
            generator = folder / 'generator.py'
            generator.write_text('import json, pathlib, sys\n'
                                 'pathlib.Path(sys.argv[2]+".args").write_text('
                                 'json.dumps(sys.argv[3:]))\n')
            env = {k: v for k, v in os.environ.items()
                   if not k.startswith('C66X_')}
            env.update(C66X_JIT_AUTO=str(folder / 'cache'),
                       C66X_JIT_PYTHON=sys.executable,
                       C66X_JIT_GEN=str(generator), C66X_JIT_AUTO_MIN='1000',
                       C66X_JIT_AUTO_S='1000000',
                       C66X_JIT_AUTO_COLD='0', C66X_JIT_AUTO_NODES='1200',
                       C66X_JIT_AUTO_CHAIN='0')
            env.update({'C66X_JIT_AUTO_LOOP_ONLY':'0x11800000'} if specific else {'C66X_JIT_AUTO_LOOPS':'32'})
            result = subprocess.run([str(binary)], env=env, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr.decode())
            profile = (folder / 'cache/batch0/profile.txt').read_text()
            self.assertIn('qprof 0x11800000 5 0x11800080 1200', profile)
            import json
            args = json.loads((folder / 'cache/batch0/m.args').read_text())
            self.assertIn('--qroots', args)
            self.assertIn('--qmin', args)
            self.assertIn('--no-chain', args)
            self.assertGreater(int(args[args.index('--owner-pid') + 1]), 0)
            self.assertEqual(args[args.index('--cold') + 1], '0')
            self.assertEqual(args[args.index('--nodes') + 1], '1200')
            if specific:
                self.assertEqual(args[args.index('--loop-only') + 1], '0x11800000')
            else:
                self.assertNotIn('--loop-only', args)
                self.assertEqual(args[args.index('--loops') + 1], '32')
