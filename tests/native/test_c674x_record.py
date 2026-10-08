from native_support import core_library
"""New DSP recordings retain their CPU model through standalone replay."""
import ctypes
from pathlib import Path
import subprocess
import tempfile
import unittest
import os

ROOT=Path(__file__).resolve().parents[2]
LIB=core_library()


class RecordTests(unittest.TestCase):
    def test_c674x_and_legacy_recordings_replay_with_their_original_decoder(self):
        lib=ctypes.CDLL(str(LIB))
        for name in ('c66x_new','c66x_new_c674x'):
            method=getattr(lib,name);method.argtypes=[ctypes.c_void_p];method.restype=ctypes.c_void_p
        lib.c66x_free.argtypes=[ctypes.c_void_p]
        lib.c66x_record_open.argtypes=[ctypes.c_void_p,ctypes.c_char_p]
        lib.c66x_map_ram.argtypes=[ctypes.c_void_p,ctypes.c_uint32,ctypes.c_uint32,ctypes.c_void_p]
        lib.c66x_reset.argtypes=[ctypes.c_void_p,ctypes.c_uint32]
        lib.c66x_step.argtypes=[ctypes.c_void_p,ctypes.c_uint64,ctypes.POINTER(ctypes.c_uint64)]
        with tempfile.TemporaryDirectory() as d:
            for factory,tag,model in [('c66x_new_c674x',b'K','C674x'),('c66x_new',b'N','compatibility')]:
                core=getattr(lib,factory)(None);memory=ctypes.create_string_buffer(4096)
                path=Path(d)/(factory+'.bin')
                try:
                    self.assertEqual(lib.c66x_record_open(core,str(path).encode()),0)
                    lib.c66x_map_ram(core,0x800000,4096,memory)
                    lib.c66x_reset(core,0x800000)
                    done=ctypes.c_uint64()
                    self.assertEqual(lib.c66x_step(core,20,ctypes.byref(done)),0)
                finally:lib.c66x_free(core)
                self.assertEqual(path.read_bytes()[:1],tag)
                hashes=Path(d)/(factory+'.hashes')
                result=subprocess.run([str(LIB.with_name('c6xreplay')),str(path)],
                    capture_output=True,text=True,env={**os.environ,'REPLAY_STATE_LOG':str(hashes)})
                self.assertEqual(result.returncode,0,result.stderr)
                self.assertIn('CPU '+model,result.stderr)
                self.assertIn('verdict: EXACT',result.stdout)
                self.assertTrue(hashes.is_file())
                self.assertTrue(hashes.read_text().strip())
                # Refuse to replace an existing record or report via the log.
                before=path.read_bytes()
                failed=subprocess.run([str(LIB.with_name('c6xreplay')),str(path)],
                    capture_output=True,text=True,env={**os.environ,'REPLAY_STATE_LOG':str(path)})
                self.assertNotEqual(failed.returncode,0)
                self.assertEqual(path.read_bytes(),before)
