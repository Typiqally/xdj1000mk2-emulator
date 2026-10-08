/* SPDX-License-Identifier: GPL-2.0-or-later */
#include "dsp_host.h"
#include <time.h>
#ifdef __APPLE__
#include <pthread/qos.h>
#endif
/*
 * The core runs on its own thread in chunks, at most one quantum ahead of
 * QEMU's virtual clock. Run on the main loop instead, a chunk of the
 * interpreted core takes several times its own length in host time and every
 * QEMU timer, MAIN's tick among them, fires late and in bursts. When the host
 * cannot keep up the core lags; the worst lag is reported at exit.
 *
 * MAIN's host-port accesses and a chunk never run at the same time (@lock).
 * On the board the DSP follows the command pins within microseconds, so every
 * change of command runs the core in small pieces until the DSP has seen it:
 * for a command, until it answers on HINT; for the drop back to 0, until it
 * has read the pins again. Without the second, MAIN drops the command and
 * raises the next one before the loader, still waiting for the drop
 * (0x1180292C), has looked.
 */

#define QUANTUM_NS      (1000 * 1000)
#define CHUNK_NS        (100 * 1000)
#define REACT_STEP      2000
#define REACT_MAX       (4 * 1000 * 1000)

#define MCASP_GBLCTL    0x44
#define MCASP_RGBLCTL   0x60
#define MCASP_XGBLCTL   0xA0
#define GBLCTL_R        0x001Fu
#define GBLCTL_X        0x1F00u

static int64_t perf_begin(CdjDspHost *h)
{
    return qatomic_read(&h->perf.enabled) ? g_get_monotonic_time() : 0;
}

static void perf_elapsed(int64_t started, uint64_t *counter)
{
    if (started) { qatomic_add(counter, g_get_monotonic_time() - started); }
}

static uint64_t thread_cpu_us(void)
{
#ifdef CLOCK_THREAD_CPUTIME_ID
    struct timespec value;
    if (!clock_gettime(CLOCK_THREAD_CPUTIME_ID, &value)) {
        return (uint64_t)value.tv_sec * 1000000 + value.tv_nsec / 1000;
    }
#endif
    return 0;
}

static bool host_stopped(CdjDspHost *h, c66x_stop stop)
{
    h->last_stop = stop;
    if (stop != C66X_STOP_UNDEF && stop != C66X_STOP_FAULT) {
        return false;
    }
    h->trap_pc = c66x_trap_pc(h->core);
    h->running = false;
    warn_report("%s: core stopped (%s) at 0x%08x", h->name,
                stop == C66X_STOP_UNDEF ? "undefined instruction" : "fault",
                h->trap_pc);
    return true;
}

/* Called under @lock with a recent virtual clock. Returns the time forgiven
 * so the caller's cached deadlines move with dsp_ns. */
static int64_t host_forgive_lag(CdjDspHost *h, int64_t virt)
{
    int64_t lag = virt - h->dsp_ns;

    if (!h->lag_cap_ns) {
        return 0;
    }
    if (lag <= h->lag_cap_ns) {
        /* One episode per burst: hysteresis keeps a core hovering at the
         * cap from counting every arbitration interval as a new loss. */
        if (h->lag_forgiving && lag < h->lag_cap_ns / 2) {
            h->lag_forgiving = false;
        }
        return 0;
    }
    lag -= h->lag_cap_ns;
    qatomic_add(&h->dsp_ns, lag);
    h->lag_forgiven_ns += lag;
    if (!h->lag_forgiving) {
        h->lag_forgiving = true;
        h->lag_forgiven_events++;
    }
    return lag;
}

static void *host_thread(void *opaque)
{
    CdjDspHost *h = opaque;
    uint64_t done;
    int64_t virt = 0, sample_at = h->dsp_ns;
#ifdef __APPLE__
    /* Native DSP work supplies interactive audio and control responses. */
    pthread_set_qos_class_self_np(QOS_CLASS_USER_INTERACTIVE, 0);
#endif

    while (qatomic_read(&h->running)) {
        /* Fine-grained arbitration need not query the host clock each time.
         * A cached clock only tightens the ahead bound; refresh immediately
         * when reaching it, including while QEMU's virtual clock is paused. */
        if (h->dsp_ns >= sample_at || h->dsp_ns >= virt + QUANTUM_NS) {
            virt = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
            sample_at = h->dsp_ns + CHUNK_NS;
        }

        if (qatomic_read(&h->host_waiting)) {
            /* On macOS a scheduler yield can become a timed sleep. Wait
             * for MAIN's actual unlock instead: the predicate and signal
             * share the mutex, so an early handoff cannot lose a wakeup. */
            qemu_mutex_lock(&h->lock);
            while (qatomic_read(&h->host_waiting)) {
                qemu_cond_wait(&h->handoff, &h->lock);
            }
            qemu_mutex_unlock(&h->lock);
            continue;
        }
        if (h->dsp_ns >= virt + QUANTUM_NS) {
            g_usleep(1000);
            continue;
        }
        done = 0;
        int64_t timing = perf_begin(h);
        qemu_mutex_lock(&h->lock);
        perf_elapsed(timing, &h->perf.thread_wait_us);
        uint64_t cpu_at = timing ? thread_cpu_us() : 0;
        sample_at += host_forgive_lag(h, virt);
        do {
            h->lag_max_ns = MAX(h->lag_max_ns, virt - h->dsp_ns);
            uint64_t scaled_cycles = h->clock_hz * h->chunk_ns + h->cycle_fraction;
            uint64_t budget = scaled_cycles / UINT64_C(1000000000);
            h->cycle_fraction = scaled_cycles % UINT64_C(1000000000);
            timing = perf_begin(h);
            uint32_t pc = timing ? c66x_get_pc(h->core) : 0;
            c66x_stop stop = c66x_step(h->core, budget, &done);
            host_stopped(h, stop);
            if (stop == C66X_STOP_IDLE && done < budget) {
                /* The polling loop is a fixed point until an interrupt or a
                 * host store it reads: the rest of this step would only spin.
                 * The cycles still pass, on the time-stamp counter and for
                 * the peripherals advanced below. */
                c66x_skip_cycles(h->core, budget - done);
                h->idle_steps++;
                h->idle_skipped_cycles += budget - done;
            }
            if (timing) {
                uint64_t elapsed = g_get_monotonic_time() - timing;
                h->perf.step_us += elapsed;
                if (elapsed > h->perf.step_max_us) {
                    h->perf.step_max_us = elapsed;
                    h->perf.step_max_pc = pc;
                }
                if (elapsed > 5000) {
                    h->perf.slow_steps++;
                }
                if (h->timeline.enabled && (h->dsp_ns >= h->timeline_next_ns || elapsed > 5000 || qatomic_read(&h->timeline_freeze))) {
                    cdj_timeline_add(&h->timeline,timing,h->dsp_ns,elapsed>5000?2:1,
                        pc,elapsed,virt > h->dsp_ns ? virt-h->dsp_ns : 0,qatomic_read(&h->timeline_freeze));
                    h->timeline_next_ns = h->dsp_ns + 1000000;
                }
            }
            if (timing) { qatomic_inc(&h->perf.steps); }
            timing = perf_begin(h);
            if (h->advance && h->running) { h->advance(h->chip, h->chunk_ns); }
            perf_elapsed(timing, &h->perf.advance_us);
            /* Keep serializer/DMA arbitration fine-grained. An uncontended
             * mutex need not be handed back to the OS at every such step. */
            qatomic_add(&h->dsp_ns,h->chunk_ns);
        } while (qatomic_read(&h->running) && !qatomic_read(&h->host_waiting)
                 && h->dsp_ns < sample_at && h->dsp_ns < virt + QUANTUM_NS);
        if (cpu_at) { h->perf.worker_cpu_us += thread_cpu_us() - cpu_at; }
        if (h->observe) {
            int64_t now = g_get_monotonic_time();
            if (now >= h->observe_next_us || !h->running) {
                h->observe(h->chip);
                h->observe_next_us = now + 100000;
            }
        }
        qemu_mutex_unlock(&h->lock);
    }
    return NULL;
}

/*
 * MAIN waits for a chunk to end, and then may run the core itself
 * (host_react), for milliseconds of host time. Holding the BQL all that
 * while stalls the main loop and every QEMU timer with it: MAIN's 1 kHz
 * tick underflowed in bursts of four and the RTOS counted half its ticks.
 * Nothing behind @lock needs the BQL, so MAIN's side lets it go.
 */
void cdj_dsp_host_lock(CdjDspHost *h)
{
    bool had_bql = bql_locked();

    if (had_bql) {
        bql_unlock();
    }
    qatomic_inc(&h->host_waiting);
    int64_t timing = perf_begin(h);
    qemu_mutex_lock(&h->lock);
    perf_elapsed(timing, &h->perf.host_wait_us);
    if (h->timeline.enabled) {
        int64_t now = g_get_monotonic_time();
        cdj_timeline_add(&h->timeline,now,h->dsp_ns,3,
            h->core ? c66x_get_pc(h->core) : 0,timing ? now-timing : 0,
            h->command,0);
    }
    if (timing) { qatomic_inc(&h->perf.host_calls); }
    qatomic_dec(&h->host_waiting);
    h->host_had_bql = had_bql;
}

void cdj_dsp_host_unlock(CdjDspHost *h)
{
    bool had_bql = h->host_had_bql;

    qemu_cond_signal(&h->handoff);
    qemu_mutex_unlock(&h->lock);
    if (had_bql) {
        int64_t timing = perf_begin(h);
        bql_lock();
        perf_elapsed(timing, &h->perf.bql_wait_us);
    }
}

static bool host_answered(CdjDspHost *h, uint64_t edges, uint64_t reads)
{
    return h->command ? h->hint_edges != edges : h->pin_reads != reads;
}

static void host_elapsed_cycles(CdjDspHost *h, uint64_t cycles, uint64_t hz)
{
    if (!cycles || !hz) { return; }
    /* Keep the remainder in clock-independent Q32 nanoseconds: a later PLL
     * change must not reinterpret a remainder measured against the old Hz. */
    __uint128_t scaled = ((__uint128_t)cycles * 1000000000 << 32) / hz;
    scaled += h->react_fraction;
    uint64_t ns = scaled >> 32;
    h->react_fraction = (uint32_t)scaled;
    qatomic_add(&h->dsp_ns,ns);
    if (ns && h->advance) { h->advance(h->chip, ns); }
}

static void host_react(CdjDspHost *h)
{
    uint64_t edges = h->hint_edges, reads = h->pin_reads, total = 0, done;
    int64_t timing = perf_begin(h);

    /* The command pins are an input no store or interrupt announces: a loop
     * idling at a fixed point has to run to see them. */
    if (h->core) {
        c66x_idle_wake(h->core);
    }

    while (h->running && h->clock_hz && !host_answered(h, edges, reads) && total < REACT_MAX) {
        done = 0;
        uint64_t hz = h->clock_hz;
        bool stopped = host_stopped(h, c66x_step(h->core, REACT_STEP, &done));
        host_elapsed_cycles(h, done, hz);
        total += done;
        /* A genuinely idle or stopped core cannot answer by repeatedly
         * requesting cycles that it did not execute. */
        if (stopped || !done) { break; }
    }
    h->react_cycles += total;
    perf_elapsed(timing, &h->perf.react_us);
}

static void set_hpic(CdjDspHost *h, uint32_t hpic)
{
    if ((hpic ^ h->hpic) & HPIC_HINT) {
        h->hint_edges++;
    }
    qatomic_set(&h->hpic, hpic);
}

/* The HPIC as either side reads it; the port is always ready. */
uint32_t cdj_dsp_host_hpic(const CdjDspHost *h)
{
    return qatomic_read(&h->hpic) | HPIC_HRDY;
}

bool cdj_dsp_host_main_hpic(CdjDspHost *h, uint32_t val)
{
    set_hpic(h, (h->hpic & ~(val & HPIC_HINT)) | (val & HPIC_DSPINT)
                | (val & HPIC_HWOB));
    return val & HPIC_DSPINT;
}

void cdj_dsp_host_dsp_hpic(CdjDspHost *h, uint32_t val)
{
    set_hpic(h, (h->hpic & ~(val & HPIC_DSPINT)) | (val & HPIC_HINT));
}

/* The HINT pin as MAIN's latch sees it: high (busy) until the DSP raises
 * HINT. */
static bool host_busy(void *opaque)
{
    CdjDspHost *h = opaque;

    return !(qatomic_read(&h->hpic) & HPIC_HINT);
}

static void host_command(void *opaque, unsigned bits)
{
    CdjDspHost *h = opaque;

    /* MAIN rewrites the latch for its other bits too. */
    if (bits == h->command) {
        return;
    }
    cdj_dsp_host_lock(h);
    h->command = bits;
    h->set_pins(h->chip, bits);
    if (bits) {
        h->commands++;
    }
    host_react(h);
    if (h->timeline.enabled) {
        cdj_timeline_add(&h->timeline,g_get_monotonic_time(),h->dsp_ns,4,
            h->core ? c66x_get_pc(h->core) : 0,bits,
            qatomic_read(&h->hpic),0);
    }
    cdj_dsp_host_unlock(h);
}

void cdj_dsp_host_init(CdjDspHost *h, const char *name, const char *env_prefix,
                       unsigned default_mhz,
                       void (*set_pins)(void *chip, unsigned bits), void *chip)
{
    g_autofree char *on_var = g_strdup_printf("CDJ_%s", env_prefix);
    g_autofree char *mhz_var = g_strdup_printf("CDJ_%s_MHZ", env_prefix);
    const char *on = getenv(on_var);
    const char *mhz = getenv(mhz_var);

    h->name = name;
    h->enabled = !(on && !strcmp(on, "0"));
    h->chunk_ns = CHUNK_NS;
    h->clock_hz = (mhz ? strtoull(mhz, NULL, 0) : default_mhz) * 1000000;
    h->cycles_per_chunk = h->clock_hz * h->chunk_ns / UINT64_C(1000000000);
    qemu_mutex_init(&h->lock);
    qemu_cond_init(&h->handoff);
    /* The chip's ROM boot loader raises HINT at reset to tell the host it
     * is ready; MAIN waits for that before it loads anything. */
    h->hpic = HPIC_HINT;
    h->set_pins = set_pins;
    h->chip = chip;
    h->wires = (CdjDspWires){ h, host_command, host_busy };
}

void cdj_dsp_host_quantum(CdjDspHost *h, uint64_t ns)
{
    assert(!h->core && ns && ns <= CHUNK_NS);
    h->cycles_per_chunk = h->cycles_per_chunk * ns / h->chunk_ns;
    assert(h->cycles_per_chunk);
    h->chunk_ns = ns;
}

void cdj_dsp_host_lag_cap(CdjDspHost *h, int64_t ns)
{
    /* Below one scheduling quantum the cap would forgive ordinary jitter. */
    assert(!h->core && (!ns || ns >= QUANTUM_NS));
    h->lag_cap_ns = ns;
}

void cdj_dsp_host_clock(CdjDspHost *h, uint64_t hz)
{
    /* A live caller holds the DSP lock. PLL transitions and clock gating
     * change future budgets without discarding already accrued fractions. */
    assert(hz <= UINT64_C(1000000000));
    h->clock_hz = hz;
    h->cycles_per_chunk = hz * h->chunk_ns / UINT64_C(1000000000);
    if (!h->core) {
        h->cycle_fraction = 0;
    }
}

c66x_core *cdj_dsp_host_core(CdjDspHost *h, const c66x_bus *bus)
{
    h->core = h->c674x_only ? c66x_new_c674x(bus) : c66x_new(bus);
    return h->core;
}

void cdj_dsp_host_run(CdjDspHost *h, uint32_t entry)
{
    c66x_reset(h->core, entry);
    h->running = true;
    qatomic_set(&h->dsp_ns,qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL));
    info_report("%s: started at 0x%08x", h->name, entry);
    qemu_thread_create(&h->thread, h->name, host_thread, h,
                       QEMU_THREAD_DETACHED);
}

/* The GBLCTL register @addr is in, or -1; its McASP in @n. */
static int mcasp_reg(const DspMcasps *m, uint32_t addr, unsigned *n)
{
    uint32_t off = (addr - m->base) % m->stride;

    *n = (addr - m->base) / m->stride;
    if (addr < m->base || *n >= ARRAY_SIZE(m->gblctl)) {
        return -1;
    }
    return off == MCASP_GBLCTL || off == MCASP_RGBLCTL
        || off == MCASP_XGBLCTL ? off : -1;
}

uint32_t cdj_dsp_mcasp_read(const DspMcasps *m, uint32_t addr)
{
    unsigned n;

    return mcasp_reg(m, addr, &n) < 0 ? 0 : m->gblctl[n];
}

void cdj_dsp_mcasp_write(DspMcasps *m, uint32_t addr, uint32_t val)
{
    unsigned n;
    uint32_t mask;

    switch (mcasp_reg(m, addr, &n)) {
    case MCASP_RGBLCTL:
        mask = GBLCTL_R;
        break;
    case MCASP_XGBLCTL:
        mask = GBLCTL_X;
        break;
    case MCASP_GBLCTL:
        mask = GBLCTL_R | GBLCTL_X;
        break;
    default:
        return;
    }
    m->gblctl[n] = (m->gblctl[n] & ~mask) | (val & mask);
}

void cdj_dsp_dump_mem(const char *dir, const char *name, const void *p,
                      size_t len)
{
    g_autofree char *path = g_strdup_printf("%s/%s", dir, name);
    g_autoptr(GError) err = NULL;

    if (!g_file_set_contents(path, p, len, &err)) {
        warn_report("dsp: cannot write %s: %s", path, err->message);
    }
}

void cdj_dsp_count(DspAddrTable *t, uint32_t addr, uint32_t val)
{
    unsigned i;

    for (i = 0; i < t->n; i++) {
        if (t->e[i].addr == addr) {
            t->e[i].count++;
            t->e[i].last = val;
            return;
        }
    }
    if (t->n < DSP_HOST_MAX_ADDR) {
        t->e[t->n++] = (DspAddrCount){ addr, 1, val };
    }
}

static void print_counts(const char *name, const char *what,
                         const DspAddrTable *t)
{
    unsigned i;

    for (i = 0; i < t->n; i++) {
        info_report("%s: %s 0x%08x x%" PRIu64 ", last 0x%08x", name, what,
                    t->e[i].addr, t->e[i].count, t->e[i].last);
    }
}

void cdj_dsp_host_report(CdjDspHost *h)
{
    c66x_stats st;

    if (!h->core) {
        info_report("%s: never started", h->name);
        return;
    }
    cdj_dsp_host_lock(h);
    c66x_get_stats(h->core, &st);
    info_report("%s: %s, pc 0x%08x, %" PRIu64 " cycles, %" PRIu64
                " packets, last stop %d", h->name,
                h->running ? "running" : "stopped", c66x_get_pc(h->core),
                st.cycles, st.packets, h->last_stop);
    if (h->trap_pc) {
        info_report("%s: trap at 0x%08x", h->name, h->trap_pc);
    }
    info_report("%s: %" PRIu64 " commands from MAIN, %" PRIu64
                " HINT edges, HPIC 0x%08x, %" PRIu64 " cycles run to answer",
                h->name, h->commands, h->hint_edges, h->hpic, h->react_cycles);
    info_report("%s: at most %" PRId64 " ms behind the virtual clock",
                h->name, h->lag_max_ns / SCALE_MS);
    if (h->idle_steps) {
        int confirmed = 0;
        uint32_t head = c66x_idle_head(h->core, &confirmed);
        info_report("%s: polling loop at 0x%08x: %" PRIu64 " idle steps, %" PRIu64
                    " cycles skipped of %" PRIu64, h->name, head, h->idle_steps,
                    h->idle_skipped_cycles, st.cycles);
    }
    if (h->lag_cap_ns) {
        info_report("%s: lag capped at %" PRId64 " ms; %" PRId64 " ms forgiven "
                    "in %" PRIu64 " bursts", h->name, h->lag_cap_ns / SCALE_MS,
                    h->lag_forgiven_ns / SCALE_MS, h->lag_forgiven_events);
    }
    info_report("%s: current clock offset %" PRId64 " us, DSP time %" PRId64 " ms",
                h->name, (qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) - h->dsp_ns) / 1000,
                h->dsp_ns / SCALE_MS);
    print_counts(h->name, "bus read ", &h->busr);
    print_counts(h->name, "bus write", &h->busw);
    cdj_dsp_host_unlock(h);
}
