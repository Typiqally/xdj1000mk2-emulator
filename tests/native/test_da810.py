"""Standalone DA810 device tests; no firmware or QEMU required."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
DEVICE = ROOT / 'native/qemu/hw/cdj/boards/xdj1000mk2'


class Da810Tests(unittest.TestCase):
    def test_host_interrupt_routes_through_guest_mux_and_latches_until_clear(self):
        source = r'''
#include <assert.h>
#include "da810_intc.h"
static unsigned pulses, last;
static void irq(void *opaque, unsigned line) { pulses++; last=line; }
int main(void) {
    Da810Intc s;
    da810_intc_init(&s, irq, 0);
    assert(da810_intc_read(&s, 0x80) == 0xffffffff);
    assert(da810_intc_read(&s, 0x104) == 0x07060504);
    assert(da810_intc_read(&s, 0x108) == 0x0b0a0908);
    da810_intc_event(&s, 8);
    assert(pulses == 1 && last == 8);
    pulses = 0;
    da810_intc_write(&s, 0x10c, 0x220e0d0c);
    da810_intc_event(&s, 34);
    assert(pulses == 1 && last == 15);
    assert(da810_intc_read(&s, 4) == 4);
    assert(da810_intc_read(&s, 0xa4) == 0);
    da810_intc_write(&s, 0x84, 0xfffffffb);
    assert(da810_intc_read(&s, 0xa4) == 4);
    da810_intc_write(&s, 0x44, 4);
    assert(da810_intc_read(&s, 4) == 0);
    da810_intc_write(&s, 0x24, 4);
    assert(pulses == 2 && last == 15);
    da810_intc_event(&s, 200);
    assert(pulses == 2);
    return 0;
}
'''
        with tempfile.TemporaryDirectory() as d:
            test = Path(d) / 'test.c'
            test.write_text(source)
            binary = Path(d) / 'test'
            subprocess.run(['cc', '-std=c11', '-Wall', '-Werror', '-I', str(DEVICE),
                            str(test), str(DEVICE / 'da810_intc.c'), '-o', str(binary)],
                           check=True, capture_output=True)
            subprocess.run([str(binary)], check=True, capture_output=True)
