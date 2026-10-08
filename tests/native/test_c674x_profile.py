from native_support import core_library
"""Documented C674x CSR semantics and exclusion of C66x-only encodings."""
import ctypes
import shlex
import subprocess
import tempfile
import unittest
from pathlib import Path

LIB = core_library()


class C674xTests(unittest.TestCase):
    def test_sat_write_one_preserves_functional_unit_status_write_zero_clears(self):
        root=Path(__file__).resolve().parents[2]
        core=root/'native/qemu/hw/cdj/c6x'
        code=r'''
#include <assert.h>
#include "c66x_priv.h"
int main(void) {
    c66x_core *c=c66x_new(NULL);
    c->cr[CR_CSR]|=CSR_SAT; /* functional-unit saturation already occurred */
    c66x_set_creg(c,CR_CSR,0x200);assert(c66x_get_creg(c,CR_CSR)&CSR_SAT);
    c66x_set_creg(c,CR_CSR,0);assert(!(c66x_get_creg(c,CR_CSR)&CSR_SAT));
    c66x_set_creg(c,CR_CSR,0x200);assert(!(c66x_get_creg(c,CR_CSR)&CSR_SAT));
    c66x_free(c);
}
'''
        with tempfile.TemporaryDirectory() as d:
            path=Path(d);source=path/'sat.c';source.write_text(code)
            flags=shlex.split(subprocess.check_output(['pkg-config','--cflags','gmodule-2.0'],text=True))
            build=subprocess.run(['cc','-I',str(core),str(source),str(LIB),'-o',str(path/'sat'),*flags],capture_output=True,text=True)
            self.assertEqual(build.returncode,0,build.stderr)
            run=subprocess.run([str(path/'sat')],capture_output=True,text=True)
            self.assertEqual(run.returncode,0,run.stderr)

    def setUp(self):
        self.lib = ctypes.CDLL(str(LIB))
        self.lib.c66x_new.argtypes = [ctypes.c_void_p]
        self.lib.c66x_new.restype = ctypes.c_void_p
        self.lib.c66x_free.argtypes = [ctypes.c_void_p]
        self.lib.c66x_get_creg.argtypes = [ctypes.c_void_p,ctypes.c_uint]
        self.lib.c66x_get_creg.restype = ctypes.c_uint32
        self.lib.c66x_set_creg.argtypes = [ctypes.c_void_p,ctypes.c_uint,ctypes.c_uint32]

    def test_csr_identity_and_endianness_read_only_sat_cleared_on_write(self):
        # SPRUFE8B table 2-9: ID=14h, EN is read-only, SAT cannot be set by MVC.
        core = self.lib.c66x_new(None)
        try:
            self.assertEqual(self.lib.c66x_get_creg(core,1),0x14000100)
            self.lib.c66x_set_creg(core,1,0xffffffff)
            self.assertEqual(self.lib.c66x_get_creg(core,1)&0xffff0300,0x14000100)
            self.lib.c66x_set_creg(core,1,0)
            self.assertEqual(self.lib.c66x_get_creg(core,1),0x14000100)
        finally:
            self.lib.c66x_free(core)

    def test_c674x_rejects_c66x_only_instruction_and_retains_standard_nop(self):
        make = self.lib.c66x_new_c674x
        make.argtypes = [ctypes.c_void_p]; make.restype = ctypes.c_void_p
        self.lib.c66x_map_ram.argtypes = [ctypes.c_void_p,ctypes.c_uint32,ctypes.c_uint32,ctypes.c_void_p]
        self.lib.c66x_reset.argtypes = [ctypes.c_void_p,ctypes.c_uint32]
        self.lib.c66x_step.argtypes = [ctypes.c_void_p,ctypes.c_uint64,ctypes.POINTER(ctypes.c_uint64)]
        self.lib.c66x_step.restype = ctypes.c_int
        # C66x SHL2 .S1 A0,4,A15 from SPRUGH7 (not in the C674x ISA).
        word = (1<<28)|(15<<23)|(4<<13)|(3<<6)|(8<<2)
        ram = ctypes.create_string_buffer(64)
        ram[4:8] = word.to_bytes(4,'little')
        core = make(None)
        try:
            self.lib.c66x_map_ram(core,0x800000,64,ram)
            self.lib.c66x_reset(core,0x800000)
            cycles=ctypes.c_uint64()
            self.assertEqual(self.lib.c66x_step(core,1,ctypes.byref(cycles)),0)
            self.assertEqual(self.lib.c66x_step(core,4,ctypes.byref(cycles)),2)
        finally:
            self.lib.c66x_free(core)
