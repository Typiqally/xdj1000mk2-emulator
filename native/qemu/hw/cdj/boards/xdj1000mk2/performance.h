/* SPDX-License-Identifier: GPL-2.0-or-later */
#ifndef XDJ_SYSTEM_PERFORMANCE_H
#define XDJ_SYSTEM_PERFORMANCE_H
#include "qemu/osdep.h"
#include "qom/object.h"

/* These devices and their QMP properties run under the BQL. No DSP lock,
 * guest memory modifications, or guest timing changes are needed to read them. */
typedef struct XdjHostTiming {
    uint64_t calls, wall_us, max_us;
} XdjHostTiming;

typedef struct XdjSystemPerformance {
    bool enabled;
    XdjHostTiming graphics, display, panel;
    uint64_t vblanks, vblank_late_us, vblank_late_max_us;
    uint64_t panel_last_us, panel_gap_max_us;
    uint64_t input_events, input_replies, input_reply_us, input_reply_max_us;
    uint64_t pending_input_us;
} XdjSystemPerformance;

extern XdjSystemPerformance xdj_system_performance;

typedef struct XdjPerfScope {
    XdjHostTiming *metric;
    int64_t start;
} XdjPerfScope;

static inline XdjPerfScope xdj_perf_start(XdjHostTiming *metric)
{
    return (XdjPerfScope) { metric, xdj_system_performance.enabled ? g_get_monotonic_time() : 0 };
}

static inline void xdj_perf_finish(XdjPerfScope *scope)
{
    if (scope->start) {
        uint64_t elapsed = g_get_monotonic_time() - scope->start;
        scope->metric->calls++;
        scope->metric->wall_us += elapsed;
        scope->metric->max_us = MAX(scope->metric->max_us, elapsed);
    }
}

G_DEFINE_AUTO_CLEANUP_CLEAR_FUNC(XdjPerfScope, xdj_perf_finish)
#define XDJ_PERF_SCOPE(field) \
    g_auto(XdjPerfScope) perf_scope_ = xdj_perf_start(&xdj_system_performance.field)

void xdj_system_performance_properties(Object *machine);
void xdj_perf_input(void);
void xdj_perf_panel_reply(void);
void xdj_perf_vblank(int64_t late_ns);
#endif
