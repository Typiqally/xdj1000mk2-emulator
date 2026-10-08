"""Host arbitration remains fine-grained without polling the Mac clock per word."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / 'native/qemu/hw/cdj/boards/cdj2000/dsp_host.c'

class DspClockTests(unittest.TestCase):
    def test_live_pll_clock_changes_preserve_fraction_and_allow_gating(self):
        source = SOURCE.read_text()
        body = source[source.index('void cdj_dsp_host_clock('):source.index('c66x_core *cdj_dsp_host_core(')]
        code = r'''
#include <assert.h>
#include <stdint.h>
typedef struct CdjDspHost { void *core; uint64_t clock_hz,cycles_per_chunk,chunk_ns,cycle_fraction; } CdjDspHost;
''' + body + r'''
int main(void) {
    CdjDspHost h={.core=(void *)1,.chunk_ns=10000,.cycle_fraction=840000000};
    cdj_dsp_host_clock(&h,22579200);
    assert(h.clock_hz==22579200 && h.cycles_per_chunk==225 && h.cycle_fraction==840000000);
    cdj_dsp_host_clock(&h,451584000);
    assert(h.cycles_per_chunk==4515 && h.cycle_fraction==840000000);
    cdj_dsp_host_clock(&h,0);
    assert(!h.clock_hz && !h.cycles_per_chunk && h.cycle_fraction==840000000);
    h.core=0; cdj_dsp_host_clock(&h,451584000);assert(!h.cycle_fraction);
}
'''
        with tempfile.TemporaryDirectory() as d:
            src=Path(d)/'test.c';src.write_text(code);binary=Path(d)/'test'
            subprocess.run(['cc','-std=c11',str(src),'-o',str(binary)],check=True,capture_output=True)
            result=subprocess.run([str(binary)],capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stderr)

    def test_thread_clock_sampling_and_ahead_bound(self):
        source = SOURCE.read_text()
        body = source[source.index('static int64_t host_forgive_lag('):source.index('\n/*\n * MAIN waits')]
        code = r'''
#include <assert.h>
#include <stdbool.h>
#include <stdint.h>
#include <stddef.h>
#include <inttypes.h>
#define info_report(...) ((void)0)
#define QUANTUM_NS 1000000
#define CHUNK_NS 100000
#define QEMU_CLOCK_VIRTUAL 0
#define MAX(a,b) ((a)>(b)?(a):(b))
#define qatomic_read(p) (*(p))
#define qatomic_inc(p) (++*(p))
#define qatomic_add(p,v) (*(p)+=(v))
typedef unsigned c66x_stop;
#define C66X_STOP_IDLE 3
typedef struct CdjDspHost {
    struct { uint64_t thread_wait_us, step_us, worker_cpu_us, steps, advance_us, step_max_us, slow_steps; uint32_t step_max_pc; } perf;
    struct { unsigned enabled; } timeline;
    int64_t timeline_next_ns; unsigned timeline_freeze;
    bool running; unsigned host_waiting; int64_t dsp_ns, lag_max_ns;
    int64_t lag_cap_ns, lag_forgiven_ns; uint64_t lag_forgiven_events; bool lag_forgiving;
    uint64_t idle_steps, idle_skipped_cycles;
    unsigned lock, core, handoff; uint64_t cycles_per_chunk, chunk_ns;
    uint64_t clock_hz, cycle_fraction;
    void (*advance)(void *,uint64_t); void *chip;
    void (*observe)(void *); int64_t observe_next_us;
} CdjDspHost;
static int64_t g_get_monotonic_time(void) { return 0; }
static unsigned cpu_reads;
static uint64_t thread_cpu_us(void) { return ++cpu_reads; }
static void cdj_timeline_add(void *t,uint64_t host,uint64_t guest,unsigned kind,unsigned pc,uint64_t a,uint64_t b,unsigned freeze) {}
static uint32_t c66x_get_pc(unsigned core) { return 0; }
static int64_t perf_begin(CdjDspHost *h) { return 1; }
static void perf_elapsed(int64_t started, uint64_t *counter) { assert(started); }
static CdjDspHost *active;
static int64_t virtual_ns;
static unsigned reads, steps, sleeps, qos_calls, locks, main_handoff, yields, condition_waits;
static bool inject_main;
static uint64_t cycle_sum, skipped_sum, idle_after;
#define QOS_CLASS_USER_INTERACTIVE 0x21
static int pthread_set_qos_class_self_np(unsigned qos,int priority) {
    assert(qos==QOS_CLASS_USER_INTERACTIVE && priority==0); qos_calls++; return 0;
}
static int64_t qemu_clock_get_ns(unsigned clock) { reads++; return virtual_ns; }
static void g_usleep(unsigned us) { assert(us); sleeps++; virtual_ns+=us*1000; }
static void g_thread_yield(void) { yields++; active->host_waiting=0; }
static void qemu_mutex_lock(unsigned *p) { assert(!*p); *p=1; locks++; }
static void qemu_mutex_unlock(unsigned *p) {
    assert(*p); *p=0;
    if(active->host_waiting) {
        main_handoff=steps; active->host_waiting=0;
    }
}
static void qemu_cond_wait(unsigned *cond,unsigned *mutex) {
    assert(*mutex && active->host_waiting);condition_waits++;
    /* MAIN acquires the released mutex, finishes and signals the handoff. */
    active->host_waiting=0;
}
static bool host_stopped(CdjDspHost *h,c66x_stop stop) { return false; }
static c66x_stop c66x_step(unsigned core,uint64_t budget,uint64_t *done) {
    assert(active->lock);
    if (active->clock_hz == 451000000) assert(budget==4510);
    else assert(budget==4515 || budget==4516);
    cycle_sum += budget;
    assert(active->dsp_ns <= virtual_ns+QUANTUM_NS);
    *done=idle_after ? idle_after : budget;
    if(++steps==1000) active->running=false;
    return idle_after ? C66X_STOP_IDLE : 0;
}
static void c66x_skip_cycles(unsigned core,uint64_t n) {
    /* Only the cycles the idle core left of its step are skipped. */
    assert(active->lock && idle_after && n); skipped_sum += n;
}
static void advance(void *chip,uint64_t ns) {
    assert(ns==10000);
    if(inject_main && steps==5) active->host_waiting=1;
}
static unsigned observations;
static void observe(void *chip) { assert(active->lock); observations++; }
''' + body + r'''
int main(void) {
    CdjDspHost h={.running=true,.chunk_ns=10000,.cycles_per_chunk=4510,
                 .clock_hz=451000000,.advance=advance,.observe=observe};
    active=&h; virtual_ns=1000000000; inject_main=true; host_thread(&h);
    assert(steps==1000 && h.dsp_ns==10000000 && !sleeps);
    /* One initial publication and one final stopped-worker publication. */
    assert(observations==2 && h.observe_next_us==100000);
    /* Clock reads are costly on macOS. One per 100us is sufficient even
     * when releasing the DSP lock every 10us for native UHPI accesses. */
    assert(reads<=102);
    /* Retain the lock across fine-grained serializer steps when uncontended,
     * but release at the next 10us boundary for a waiting MAIN transaction. */
    assert(locks<=102 && main_handoff==5);
    /* Diagnostic CPU clocks are sampled at batch boundaries, not at every
     * serializer step. Profiling must not introduce two syscalls per step. */
    assert(cpu_reads<=2*locks && h.perf.worker_cpu_us==locks);
    inject_main=false;
#ifdef __APPLE__
    assert(qos_calls==1);
#endif
    h=(CdjDspHost){.running=true,.chunk_ns=10000,.cycles_per_chunk=4510,
                  .clock_hz=451000000,.advance=advance};
    active=&h; virtual_ns=0; reads=steps=sleeps=0; host_thread(&h);
    assert(steps==1000 && h.dsp_ns==10000000 && sleeps>0);
    assert(h.dsp_ns<=virtual_ns+QUANTUM_NS+10000);
    /* The board's 22.5792 MHz reference times PLLM+1=20 is 451.584 MHz.
     * Carry fractional cycles instead of rounding every 10us quantum. */
    h=(CdjDspHost){.running=true,.chunk_ns=10000,.cycles_per_chunk=4515,
                  .clock_hz=451584000,.advance=advance};
    active=&h; virtual_ns=1000000000; reads=steps=sleeps=0; cycle_sum=0;
    host_thread(&h);
    assert(cycle_sum==4515840 && h.cycle_fraction==0);
    /* MAIN hands the lock back without a scheduler sleep or busy-spin. */
    h=(CdjDspHost){.running=true,.host_waiting=1,.chunk_ns=10000,
                  .clock_hz=451584000,.advance=advance};
    active=&h; virtual_ns=1000000000; reads=steps=sleeps=yields=0;
    host_thread(&h); assert(condition_waits==1 && yields==0 && steps==1000 && !sleeps);
    /* Strict pacing repays all debt: a second behind, nothing is forgiven. */
    assert(!h.lag_forgiven_ns && !h.lag_forgiven_events && h.lag_max_ns>=990000000);
    /* A core idling at its polling loop executes part of each step; the
     * rest is skipped on its counters while DSP time and the peripherals
     * advance exactly as for a full step. */
    h=(CdjDspHost){.running=true,.chunk_ns=10000,.clock_hz=451584000,.advance=advance};
    active=&h; virtual_ns=1000000000; reads=steps=sleeps=0; cycle_sum=skipped_sum=0; idle_after=100;
    host_thread(&h);
    assert(steps==1000 && h.dsp_ns==10000000 && h.idle_steps==1000);
    assert(skipped_sum==cycle_sum-1000*100 && h.idle_skipped_cycles==skipped_sum && cycle_sum==4515840);
    idle_after=0;
    /* Interactive pacing: debt beyond the cap is forgiven at the next
     * arbitration boundary and only the capped remainder is repaid. */
    h=(CdjDspHost){.running=true,.chunk_ns=10000,.clock_hz=451584000,
                  .advance=advance,.lag_cap_ns=300000000};
    active=&h; virtual_ns=1000000000; reads=steps=sleeps=0; host_thread(&h);
    assert(h.lag_forgiven_ns==700000000 && h.lag_forgiven_events==1 && h.lag_forgiving);
    assert(steps==1000 && h.dsp_ns==710000000 && h.lag_max_ns==300000000 && !sleeps);
    assert(reads<=102 && locks);
    /* One burst is one event: hovering at the cap adds time, not events. */
    h.dsp_ns=0; assert(host_forgive_lag(&h,300000001)==1 && h.lag_forgiven_events==1);
    /* Recovered below half the cap, the next burst counts separately. */
    h.dsp_ns=0; assert(!host_forgive_lag(&h,100000000) && !h.lag_forgiving);
    h.dsp_ns=0; assert(host_forgive_lag(&h,400000000)==100000000);
    assert(h.dsp_ns==100000000 && h.lag_forgiven_events==2 && h.lag_forgiven_ns==800000001);
    return 0;
}
'''
        with tempfile.TemporaryDirectory() as d:
            src = Path(d)/'test.c'; src.write_text(code)
            binary = Path(d)/'test'
            subprocess.run(['cc','-std=c11','-O2',str(src),'-o',str(binary)],check=True,capture_output=True)
            result = subprocess.run([str(binary)],capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
