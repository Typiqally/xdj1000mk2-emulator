"""C674x compact register selection, independent of proprietary firmware."""
import ctypes
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest

CORE = Path(__file__).resolve().parents[2]/'native/qemu/hw/cdj/c6x'
BASE = 0xc1000000

def bind(library):
    lib=ctypes.CDLL(str(library))
    hook=ctypes.CFUNCTYPE(ctypes.c_int,ctypes.c_void_p,ctypes.c_void_p)
    for name,args in {
        'c66x_new_c674x':[ctypes.c_void_p],'c66x_free':[ctypes.c_void_p],
        'c66x_map_ram':[ctypes.c_void_p,ctypes.c_uint32,ctypes.c_uint32,ctypes.c_void_p],
        'c66x_reset':[ctypes.c_void_p,ctypes.c_uint32],
        'c66x_set_reg':[ctypes.c_void_p,ctypes.c_uint,ctypes.c_uint32],
        'c66x_get_reg':[ctypes.c_void_p,ctypes.c_uint],
        'c66x_get_pc':[ctypes.c_void_p],
        'c66x_hook_pc':[ctypes.c_void_p,ctypes.c_uint32,hook,ctypes.c_void_p],
        'c66x_step':[ctypes.c_void_p,ctypes.c_uint64,ctypes.POINTER(ctypes.c_uint64)],
    }.items():getattr(lib,name).argtypes=args
    lib.c66x_new_c674x.restype=ctypes.c_void_p
    lib.c66x_get_reg.restype=ctypes.c_uint32
    lib.c66x_get_pc.restype=ctypes.c_uint32
    return lib,hook


def execute_compact(library,opcode,rs,registers):
    lib,hook_type=bind(library)
    core=lib.c66x_new_c674x(None)
    code=(ctypes.c_uint8*32)()
    struct.pack_into('<H',code,0,opcode)
    # Word0 is compact. Its two instructions have no parallel P bits; the
    # stop hook runs before the second instruction executes.
    struct.pack_into('<I',code,28,0xe0200000 | (int(rs)<<19))
    reached=[]
    @hook_type
    def stop(c,opaque):reached.append(True);return 1
    try:
        lib.c66x_map_ram(core,BASE,len(code),ctypes.addressof(code))
        lib.c66x_reset(core,BASE)
        for reg,value in registers.items():lib.c66x_set_reg(core,reg,value&0xffffffff)
        lib.c66x_hook_pc(core,BASE+2,stop,None)
        cycles=ctypes.c_uint64()
        lib.c66x_step(core,100,ctypes.byref(cycles))
        if not reached or lib.c66x_get_pc(core)!=BASE+2:
            raise RuntimeError('Compact regression did not stop after original instruction')
        return {i:lib.c66x_get_reg(core,i) for i in range(64)}
    finally:lib.c66x_free(core)


class CompactRegisterTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = tempfile.TemporaryDirectory(prefix='xdj-compact-registers-')
        cls.addClassCleanup(cls.tmp.cleanup)
        cls.library = Path(cls.tmp.name)/'libc66x.so'
        result = subprocess.run(['make', '-C', str(CORE), 'O='+cls.tmp.name,
                                 str(cls.library)], capture_output=True, text=True)
        if result.returncode:
            raise RuntimeError(result.stderr[-3000:])

    def test_predicate_destinations_ignore_rs_and_data_sources_follow_rs(self):
        # Both C674x register banks and both fixed predicate destinations.
        # Pointer decoys catch corrupting A16/A17/B16/B17 even when the Boolean
        # result would accidentally match the expected source-bank result.
        for rs in (0,1):
            for side in (0,1):
                for dst in (0,1):
                    for name,opbits in [('eq',None),('lt',0),('gt',1),('ltu',2),('gtu',3)]:
                        for value in (0,1,2,0xffffffff):
                            with self.subTest(rs=rs,side=side,dst=dst,name=name,value=value):
                                bank=side*32
                                opcode=(0x26 | (1<<13) if name=='eq' else 0x1026 | (opbits<<14) | (1<<13))
                                opcode|=(2<<7)|(dst<<11)|side
                                regs={bank+dst:0x99,bank+16+dst:0x1180fb70,
                                      bank+2:3,bank+18:3}
                                regs[bank+2+16*rs]=value
                                signed=value if value<0x80000000 else value-2**32
                                expected={'eq':value==1,'lt':1<signed,'gt':1>signed,
                                          'ltu':1<value,'gtu':1>value}[name]
                                out=execute_compact(self.library,opcode,rs,regs)
                                self.assertEqual(out[bank+dst],int(expected))
                                self.assertEqual(out[bank+16+dst],0x1180fb70)

    def test_register_compare_existing_nors_and_three_bit_data_destination(self):
        for rs in (0,1):
            for side in (0,1):
                bank=side*32
                # L2c CMPEQ register form already specifies OWREG1NORS.
                opcode=0x0408 | (3<<5) | (3<<13) | (2<<7) | (1<<4) | side
                regs={bank+3+16*rs:123,bank+2+16*rs:123,bank+1:0,bank+17:0x1180fb70}
                out=execute_compact(self.library,opcode,rs,regs)
                self.assertEqual(out[bank+1],1)
                self.assertEqual(out[bank+17],0x1180fb70)
                # L3i ADD's 3-bit data destination retains RS selection.
                opcode=0x0400 | (1<<13) | (2<<7) | (4<<4) | side
                regs={bank+2+16*rs:23,bank+4:77,bank+20:88}
                out=execute_compact(self.library,opcode,rs,regs)
                self.assertEqual(out[bank+4+16*rs],24)
                self.assertEqual(out[bank+20 if not rs else bank+4],88 if not rs else 77)

    def test_existing_full_register_compact_moves_are_unchanged(self):
        # SPRU732J G-1/G-2: RS remaps only the 3-bit field, not the
        # 5-bit source/destination naming any architectural register.
        for opcode,regs,dest,value in (
            (0x4307,{38:6,54:22,50:0},50,6),
            (0xb98e,{51:19,53:0,19:77},21,19),
            (0x24d6,{17:17,9:0,25:25},9,17),
            (0x9047,{16:16,48:88,36:0},36,16),
        ):
            with self.subTest(opcode=hex(opcode)):
                out=execute_compact(self.library,opcode,1,regs)
                self.assertEqual(out[dest],value)


if __name__ == '__main__': unittest.main()
