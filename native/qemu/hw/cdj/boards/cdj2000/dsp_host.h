/* SPDX-License-Identifier: GPL-2.0-or-later */
/*
 * What the CDJ-2000's and the CDJ-2000NXS's DSPs have in common as MAIN sees
 * them: a TI C67x-family chip booted through its host port (UHPI), with the
 * same handshake on both boards --
 *   MAIN drives a 2-bit command on two DSP pins from its port latch;
 *   the DSP's loader answers each command by raising HINT in its HPIC;
 *   the HINT pin (active low) comes back to MAIN as the latch's busy input,
 *   and MAIN clears HINT through its own view of the HPIC.
 * The chip files (dsp_c6747.c, dsp_c6727.c) supply the memory map, the host
 * window and which pins carry the command; this runs the core, on its own
 * thread. What the core and MAIN's host port share is under @lock, which the
 * chip takes in its host-port handlers (CDJ_DSP_HOST_GUARD).
 */
#ifndef CDJ2000_DSP_HOST_H
#define CDJ2000_DSP_HOST_H
#include "sh7763.h"
#include "qemu/timer.h"
#include "qemu/thread.h"
#include "c6x/c66x.h"
#include "dsp_timeline.h"

#define HPIC_HWOB       (1u << 0)
#define HPIC_DSPINT     (1u << 1)
#define HPIC_HINT       (1u << 2)
#define HPIC_HRDY       (1u << 3)

#define DSP_HOST_MAX_ADDR 128

/* Both images send control words to a converter on SPI1 and nothing comes
 * back: after each write to SPIDAT1 they wait for SPIBUF to show the
 * transmitter free and the receive buffer empty (C6747 image B 0xC004F3C4). */
#define SPIBUF_RXEMPTY  (1u << 31)

typedef struct DspAddrCount {
    uint32_t addr;
    uint64_t count;
    uint32_t last;
} DspAddrCount;

typedef struct DspAddrTable {
    DspAddrCount e[DSP_HOST_MAX_ADDR];
    unsigned n;
} DspAddrTable;

/* The three McASPs' global control: both chips bring each one up by
 * setting reset bits through RGBLCTL/XGBLCTL and polling GBLCTL until they
 * read back (C6747 image B 0xC004E700, C6727 image B on 0x45000044). */
typedef struct DspMcasps {
    uint32_t base, stride;
    uint32_t gblctl[3];
} DspMcasps;

typedef struct CdjDspHost {
    const char *name;
    c66x_core *core;
    bool enabled, running;
    bool c674x_only;
    c66x_stop last_stop;
    uint32_t trap_pc;
    uint64_t cycles_per_chunk;
    uint64_t clock_hz, cycle_fraction;
    uint64_t react_fraction;    /* fractional nanoseconds, Q32, across commands */
    uint64_t chunk_ns;          /* maximum execution quantum while holding the host port */
    QemuMutex lock;
    QemuCond handoff;           /* MAIN has released the DSP port */
    unsigned host_waiting;      /* MAIN waits for @lock */
    bool host_had_bql;          /* MAIN's holder of @lock let the BQL go */
    QemuThread thread;
    int64_t dsp_ns;             /* the virtual time the core has run up to */
    int64_t lag_max_ns;
    /* Interactive pacing. A host that cannot execute a DSP burst in real
     * time leaves the core behind the virtual clock; repaying all of that
     * debt runs it faster than real time for as long as the burst lasted.
     * With a cap, debt beyond it is forgiven instead: the core loses that
     * time, as a clock-gated chip would, and each loss is counted. 0 keeps
     * the strict behaviour and repays everything. */
    /* Cycles the core did not have to execute: its polling loop was at a
     * fixed point (c66x_set_idle_loop) for the rest of an arbitration step. */
    uint64_t idle_steps, idle_skipped_cycles;
    int64_t lag_cap_ns;
    int64_t lag_forgiven_ns;
    uint64_t lag_forgiven_events;
    bool lag_forgiving;
    CdjTimeline timeline;
    int64_t timeline_next_ns;
    unsigned timeline_freeze;   /* atomic request from the PCM consumer */

    uint32_t hpic;
    unsigned command;
    uint64_t commands, hint_edges, react_cycles;
    uint64_t pin_reads;         /* DSP reads of the command pins */
    /* The chip drives its command pins from MAIN's 2-bit command. */
    void (*set_pins)(void *chip, unsigned bits);
    void *chip;
    void (*advance)(void *chip, uint64_t ns);
    /* Optional telemetry publication under the DSP lock, at most 10 Hz.
     * Readers consume the chip snapshot without taking this host port. */
    void (*observe)(void *chip);
    int64_t observe_next_us;
    CdjDspWires wires;

    DspAddrTable busr, busw;
    /* Opt-in host timings; never part of DSP instruction state or cache ABI. */
    struct {
        bool enabled;
        uint64_t steps, step_us, worker_cpu_us, advance_us, thread_wait_us;
        uint64_t host_calls, host_wait_us, bql_wait_us, react_us;
        uint64_t step_max_us, slow_steps;
        uint32_t step_max_pc;
    } perf;
} CdjDspHost;

/* CDJ_<PREFIX>=0 turns the core off; CDJ_<PREFIX>_MHZ sets its clock. */
void cdj_dsp_host_init(CdjDspHost *h, const char *name, const char *env_prefix,
                       unsigned default_mhz,
                       void (*set_pins)(void *chip, unsigned bits), void *chip);
/* A core on @bus with nothing mapped yet; the chip maps its RAM, then
 * cdj_dsp_host_run() starts it at @entry. */
/* Select a smaller host-port arbitration quantum before starting the core. */
void cdj_dsp_host_quantum(CdjDspHost *h, uint64_t ns);
/* Bound the debt the core repays; see lag_cap_ns. Before starting the core. */
void cdj_dsp_host_lag_cap(CdjDspHost *h, int64_t ns);
void cdj_dsp_host_clock(CdjDspHost *h, uint64_t hz);

c66x_core *cdj_dsp_host_core(CdjDspHost *h, const c66x_bus *bus);
void cdj_dsp_host_run(CdjDspHost *h, uint32_t entry);

/* MAIN's side of @lock, for the chip's host-port handlers. The core's
 * thread steps aside while MAIN waits: a mutex alone lets it take the lock
 * straight back, chunk after chunk, and MAIN starves. */
void cdj_dsp_host_lock(CdjDspHost *h);
void cdj_dsp_host_unlock(CdjDspHost *h);
G_DEFINE_AUTOPTR_CLEANUP_FUNC(CdjDspHost, cdj_dsp_host_unlock)
#define CDJ_DSP_HOST_GUARD(h) \
    g_autoptr(CdjDspHost) host_guard_ = (cdj_dsp_host_lock(h), (h))

uint32_t cdj_dsp_host_hpic(const CdjDspHost *h);
/* The HPIC as MAIN writes it (clears HINT, raises DSPINT) and as the DSP
 * writes its own (clears DSPINT, raises HINT). Returns true when MAIN raised
 * DSPINT. */
bool cdj_dsp_host_main_hpic(CdjDspHost *h, uint32_t val);
void cdj_dsp_host_dsp_hpic(CdjDspHost *h, uint32_t val);

/* GBLCTL, RGBLCTL and XGBLCTL of McASP n; 0 for any other address. */
uint32_t cdj_dsp_mcasp_read(const DspMcasps *m, uint32_t addr);
void cdj_dsp_mcasp_write(DspMcasps *m, uint32_t addr, uint32_t val);

/* Save @len bytes of DSP memory as <dir>/<name>. */
void cdj_dsp_dump_mem(const char *dir, const char *name, const void *p,
                      size_t len);
void cdj_dsp_count(DspAddrTable *t, uint32_t addr, uint32_t val);
void cdj_dsp_host_report(CdjDspHost *h);

#endif
