/* SPDX-License-Identifier: GPL-2.0-or-later */
#include "../cdj2000/dsp_host.h"
#include "cdj_getenv.h"
#include "da810_intc.h"
#include "da810_edma.h"
#include "da810_mcasp.h"
#include "da810_pll.h"
#include "da810_timer.h"
#include "da810_syscfg.h"
#include "dsp_memory.h"
#include "hardware.h"
#include "audio/audio.h"
/*
 * The XDJ-1000MK2 DSP: a DA810 with the C674x core that MAIN boots and feeds
 * through its host port (UHPI). The C674x instruction set is a subset of the
 * C66x's, so the project's C66x core runs it; this file is the chip around
 * that core as far as the boot needs it.
 *
 * MAIN's side, its area 3: four 32-bit registers 256 KiB apart --
 *   +0x00000 HPIC  control (HWOB, DSPINT, HINT, HRDY)
 *   +0x40000 HPIA  the DSP address the next data access goes to
 *   +0x80000 HPID  data, HPIA advancing by 4 after each access
 *   +0xC0000 HPID  data, HPIA left alone
 * The HPI bus is 16 bits wide, so each 32-bit access is two half-word cycles
 * and a register write carries the same half-word twice (HPIC 0x00010001).
 *
 * The boot, as the firmware does it: MAIN writes the first image into L2 at
 * 0x11801DA0 and its entry into the boot word at 0x11800000, then sets
 * DSPINT, which starts the DSP there. That image is a resident loader: it
 * clears a mailbox at 0x1183FFF0/F4, then takes the second image in 32 KiB
 * chunks through a window at 0x11837800, copying each to SDRAM at 0xC0000000.
 * The loader takes its orders on two GPIO pins (bank 4/5 bits 5 and 2, read
 * by 0x118027EC) and answers each by raising HINT (0x11802918); it never
 * writes a GPIO output. The handshake itself is in dsp_host.c.
 *
 * Unmodelled DSP bus addresses read 0; the busiest are listed at exit.
 *
 *   CDJ_DA810_MHZ=<n>       diagnostic override of the firmware-programmed clock
 *   CDJ_C6747=0             no core: the host port is plain memory
 *   CDJ_HPI_DUMP=<dir>      at exit, save each run of host-port writes as
 *                           <dir>/hpi_<addr>.bin, and L2 and the first MiB
 *                           of SDRAM as <dir>/l2.bin and <dir>/sdram.bin
 */

#define L2_BASE         XDJ_DSP_L2_BASE
#define L2_ALIAS        XDJ_DSP_L2_ALIAS
#define L2_SIZE         XDJ_DSP_L2_SIZE
#define L1P_BASE        XDJ_DSP_L1P_BASE
#define L1D_BASE        XDJ_DSP_L1D_BASE
#define L1_SIZE         XDJ_DSP_L1P_SIZE
#define SHRAM_BASE      XDJ_DSP_SHARED_BASE
#define SHRAM_SIZE      XDJ_DSP_SHARED_SIZE
#define SDRAM_BASE      XDJ_DSP_SDRAM_BASE
#define SDRAM_SIZE      XDJ_DSP_SDRAM_SIZE

#define BOOT_WORD       0x11800000u
#define UHPI_BASE       0x01E10000u
#define UHPI_HPIC       (UHPI_BASE + 0x30)

/* GPIO: five bank pairs, 0x28 apart from +0x10. */
#define GPIO_BASE       0x01E26000u
#define GPIO_PAIRS      5
#define GPIO_DIR        0x00            /* 1 = input */
#define GPIO_OUT        0x04
#define GPIO_SET        0x08
#define GPIO_CLR        0x0C
#define GPIO_IN         0x10
#define GPIO_CMD_PAIR   2               /* banks 4/5 */
#define GPIO_CMD_BIT0   (1u << 5)       /* MAIN's command bit 0 */
#define GPIO_CMD_BIT1   (1u << 2)       /* MAIN's command bit 1 */

#define MCASP_BASE      0x01D00000u
#define MCASP_STRIDE    0x4000

#define SPI1_SPIBUF     0x01E12040u

#define MAX_RUNS        64
#define MAX_POLLS       16

/*
 * The DAC queue is the only elastic buffer between the DSP, which the host
 * executes in bursts, and an output device that consumes at a constant rate.
 * It must hold the cushion that covers a burst and the frames a catch-up adds
 * on top of it: a queue smaller than one burst overflows on every catch-up,
 * and discarding to near-empty guarantees the next burst underruns again.
 * After an overflow the newest PCM_RECOVER_FRAMES stay queued, so recovery
 * keeps the cushion. The largest debt the DSP repays (CDJ_XDJ_DSP_LAG_CAP_MS)
 * must fit between the two.
 */
#define PCM_QUEUE_FRAMES    44100u
#define PCM_RECOVER_FRAMES  (PCM_QUEUE_FRAMES * 3 / 10)
#define PCM_FRAME_BYTES     8u
#define DSP_LAG_CAP_MAX_MS  ((PCM_QUEUE_FRAMES - PCM_RECOVER_FRAMES) * 1000 / 44100)

typedef struct HpiRun {
    uint32_t start, end;        /* [start, end) in DSP bytes */
} HpiRun;

typedef struct CdjC6747 {
    MemoryRegion iomem;
    uint8_t *l2, *l1p, *l1d, *shram, *sdram;
    CdjDspHost host;
    Da810Pll pll;
    Da810Syscfg syscfg;
    Da810Timer timer[2];
    bool clock_override;

    uint32_t hpia;
    uint64_t writes, reads, dspints, dropped;
    HpiRun runs[MAX_RUNS];
    unsigned nruns;
    bool runs_overflowed;
    DspAddrTable polls;

    uint32_t gpio_dir[GPIO_PAIRS], gpio_out[GPIO_PAIRS], gpio_ext[GPIO_PAIRS];
    Da810Mcasp mcasp[3];
    QEMUSoundCard card;
    SWVoiceOut *voice;
    bool audio_muted;
    CdjTimeline pcm_timeline;
    uint64_t unmodeled_reads, unmodeled_writes;
    struct { uint32_t addr, pc, value; unsigned size, write; } first_gap;
    /* PCM delivery must not wait for DSP execution or adaptive compilation.
     * Lock order is DSP -> PCM; output takes only PCM and releases it before
     * entering the QEMU audio core (BQL). */
    QemuMutex pcm_lock;
    /* Published while holding DSP -> PCM; readers only take live_lock. */
    QemuMutex live_lock;
    char *live_audio, *live_performance;
    int64_t live_sample_us;
    uint8_t pcm[PCM_QUEUE_FRAMES * PCM_FRAME_BYTES];
    unsigned pcm_head, pcm_length;
    bool pcm_reading, pcm_resync_pending;
    int32_t left_sample;
    uint64_t pcm_frames, pcm_overflows, pcm_nonzero, pcm_peak;
    uint64_t pcm_consumed_frames;
    uint64_t pcm_resyncs;
    int64_t pcm_last_overflow_ns;
    FILE *pcm_capture, *pcm_consumed;
    /* The polling-loop head the core settled on, kept beside the compiled
     * code built for it so the next run starts with both. */
    char *idle_hint_path;
    uint32_t idle_saved;
    Da810Intc intc;
    Da810Edma edma;

    Notifier exit;
} CdjC6747;

static CdjC6747 c6747;

/* The host pointer behind a DSP address, NULL for anything that is not RAM. */
static uint8_t *dsp_ram(CdjC6747 *s, uint32_t addr)
{
    if (addr - L2_BASE < L2_SIZE) {
        return s->l2 + (addr - L2_BASE);
    }
    if (addr - L2_ALIAS < L2_SIZE) {
        return s->l2 + (addr - L2_ALIAS);
    }
    if (addr - SDRAM_BASE < SDRAM_SIZE) {
        return s->sdram + (addr - SDRAM_BASE);
    }
    if (addr - SHRAM_BASE < SHRAM_SIZE) {
        return s->shram + (addr - SHRAM_BASE);
    }
    if (addr - L1P_BASE < L1_SIZE) {
        return s->l1p + (addr - L1P_BASE);
    }
    if (addr - XDJ_DSP_L1P_ALIAS < L1_SIZE) {
        return s->l1p + (addr - XDJ_DSP_L1P_ALIAS);
    }
    if (addr - L1D_BASE < L1_SIZE) {
        return s->l1d + (addr - L1D_BASE);
    }
    if (addr - XDJ_DSP_L1D_ALIAS < L1_SIZE) {
        return s->l1d + (addr - XDJ_DSP_L1D_ALIAS);
    }
    return NULL;
}

static bool gpio_reg(uint32_t addr, unsigned *pair, unsigned *reg)
{
    uint32_t off = addr - GPIO_BASE - 0x10;

    if (addr < GPIO_BASE + 0x10 || off >= GPIO_PAIRS * 0x28) {
        return false;
    }
    *pair = off / 0x28;
    *reg = off % 0x28;
    return true;
}

static uint32_t gpio_in(CdjC6747 *s, unsigned pair)
{
    return (s->gpio_ext[pair] & s->gpio_dir[pair])
         | (s->gpio_out[pair] & ~s->gpio_dir[pair]);
}

static void gpio_write(CdjC6747 *s, unsigned pair, unsigned reg, uint32_t val)
{
    switch (reg) {
    case GPIO_DIR:
        s->gpio_dir[pair] = val;
        break;
    case GPIO_OUT:
        s->gpio_out[pair] = val;
        break;
    case GPIO_SET:
        s->gpio_out[pair] |= val;
        break;
    case GPIO_CLR:
        s->gpio_out[pair] &= ~val;
        break;
    }
}

static void set_command_pins(void *opaque, unsigned bits)
{
    CdjC6747 *s = opaque;
    uint32_t *ext = &s->gpio_ext[GPIO_CMD_PAIR];

    *ext &= ~(GPIO_CMD_BIT0 | GPIO_CMD_BIT1);
    *ext |= (bits & 1 ? GPIO_CMD_BIT0 : 0) | (bits & 2 ? GPIO_CMD_BIT1 : 0);
}

static void dsp_unmodeled(CdjC6747 *s, uint32_t addr, unsigned size, uint32_t value, bool write)
{
    xdj_hardware_dsp_gap(addr, value, size, write,
        s->host.core ? c66x_get_exec_pc(s->host.core) : 0);
    if (!(s->unmodeled_reads + s->unmodeled_writes)) {
        s->first_gap.addr=addr;s->first_gap.size=size;s->first_gap.value=value;
        s->first_gap.write=write;
        s->first_gap.pc=s->host.core ? c66x_get_exec_pc(s->host.core) : 0;
    }
    if (write) { s->unmodeled_writes++; } else { s->unmodeled_reads++; }
    if (s->host.timeline.enabled) {
        cdj_timeline_add(&s->host.timeline,g_get_monotonic_time(),s->host.dsp_ns,
            9,s->host.core ? c66x_get_exec_pc(s->host.core) : 0,addr,
            ((uint64_t)write << 32) | value,0);
    }
}

static uint32_t dsp_bus_read(void *opaque, uint32_t addr, unsigned size)
{
    CdjC6747 *s = opaque;
    unsigned pair, reg;
    uint32_t val = 0;

    if (addr - 0x01800000u < 0x200) {
        val = da810_intc_read(&s->intc, addr - 0x01800000);
    } else if (addr - 0x01c14000u < 0x1000) {
        if (!da810_syscfg_read(&s->syscfg, addr - 0x01c14000, size, &val))
            dsp_unmodeled(s, addr, size, 0, false);
    } else if (addr - 0x01c11000u < sizeof s->pll.reg) {
        val = da810_pll_read(&s->pll, addr - 0x01c11000, size);
    } else if (addr - 0x01c20000u < sizeof s->timer[0].reg) {
        val = da810_timer_read(&s->timer[0], addr - 0x01c20000, size);
    } else if (addr - 0x01c21000u < sizeof s->timer[1].reg) {
        val = da810_timer_read(&s->timer[1], addr - 0x01c21000, size);
    } else if (addr - 0x01c00000u < 0x8000) {
        val = da810_edma_read(&s->edma, addr - 0x01c00000, size);
    } else if (addr == UHPI_HPIC) {
        val = cdj_dsp_host_hpic(&s->host);
    } else if (addr == SPI1_SPIBUF) {
        val = SPIBUF_RXEMPTY;
    } else if (gpio_reg(addr, &pair, &reg)) {
        val = reg == GPIO_DIR ? s->gpio_dir[pair]
            : reg == GPIO_OUT ? s->gpio_out[pair]
            : reg == GPIO_IN  ? gpio_in(s, pair) : 0;
        if (pair == GPIO_CMD_PAIR && reg == GPIO_IN) {
            s->host.pin_reads++;
        }
    } else {
        if (addr - MCASP_BASE < 3 * MCASP_STRIDE) {
            val = da810_mcasp_read(&s->mcasp[(addr-MCASP_BASE)/MCASP_STRIDE],
                                   (addr-MCASP_BASE)%MCASP_STRIDE, size);
        } else {
            dsp_unmodeled(s,addr,size,0,false);
        }
    }
    cdj_dsp_count(&s->host.busr, addr, val);
    return val;
}

static void dsp_bus_write(void *opaque, uint32_t addr, uint32_t val,
                          unsigned size)
{
    CdjC6747 *s = opaque;
    unsigned pair, reg;

    if (addr - 0x01800000u < 0x200) {
        da810_intc_write(&s->intc, addr - 0x01800000, val);
    } else if (addr - 0x01c14000u < 0x1000) {
        if (!da810_syscfg_write(&s->syscfg, addr - 0x01c14000, val, size))
            dsp_unmodeled(s, addr, size, val, true);
    } else if (addr - 0x01c11000u < sizeof s->pll.reg) {
        da810_pll_write(&s->pll, addr - 0x01c11000, val, size);
        if (!s->clock_override) {
            uint64_t hz = da810_pll_sysclk(&s->pll, 1);
            if (hz > XDJ_DSP_MAX_HZ)
                xdj_hardware_violation("DA810 core clock exceeds 456MHz rating", hz);
            cdj_dsp_host_clock(&s->host, hz);
        }
    } else if (addr - 0x01c20000u < sizeof s->timer[0].reg) {
        da810_timer_write(&s->timer[0], addr - 0x01c20000, val, size);
    } else if (addr - 0x01c21000u < sizeof s->timer[1].reg) {
        da810_timer_write(&s->timer[1], addr - 0x01c21000, val, size);
    } else if (addr - 0x01c00000u < 0x8000) {
        da810_edma_write(&s->edma, addr - 0x01c00000, val, size);
    } else if (addr == UHPI_HPIC) {
        cdj_dsp_host_dsp_hpic(&s->host, val);
    } else if (gpio_reg(addr, &pair, &reg)) {
        gpio_write(s, pair, reg, val);
    } else {
        if (addr - MCASP_BASE < 3 * MCASP_STRIDE) {
            da810_mcasp_write(&s->mcasp[(addr-MCASP_BASE)/MCASP_STRIDE],
                              (addr-MCASP_BASE)%MCASP_STRIDE, val, size);
        } else {
            dsp_unmodeled(s,addr,size,val,true);
        }
    }
    cdj_dsp_count(&s->host.busw, addr, val);
}

static uint32_t dma_read(void *opaque, uint32_t addr, unsigned size)
{
    CdjC6747 *s = opaque;
    uint8_t *p = dsp_ram(s, addr);
    if (p) {
        uint32_t value;
        if (!xdj_dsp_ram_read(p, xdj_dsp_ram_remaining(addr), size, &value))
            dsp_unmodeled(s, addr, size, 0, false);
        return value;
    }
    return dsp_bus_read(s, addr, size);
}

static void dma_write(void *opaque, uint32_t addr, uint32_t val, unsigned size)
{
    CdjC6747 *s = opaque;
    uint8_t *p = dsp_ram(s, addr);
    if (p) {
        if (!xdj_dsp_ram_write(p, xdj_dsp_ram_remaining(addr), size, val))
            dsp_unmodeled(s, addr, size, val, true);
        else if (s->host.core) c66x_invalidate(s->host.core, addr, size);
    } else {
        dsp_bus_write(s, addr, val, size);
    }
}

/* One array between two stretches of plain DSP memory. The core is told once
 * for the whole destination, as for any host store. */
static bool dma_copy(void *opaque, uint32_t src, uint32_t dst, unsigned len)
{
    CdjC6747 *s = opaque;
    uint8_t *from = dsp_ram(s, src), *to = dsp_ram(s, dst);
    if (!len || !from || !to || dsp_ram(s, src + len - 1) != from + len - 1 ||
        dsp_ram(s, dst + len - 1) != to + len - 1) {
        return false;
    }
    /* Host pointers, so the two views of L2 count as the same memory. */
    if (from < to + len && to < from + len) {
        return false;
    }
    memcpy(to, from, len);
    if (s->host.core) { c66x_invalidate(s->host.core, dst, len); }
    return true;
}

static void audio_event(void *opaque, unsigned channel)
{
    CdjC6747 *s = opaque;
    da810_edma_event(&s->edma, channel, 0);
    da810_edma_event(&s->edma, channel, 1);
}

static void pcm_timeline(CdjC6747 *s, unsigned kind, uint64_t amount, bool freeze)
{
    /* PCM lock only: the callback must never wait for the DSP lock. */
    if (!s->pcm_timeline.enabled) { return; }
    cdj_timeline_add(&s->pcm_timeline,g_get_monotonic_time(),qatomic_read(&s->host.dsp_ns),
        kind,0,s->pcm_length/8,amount,freeze);
    if (freeze) { qatomic_set(&s->host.timeline_freeze,1); }
}

/* Called under the PCM lock. An overflow means the consumer stalled or the
 * DSP repaid more debt than the queue holds. Discard the oldest audio but
 * keep the cushion, so the next burst does not underrun; never move a read
 * currently in flight. */
static void pcm_recover(CdjC6747 *s)
{
    const unsigned keep = PCM_RECOVER_FRAMES * PCM_FRAME_BYTES;
    if (!s->pcm_resync_pending || s->pcm_reading) { return; }
    if (s->pcm_length > keep) {
        unsigned discarded = s->pcm_length - keep;
        s->pcm_head = (s->pcm_head + discarded) % sizeof s->pcm;
        s->pcm_length = keep;
        s->pcm_overflows += discarded / 8;
        s->pcm_resyncs++;
        pcm_timeline(s,8,discarded/8,true);
    }
    s->pcm_resync_pending = false;
}

static void audio_sample(void *opaque, unsigned port, unsigned slot, int32_t value)
{
    CdjC6747 *s = opaque;
    unsigned tail;
    uint64_t magnitude = value < 0 ? -(int64_t)value : value;
    if (value) { s->pcm_nonzero++; }
    s->pcm_peak = MAX(s->pcm_peak, magnitude);
    if (slot == 0) { s->left_sample = value; return; }
    if (slot != 1) { return; }
    s->pcm_frames++;
    if (s->pcm_capture) {
        uint8_t frame[8];
        stl_le_p(frame, s->left_sample);
        stl_le_p(frame + 4, value);
        fwrite(frame, 1, sizeof frame, s->pcm_capture);
    }
    qemu_mutex_lock(&s->pcm_lock);
    if (s->pcm_length + 8 > sizeof s->pcm) {
        s->pcm_last_overflow_ns = s->host.dsp_ns;
        s->pcm_resync_pending = true;
        pcm_recover(s);
        if (s->pcm_length + 8 > sizeof s->pcm) {
            s->pcm_overflows++;
            qemu_mutex_unlock(&s->pcm_lock);
            return;
        }
    }
    tail = (s->pcm_head + s->pcm_length) % sizeof s->pcm;
    stl_le_p(s->pcm + tail, s->left_sample);
    stl_le_p(s->pcm + tail + 4, value);
    s->pcm_length += 8;
    if (!(s->pcm_frames % 441)) { pcm_timeline(s,5,s->pcm_frames,false); }
    qemu_mutex_unlock(&s->pcm_lock);
}

static void audio_callback(void *opaque, int available)
{
    CdjC6747 *s = opaque;
    uint8_t data[4096];
    while (available >= 8) {
        unsigned length, wrote;
        {
            qemu_mutex_lock(&s->pcm_lock);
            length = MIN(s->pcm_length, sizeof s->pcm - s->pcm_head);
            length = MIN(length, MIN(sizeof data, (unsigned)available)) & ~7u;
            if (!length && s->pcm_frames) { pcm_timeline(s,7,available/8,true); }
            memcpy(data, s->pcm+s->pcm_head, length);
            s->pcm_reading = length != 0;
            qemu_mutex_unlock(&s->pcm_lock);
        }
        if (!length) { break; }
        /* Copy and commit under the short PCM lock. Never wait for a DSP
         * step, drop the BQL or hold PCM while entering AUD_write. Queue
         * recovery waits for this copied read to finish. */
        wrote = AUD_write(s->voice, data, length);
        if (s->pcm_consumed) {
            fwrite(data, 1, wrote, s->pcm_consumed);
        }
        {
            qemu_mutex_lock(&s->pcm_lock);
            s->pcm_head = (s->pcm_head+wrote) % sizeof s->pcm;
            s->pcm_length -= wrote;
            s->pcm_consumed_frames += wrote / 8;
            pcm_timeline(s,6,wrote/8,false);
            s->pcm_reading = false;
            pcm_recover(s);
            qemu_mutex_unlock(&s->pcm_lock);
        }
        if (!wrote) { break; }
        available -= wrote;
    }
}

static void timer0_event(void *opaque, unsigned half)
{
    CdjC6747 *s = opaque;
    /* SPRUH96B Table 2-1: TINT0L / TINT0H. */
    da810_intc_event(&s->intc, half ? 64 : 4);
}

static void timer1_event(void *opaque, unsigned half)
{
    CdjC6747 *s = opaque;
    da810_intc_event(&s->intc, half ? 48 : 40);
}

static void dsp_advance(void *opaque, uint64_t ns)
{
    CdjC6747 *s = opaque;
    unsigned i;
    uint64_t auxclk = da810_pll_auxclk(&s->pll);
    for (i = 0; i < 2; i++) {
        da810_timer_advance(&s->timer[i], ns, auxclk);
    }
    for (i = 0; i < 3; i++) { da810_mcasp_advance(&s->mcasp[i], ns); }
}

static void dma_irq(void *opaque, unsigned region, int level)
{
    CdjC6747 *s = opaque;
    /* DSP firmware uses region 1; event IDs from SPRUH96B Table 6-1. */
    if (level && (region == 1 || region == 4)) {
        da810_intc_event(&s->intc, region == 1 ? 8 : 56);
    }
}

/* A fresh core at the boot word's entry, as the chip does on the first
 * DSPINT after the host has loaded it. */
static void dsp_irq(void *opaque, unsigned line)
{
    CdjC6747 *s = opaque;
    if (s->host.core) {
        c66x_set_irq(s->host.core, line, 1);
        c66x_set_irq(s->host.core, line, 0);
    }
}

static uint32_t idle_hint_load(const char *path);

static void dsp_start(CdjC6747 *s)
{
    c66x_bus bus = { s, dsp_bus_read, dsp_bus_write };
    c66x_core *core = cdj_dsp_host_core(&s->host, &bus);
    const char *record = getenv("CDJ_XDJ_DSP_RECORD");

    /* Optional deterministic hardware trace for the offline core oracle.
     * Open before mapping RAM so replay receives boot bytes and aliases. */
    if (record && c66x_record_open(core, record)) {
        error_report("Cannot open native DSP execution record %s", record);
        exit(1);
    }

    c66x_map_ram(core, L2_BASE, L2_SIZE, s->l2);
    c66x_map_ram(core, L2_ALIAS, L2_SIZE, s->l2);
    c66x_map_ram(core, SDRAM_BASE, SDRAM_SIZE, s->sdram);
    c66x_map_ram(core, SHRAM_BASE, SHRAM_SIZE, s->shram);
    c66x_map_ram(core, L1P_BASE, L1_SIZE, s->l1p);
    c66x_map_ram(core, XDJ_DSP_L1P_ALIAS, L1_SIZE, s->l1p);
    c66x_map_ram(core, L1D_BASE, L1_SIZE, s->l1d);
    c66x_map_ram(core, XDJ_DSP_L1D_ALIAS, L1_SIZE, s->l1d);
    /* The DSP firmware has no IDLE instruction; between interrupts it spins
     * in a polling loop for most of its cycles. The core finds that loop and
     * skips passes it proves to be fixed points. CDJ_XDJ_DSP_IDLE=0 runs
     * every cycle; CDJ_XDJ_DSP_IDLE_HINT names the file the head is kept in. */
    const char *idle = getenv("CDJ_XDJ_DSP_IDLE");
    if (!idle || strcmp(idle, "0")) {
        const char *hint = getenv("CDJ_XDJ_DSP_IDLE_HINT");
        s->idle_hint_path = hint && *hint ? g_strdup(hint) : NULL;
        s->idle_saved = s->idle_hint_path ? idle_hint_load(s->idle_hint_path) : 0;
        /* The audio engine runs in the handlers: they must not pay for
         * the background loop's tracking on every store. */
        c66x_idle_isr_fast(core, 1);
        c66x_idle_discover(core, s->idle_saved);
    }
    cdj_dsp_host_run(&s->host, ldl_le_p(s->l2 + (BOOT_WORD - L2_BASE)));
}

static void hpi_note_write(CdjC6747 *s, uint32_t addr)
{
    unsigned i;

    for (i = 0; i < s->nruns; i++) {
        HpiRun *r = &s->runs[i];

        if (addr == r->end) {
            r->end += 4;
            return;
        }
        if (addr >= r->start && addr < r->end) {
            return;
        }
    }
    if (s->nruns < MAX_RUNS) {
        s->runs[s->nruns++] = (HpiRun){ addr, addr + 4 };
    } else {
        s->runs_overflowed = true;
    }
}

static uint64_t hpi_read(void *opaque, hwaddr off, unsigned size)
{
    CdjC6747 *s = opaque;
    uint8_t *p;
    uint32_t val;

    switch (off >> 18) {
    case 0:
        return cdj_dsp_host_hpic(&s->host);
    case 1:
        return s->hpia;
    default: {
        /* A host-port read of DSP memory needs nothing from the core: on the
         * board it steals one bus cycle from a DSP that keeps running, and it
         * sees whatever word is there at that moment. Taking the DSP lock for
         * it instead stops the core's thread for every word MAIN polls (16 k
         * a second during playback), which costs both sides far more than
         * the read. One aligned 32-bit load is a whole word either side of a
         * concurrent DSP store. */
        p = dsp_ram(s, s->hpia & ~3u);
        if (p) {
            val = le32_to_cpu(qatomic_read__nocheck((uint32_t *)p));
        } else {
            CDJ_DSP_HOST_GUARD(&s->host);
            val = 0;
            dsp_unmodeled(s, s->hpia, size, 0, false);
        }
        s->reads++;
        cdj_dsp_count(&s->polls, s->hpia, val);
        if ((off >> 18) == 2) {
            s->hpia += 4;
        }
        return val;
    }
    }
}

/* The channel transfers synchronously already. Keep one DSP arbitration
 * interval for a contiguous RAM transfer instead of one per UHPI word.
 * Unsupported addressing/regions retain the ordinary MMIO behavior. */
static bool hpi_dma_bulk(void *opaque, uint32_t sar, uint32_t dar,
                         unsigned unit, unsigned sm, unsigned dm,
                         uint64_t length)
{
    CdjC6747 *s = opaque;
    bool write = sm == 1 && dm == 0 && A7ADDR(dar) == 0x18080000;
    bool read = sm == 0 && dm == 1 && A7ADDR(sar) == 0x18080000;
    uint32_t main_addr = A7ADDR(write ? sar : dar);
    g_autofree uint8_t *buffer = NULL;
    uint8_t *p;

    if (unit != 4 || (!write && !read) || main_addr < 0x08000000 ||
        (uint64_t)main_addr + length > 0x18000000) {
        return false;
    }
    buffer = g_malloc(length);
    if (write) {
        cpu_physical_memory_read(main_addr, buffer, length);
    }
    {
        CDJ_DSP_HOST_GUARD(&s->host);
        p = dsp_ram(s, s->hpia);
        if ((s->hpia & 3) || !p ||
            (uint64_t)s->hpia + length > UINT32_MAX ||
            (uintptr_t)dsp_ram(s, s->hpia + length - 1) !=
            (uintptr_t)p + length - 1) {
            return false;
        }
        if (write) {
            memcpy(p, buffer, length);
            if (s->host.core) {
                c66x_invalidate(s->host.core, s->hpia, length);
            }
            for (uint64_t i = 0; i < length; i += 4) {
                hpi_note_write(s, s->hpia + i);
            }
            s->writes += length / 4;
        } else {
            memcpy(buffer, p, length);
            for (uint64_t i = 0; i < length; i += 4) {
                cdj_dsp_count(&s->polls, s->hpia + i, ldl_le_p(buffer + i));
            }
            s->reads += length / 4;
        }
        s->hpia += length;
    }
    if (read) {
        /* Restore the BQL before delivering the DMA into MAIN memory. */
        cpu_physical_memory_write(main_addr, buffer, length);
    }
    return true;
}

void xdj_da810_dma_init(void)
{
    cdj_dmac_set_bulk(hpi_dma_bulk, &c6747);
}

static void hpi_write(void *opaque, hwaddr off, uint64_t val, unsigned size)
{
    CdjC6747 *s = opaque;
    uint8_t *p;

    /* HPIA belongs to MAIN's serialized MMIO path. Selecting an address
     * transfers no DSP memory and must not preempt the DSP worker. HPID and
     * HPIC writes below still synchronize RAM, core startup and interrupts. */
    if ((off >> 18) == 1) {
        s->hpia = val;
        return;
    }
    CDJ_DSP_HOST_GUARD(&s->host);

    switch (off >> 18) {
    case 0: {
        uint32_t previous = s->host.hpic;
        /* DSPINT is write-one-to-set from MAIN; writing zero preserves it. */
        bool interrupt = cdj_dsp_host_main_hpic(&s->host,
                                               val | (previous & HPIC_DSPINT));
        if (interrupt && !(previous & HPIC_DSPINT)) {
            s->dspints++;
            if (s->host.enabled && !s->host.core) {
                dsp_start(s);
            } else {
                da810_intc_event(&s->intc, 34);
            }
        }
        break;
    }
    default:
        p = dsp_ram(s, s->hpia & ~3u);
        if (p) {
            stl_le_p(p, val);
            if (s->host.core) {
                c66x_invalidate(s->host.core, s->hpia & ~3u, 4);
            }
        } else {
            s->dropped++;
            dsp_unmodeled(s, s->hpia, size, val, true);
        }
        s->writes++;
        hpi_note_write(s, s->hpia);
        if ((off >> 18) == 2) {
            s->hpia += 4;
        }
        break;
    }
}

static const MemoryRegionOps hpi_ops = {
    .read = hpi_read,
    .write = hpi_write,
    .endianness = DEVICE_LITTLE_ENDIAN,
    .valid = { .min_access_size = 4, .max_access_size = 4 },
};

static void hpi_dump_run(CdjC6747 *s, const char *dir, const HpiRun *r)
{
    g_autofree char *path = g_strdup_printf("%s/hpi_%08x.bin", dir, r->start);
    FILE *f = fopen(path, "wb");
    uint32_t a;

    if (!f) {
        warn_report("hpi: cannot write %s", path);
        return;
    }
    for (a = r->start; a < r->end; a += 4) {
        uint8_t *p = dsp_ram(s, a);
        uint8_t zero[4] = { 0 };

        fwrite(p ? p : zero, 1, 4, f);
    }
    fclose(f);
}

static char *audio_state_locked(CdjC6747 *s)
{
    int64_t host_ns = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
    qemu_mutex_lock(&s->pcm_lock);
    char *state = g_strdup_printf("{\"started\":%s,\"host_ns\":%" PRId64
                           ",\"dsp_ns\":%" PRId64 ",\"lag_max_ns\":%" PRId64
                           ",\"clock_hz\":%" PRIu64 ",\"frames\":%" PRIu64
                           ",\"queue_frames\":%u,\"dropped_frames\":%" PRIu64
                           ",\"queue_recoveries\":%" PRIu64
                           ",\"serializer_underruns\":%" PRIu64
                           ",\"timer_tcr\":[%u,%u],\"timer_tgcr\":[%u,%u]"
                           ",\"timer_unsupported\":[%u,%u],\"running\":%s,\"stop\":%u,\"trap_pc\":%u"
                           ",\"unmodeled_reads\":%" PRIu64 ",\"unmodeled_writes\":%" PRIu64
                           ",\"consumed_frames\":%" PRIu64
                           ",\"queue_capacity_frames\":%u,\"lag_cap_ns\":%" PRId64
                           ",\"lag_forgiven_ns\":%" PRId64
                           ",\"lag_forgiven_events\":%" PRIu64 "}",
                           s->host.core ? "true" : "false", host_ns,
                           s->host.dsp_ns, s->host.lag_max_ns, s->host.clock_hz,
                           s->pcm_frames, s->pcm_length / 8, s->pcm_overflows,
                           s->pcm_resyncs, s->mcasp[1].underruns,
                           s->timer[0].reg[0x20/4], s->timer[1].reg[0x20/4],
                           s->timer[0].reg[0x24/4], s->timer[1].reg[0x24/4],
                           s->timer[0].unsupported, s->timer[1].unsupported,
                           s->host.running ? "true" : "false",s->host.last_stop,s->host.trap_pc,
                           s->unmodeled_reads,s->unmodeled_writes,s->pcm_consumed_frames,
                           PCM_QUEUE_FRAMES, s->host.lag_cap_ns,
                           s->host.lag_forgiven_ns, s->host.lag_forgiven_events);
    qemu_mutex_unlock(&s->pcm_lock);
    return state;
}

char *xdj_da810_audio_state(void)
{
    CdjC6747 *s = &c6747;
    CDJ_DSP_HOST_GUARD(&s->host);
    return audio_state_locked(s);
}

static bool xdj_audio_muted(Object *object, Error **errp)
{
    return c6747.audio_muted;
}

static void xdj_audio_mute(Object *object, bool muted, Error **errp)
{
    c6747.audio_muted = muted;
    AUD_set_volume_out(c6747.voice, muted, 255, 255);
}

static bool xdj_performance_enabled(Object *object, Error **errp)
{
    return qatomic_read(&c6747.host.perf.enabled);
}

static void xdj_performance_enable(Object *object, bool value, Error **errp)
{
    qatomic_set(&c6747.host.perf.enabled, value);
}

static char *performance_state_locked(CdjDspHost *h)
{
    char jit[768] = { 0 };
    int idle_confirmed = 0;
    uint32_t idle_head = h->core ? c66x_idle_head(h->core, &idle_confirmed) : 0;
    if (h->core) { c66x_jit_report(h->core, jit, sizeof jit); }
    return g_strdup_printf("{\"enabled\":%s,\"steps\":%" PRIu64
        ",\"step_us\":%" PRIu64 ",\"worker_cpu_us\":%" PRIu64 ",\"advance_us\":%" PRIu64
        ",\"thread_wait_us\":%" PRIu64 ",\"host_lock_calls\":%" PRIu64
        ",\"host_wait_us\":%" PRIu64 ",\"bql_wait_us\":%" PRIu64
        ",\"react_us\":%" PRIu64 ",\"commands\":%" PRIu64
        ",\"react_cycles\":%" PRIu64 ",\"hpi_reads\":%" PRIu64
        ",\"hpi_writes\":%" PRIu64 ",\"step_max_us\":%" PRIu64
        ",\"step_max_pc\":%u,\"slow_steps\":%" PRIu64 ",\"chain_exits\":%" PRIu64
        ",\"idle_head\":%u,\"idle_confirmed\":%s,\"idle_steps\":%" PRIu64
        ",\"idle_skipped_cycles\":%" PRIu64 ",\"jit\":\"%s\"}",
        qatomic_read(&h->perf.enabled) ? "true" : "false",
        qatomic_read(&h->perf.steps), qatomic_read(&h->perf.step_us),
        qatomic_read(&h->perf.worker_cpu_us), qatomic_read(&h->perf.advance_us), qatomic_read(&h->perf.thread_wait_us),
        qatomic_read(&h->perf.host_calls), qatomic_read(&h->perf.host_wait_us),
        qatomic_read(&h->perf.bql_wait_us), qatomic_read(&h->perf.react_us),
        h->commands, h->react_cycles, c6747.reads, c6747.writes, h->perf.step_max_us, h->perf.step_max_pc,
        h->perf.slow_steps, c66x_jit_chain_exits(h->core),
        idle_head, idle_confirmed ? "true" : "false", h->idle_steps, h->idle_skipped_cycles, jit);
}

static char *xdj_performance_state(Object *object, Error **errp)
{
    CdjDspHost *h = &c6747.host;
    CDJ_DSP_HOST_GUARD(h);
    return performance_state_locked(h);
}

/* Called under the DSP lock. A head is saved once a pass through it has been
 * a fixed point, never a proposal still on trial. */
static void idle_hint_save(CdjC6747 *s)
{
    int confirmed = 0;
    uint32_t head = s->host.core ? c66x_idle_head(s->host.core, &confirmed) : 0;
    if (!confirmed || head == s->idle_saved) { return; }
    s->idle_saved = head;
    info_report("xdj.dsp: polling loop at 0x%08x; idle passes are skipped", head);
    if (s->idle_hint_path) {
        g_autofree char *text = g_strdup_printf("0x%08x\n", head);
        g_autoptr(GError) err = NULL;
        if (!g_file_set_contents(s->idle_hint_path, text, -1, &err)) {
            warn_report("xdj.dsp: cannot save %s: %s", s->idle_hint_path, err->message);
        }
    }
}

static uint32_t idle_hint_load(const char *path)
{
    g_autofree char *text = NULL;
    char *end;
    if (!g_file_get_contents(path, &text, NULL, NULL)) { return 0; }
    unsigned long long head = g_ascii_strtoull(text, &end, 16);
    /* An execute packet starts on an even address inside the DSP's memory. */
    return end == text || head > UINT32_MAX || (head & 1) ? 0 : (uint32_t)head;
}

static void publish_live_state(void *opaque)
{
    CdjC6747 *s = opaque;
    int64_t sampled = g_get_monotonic_time();
    idle_hint_save(s);
    char *audio = audio_state_locked(s);
    char *performance = performance_state_locked(&s->host);
    qemu_mutex_lock(&s->live_lock);
    char *old_audio = s->live_audio, *old_performance = s->live_performance;
    s->live_audio = audio;
    s->live_performance = performance;
    s->live_sample_us = sampled;
    qemu_mutex_unlock(&s->live_lock);
    g_free(old_audio);
    g_free(old_performance);
}

static char *xdj_live_state(Object *object, Error **errp)
{
    CdjC6747 *s = &c6747;
    qemu_mutex_lock(&s->live_lock);
    char *state = g_strdup_printf("{\"schema\":1,\"sample_host_us\":%" PRId64
        ",\"age_us\":%" PRId64 ",\"audio\":%s,\"performance\":%s}",
        s->live_sample_us, MAX(INT64_C(0), g_get_monotonic_time() - s->live_sample_us),
        s->live_audio, s->live_performance);
    qemu_mutex_unlock(&s->live_lock);
    return state;
}

static bool timeline_enabled(Object *object, Error **errp)
{
    return c6747.host.timeline.enabled;
}

static void timeline_enable(Object *object, bool enabled, Error **errp)
{
    CdjC6747 *s=&c6747;
    CDJ_DSP_HOST_GUARD(&s->host);
    qemu_mutex_lock(&s->pcm_lock);
    cdj_timeline_reset(&s->host.timeline,enabled);
    cdj_timeline_reset(&s->pcm_timeline,enabled);
    s->host.timeline_next_ns=0;
    qatomic_set(&s->host.timeline_freeze,0);
    if (enabled) { qatomic_set(&s->host.perf.enabled,true); }
    qemu_mutex_unlock(&s->pcm_lock);
}

static void timeline_json(GString *json, CdjTimeline *t)
{
    g_string_append_printf(json,"{\"total\":%" PRIu64 ",\"overwritten_history\":%" PRIu64
        ",\"frozen\":%s,\"events\":[",t->total,t->total-t->count,t->frozen?"true":"false");
    unsigned first=(t->next+CDJ_TIMELINE_CAP-t->count)%CDJ_TIMELINE_CAP;
    for(unsigned i=0;i<t->count;i++) {
        CdjTimelineEvent *e=&t->event[(first+i)%CDJ_TIMELINE_CAP];
        g_string_append_printf(json,"%s{\"host_us\":%" PRIu64 ",\"guest_ns\":%" PRIu64
            ",\"kind\":%u,\"pc\":%u,\"a\":%" PRIu64 ",\"b\":%" PRIu64 "}",
            i?",":"",e->host_us,e->guest_ns,e->kind,e->pc,e->a,e->b);
    }
    g_string_append(json,"]}");
}

static char *diagnostic_state(Object *object, Error **errp)
{
    CdjC6747 *s=&c6747;
    CDJ_DSP_HOST_GUARD(&s->host);
    qemu_mutex_lock(&s->pcm_lock);
    GString *json=g_string_new(NULL);
    g_string_append_printf(json,"{\"schema\":1,\"enabled\":%s,\"hardware_reference\":\"unverified\","
        "\"cpu\":\"c674x\",\"unmodeled_reads\":%" PRIu64 ",\"unmodeled_writes\":%" PRIu64
        ",\"first_gap\":{\"addr\":%u,\"pc\":%u,\"size\":%u,\"write\":%u,\"value\":%u},\"dsp\":",
        s->host.timeline.enabled?"true":"false",s->unmodeled_reads,s->unmodeled_writes,
        s->first_gap.addr,s->first_gap.pc,s->first_gap.size,s->first_gap.write,s->first_gap.value);
    timeline_json(json,&s->host.timeline);g_string_append(json,",\"pcm\":");timeline_json(json,&s->pcm_timeline);
    g_string_append(json,"}");
    qemu_mutex_unlock(&s->pcm_lock);
    return g_string_free(json,false);
}

void xdj_da810_performance_properties(Object *object)
{
    object_property_add_bool(object, "xdj-audio-muted", xdj_audio_muted, xdj_audio_mute);
    object_property_add_bool(object, "xdj-performance-enabled",
                             xdj_performance_enabled, xdj_performance_enable);
    object_property_add_str(object, "xdj-live-state", xdj_live_state, NULL);
    object_property_add_str(object, "xdj-performance-state",
                            xdj_performance_state, NULL);
    object_property_add_bool(object,"xdj-timeline-enabled",timeline_enabled,timeline_enable);
    object_property_add_str(object,"xdj-diagnostic-state",diagnostic_state,NULL);
}

static void c6747_exit_report(Notifier *n, void *data)
{
    CdjC6747 *s = container_of(n, CdjC6747, exit);
    {
        CDJ_DSP_HOST_GUARD(&s->host);
        if (s->pcm_capture) { fclose(s->pcm_capture); s->pcm_capture = NULL; }
        if (s->pcm_consumed) { fclose(s->pcm_consumed); s->pcm_consumed = NULL; }
    }
    const char *dir = getenv("CDJ_HPI_DUMP");
    unsigned i;

    info_report("hpi: %" PRIu64 " data writes (%" PRIu64 " outside RAM), "
                "%" PRIu64 " data reads, %" PRIu64 " DSPINT, HPIC 0x%08x",
                s->writes, s->dropped, s->reads, s->dspints, s->host.hpic);
    for (i = 0; i < s->nruns; i++) {
        info_report("hpi: run 0x%08x..0x%08x (%u bytes)", s->runs[i].start,
                    s->runs[i].end, s->runs[i].end - s->runs[i].start);
        if (dir && *dir) {
            hpi_dump_run(s, dir, &s->runs[i]);
        }
    }
    if (dir && *dir) {
        cdj_dsp_dump_mem(dir, "l2.bin", s->l2, L2_SIZE);
        cdj_dsp_dump_mem(dir, "sdram.bin", s->sdram, MiB);
    }
    if (s->runs_overflowed) {
        info_report("hpi: more than %d runs; later ones not listed", MAX_RUNS);
    }
    for (i = 0; i < s->polls.n; i++) {
        info_report("hpi: host read 0x%08x x%" PRIu64 ", last 0x%08x",
                    s->polls.e[i].addr, s->polls.e[i].count,
                    s->polls.e[i].last);
    }
    cdj_dsp_host_report(&s->host);
    for (i = 0; i < 2; i++) {
        info_report("xdj.timer%u: TCR %08x TGCR %08x INTCTLSTAT %08x, "
                    "unsupported features %08x", i, s->timer[i].reg[0x20/4],
                    s->timer[i].reg[0x24/4], s->timer[i].reg[0x44/4],
                    s->timer[i].unsupported);
    }
    if (s->host.core) {
        c66x_stats stats;
        char jit[768];
        CDJ_DSP_HOST_GUARD(&s->host);
        c66x_get_stats(s->host.core, &stats);
        c66x_record_close(s->host.core);
        c66x_jit_report(s->host.core, jit, sizeof jit);
        info_report("xdj.dsp: %s", jit);
        info_report("xdj.dsp: CSR %08x IER %08x IFR %08x ISTP %08x; "
                    "%" PRIu64 " interrupts, EDMA %u edges, UHPI %u edges",
                    c66x_get_creg(s->host.core, 1), c66x_get_creg(s->host.core, 4),
                    c66x_get_creg(s->host.core, 2), c66x_get_creg(s->host.core, 5),
                    stats.interrupts, stats.irq_edges[8], stats.irq_edges[15]);
    }
    info_report("xdj.dac: %" PRIu64 " frames, %" PRIu64 " dropped frames at queue overruns; "
                "McASP1 %" PRIu64 " words, %" PRIu64 " serializer underruns",
                s->pcm_frames, s->pcm_overflows, s->mcasp[1].words,
                s->mcasp[1].underruns);
    info_report("xdj.dac: %" PRIu64 " nonzero words, peak %" PRIu64,
                s->pcm_nonzero, s->pcm_peak);
    if (s->pcm_overflows) {
        info_report("xdj.dac: %" PRIu64 " queue recoveries, last overrun at DSP time %" PRId64 " ms",
                    s->pcm_resyncs, s->pcm_last_overflow_ns / SCALE_MS);
    }
}

const CdjDspWires *xdj_da810_init(MemoryRegion *sysmem, hwaddr hpi_base)
{
    CdjC6747 *s = &c6747;
    unsigned i;

    qemu_mutex_init(&s->pcm_lock);
    qemu_mutex_init(&s->live_lock);
    s->l2 = g_malloc0(L2_SIZE);
    s->l1p = g_malloc0(L1_SIZE);
    s->l1d = g_malloc0(L1_SIZE);
    s->shram = g_malloc0(SHRAM_SIZE);
    s->sdram = g_malloc0(SDRAM_SIZE);
    da810_syscfg_init(&s->syscfg);
    da810_intc_init(&s->intc, dsp_irq, s);
    da810_edma_init(&s->edma, dma_read, dma_write, dma_irq, s);
    s->edma.copy_mem = dma_copy;
    cdj_dsp_host_init(&s->host, "xdj-da810", "DA810", 451, set_command_pins, s);
    s->host.c674x_only = true;
    /* MAIN polls status every 6 ms. Bound its wait for each UHPI word;
     * shortening arbitration preserves cycles/time and native execution. */
    cdj_dsp_host_quantum(&s->host, 10000);
    /* Unset repays every nanosecond of debt: the strict behaviour that
     * measurement runs depend on. The interactive shell sets a cap. */
    const char *lag_cap = getenv("CDJ_XDJ_DSP_LAG_CAP_MS");
    if (lag_cap && *lag_cap) {
        char *end;
        unsigned long ms = strtoul(lag_cap, &end, 10);
        if (*end || ms < 1 || ms > DSP_LAG_CAP_MAX_MS) {
            error_report("CDJ_XDJ_DSP_LAG_CAP_MS must be 1..%u, the debt the "
                         "DAC queue can absorb", DSP_LAG_CAP_MAX_MS);
            exit(1);
        }
        cdj_dsp_host_lag_cap(&s->host, (int64_t)ms * SCALE_MS);
    }
    /* IC301's board reference is 22.5792 MHz. The firmware controls PLLM,
     * bypass and dividers through the same registers on every update. */
    da810_pll_init(&s->pll, XDJ_DSP_REFERENCE_HZ);
    da810_timer_init(&s->timer[0], timer0_event, s);
    da810_timer_init(&s->timer[1], timer1_event, s);
    s->clock_override = getenv("CDJ_DA810_MHZ") != NULL;
    if (!s->clock_override) {
        cdj_dsp_host_clock(&s->host, da810_pll_sysclk(&s->pll, 1));
    }
    s->host.advance = dsp_advance;
    const char *capture = getenv("CDJ_XDJ_PCM_CAPTURE");
    const char *consumed = getenv("CDJ_XDJ_PCM_CONSUMED");
    if (capture && *capture && !(s->pcm_capture = fopen(capture, "wb"))) {
        error_report("Cannot open native DAC diagnostic %s", capture);
        exit(1);
    }
    if (consumed && *consumed && !(s->pcm_consumed = fopen(consumed, "wb"))) {
        error_report("Cannot open consumed DAC diagnostic %s", consumed);
        exit(1);
    }
    for (i = 0; i < 3; i++) {
        da810_mcasp_init(&s->mcasp[i], i, audio_event, audio_sample, s);
    }
    s->card.state = audio_state_by_name("xdj-audio", &error_fatal);
    AUD_register_card("xdj1000mk2.dac", &s->card, &error_fatal);
    {
        struct audsettings settings = { XDJ_AUDIO_HZ, 2, AUDIO_FORMAT_S32, 0 };
        s->voice = AUD_open_out(&s->card, NULL, "xdj1000mk2.dac", s,
                                audio_callback, &settings);
        if (!s->voice) { error_report("Cannot open XDJ DAC audio output"); exit(1); }
        AUD_set_active_out(s->voice, 1);
    }

    memory_region_init_io(&s->iomem, NULL, &hpi_ops, s, "xdj-da810.hpi", 0x100000);
    memory_region_add_subregion(sysmem, hpi_base, &s->iomem);
    s->exit.notify = c6747_exit_report;
    qemu_add_exit_notifier(&s->exit);

    for (i = 0; i < GPIO_PAIRS; i++) {
        s->gpio_dir[i] = 0xFFFFFFFF;            /* every pin an input at reset */
    }
    publish_live_state(s);
    s->host.observe = publish_live_state;
    return &s->host.wires;
}
