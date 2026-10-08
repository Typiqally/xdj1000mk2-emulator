"""Command-side DSP instructions consume hardware time, including early exits."""
from pathlib import Path
import subprocess
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parents[2] / 'native/qemu/hw/cdj/boards/cdj2000/dsp_host.c'


class CommandClockTests(unittest.TestCase):
    def test_command_time_peripherals_fraction_fault_and_clock_gating(self):
        text = SOURCE.read_text()
        helper = ''
        if 'static void host_elapsed_cycles(' in text:
            helper = text[text.index('static void host_elapsed_cycles('):text.index('static void host_react(')]
        body = text[text.index('static void host_react('):text.index('static void set_hpic(')]
        code = r'''
#include <assert.h>
#include <stdbool.h>
#include <stdint.h>
#define REACT_STEP 2000
#define REACT_MAX 4000000
#define qatomic_add(p,v) (*(p)+=(v))
typedef struct CdjDspHost {
    bool running; unsigned core,command; uint64_t hint_edges,pin_reads,react_cycles;
    uint64_t clock_hz,react_fraction; int64_t dsp_ns;
    void (*advance)(void *,uint64_t); void *chip;
    struct { uint64_t react_us; } perf;
} CdjDspHost;
static CdjDspHost *active;
static unsigned calls, fault, idle, wakes;
static void c66x_idle_wake(unsigned core) { wakes++; }
static uint64_t peripheral_ns;
static int64_t perf_begin(CdjDspHost *h) { return 0; }
static void perf_elapsed(int64_t t,uint64_t *p) {}
static bool host_answered(CdjDspHost *h,uint64_t e,uint64_t r) {
    return h->command ? h->hint_edges!=e : h->pin_reads!=r;
}
static unsigned c66x_step(unsigned c,uint64_t n,uint64_t *done) {
    calls++; *done=idle?0:fault?37:n;
    if(!fault && !idle) { active->hint_edges++;active->pin_reads++; }
    return fault?4:idle?1:0;
}
static bool host_stopped(CdjDspHost *h,unsigned stop) {
    if(stop==4) h->running=false;return stop==4;
}
static void advance(void *chip,uint64_t ns) { peripheral_ns+=ns; }
''' + helper + body + r'''
int main(void) {
    CdjDspHost h={.running=true,.command=1,.clock_hz=451584000,.advance=advance};active=&h;
    for(unsigned i=0;i<300;i++) host_react(&h);
    uint64_t expected=UINT64_C(600000)*1000000000/451584000;
    assert(calls==300 && h.react_cycles==600000);
    assert(h.dsp_ns==expected && peripheral_ns==expected);
    h.clock_hz=22579200;h.react_fraction=0;int64_t before=h.dsp_ns;
    host_react(&h);assert(h.dsp_ns-before==UINT64_C(2000)*1000000000/22579200);
    fault=1;before=h.dsp_ns;host_react(&h);
    assert(!h.running && h.react_cycles==602037 && h.dsp_ns>before);
    h.running=true;fault=0;idle=1;before=h.dsp_ns;unsigned count=calls;
    host_react(&h);assert(calls==count+1 && h.dsp_ns==before);
    idle=0;h.clock_hz=0;count=calls;host_react(&h);assert(calls==count);
    /* A command is an input no store announces: a core idling at its
     * polling loop is woken before it is asked to answer. */
    assert(!wakes);h.core=1;h.clock_hz=451584000;host_react(&h);assert(wakes==1);
}
'''
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory)/'test.c'; source.write_text(code)
            binary = Path(directory)/'test'
            build = subprocess.run(['cc','-std=gnu11',str(source),'-o',str(binary)],capture_output=True,text=True)
            self.assertEqual(build.returncode,0,build.stderr)
            run = subprocess.run([str(binary)],capture_output=True,text=True)
            self.assertEqual(run.returncode,0,run.stderr)
