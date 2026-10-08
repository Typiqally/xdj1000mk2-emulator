/* SPDX-License-Identifier: GPL-2.0-or-later */
#include "performance.h"
#include "qapi/error.h"

XdjSystemPerformance xdj_system_performance;

static bool performance_enabled(Object *obj, Error **errp)
{
    return xdj_system_performance.enabled;
}

static void performance_enable(Object *obj, bool enabled, Error **errp)
{
    /* Resuming observation must not charge the disabled interval to input or
     * panel latency. Counters remain cumulative for before/after comparisons. */
    xdj_system_performance.pending_input_us = 0;
    xdj_system_performance.panel_last_us = 0;
    xdj_system_performance.enabled = enabled;
}

static void timing_json(GString *out, const char *name, XdjHostTiming *m)
{
    g_string_append_printf(out, ",\"%s\":{\"calls\":%" PRIu64
        ",\"wall_us\":%" PRIu64 ",\"max_us\":%" PRIu64 "}",
        name, m->calls, m->wall_us, m->max_us);
}

static char *performance_state(Object *obj, Error **errp)
{
    XdjSystemPerformance *s = &xdj_system_performance;
    GString *out = g_string_new(NULL);
    g_string_append_printf(out, "{\"schema\":1,\"enabled\":%s,\"vblanks\":%" PRIu64
        ",\"vblank_late_us\":%" PRIu64 ",\"vblank_late_max_us\":%" PRIu64
        ",\"panel_gap_max_us\":%" PRIu64 ",\"input_events\":%" PRIu64
        ",\"input_replies\":%" PRIu64 ",\"input_reply_us\":%" PRIu64
        ",\"input_reply_max_us\":%" PRIu64
        ",\"input_delivery_scope\":\"panel reply built; guest response unverified\"",
        s->enabled ? "true" : "false", s->vblanks, s->vblank_late_us,
        s->vblank_late_max_us, s->panel_gap_max_us, s->input_events,
        s->input_replies, s->input_reply_us, s->input_reply_max_us);
    timing_json(out, "graphics", &s->graphics);
    timing_json(out, "display", &s->display);
    timing_json(out, "panel", &s->panel);
    g_string_append_c(out, '}');
    return g_string_free(out, false);
}

void xdj_perf_input(void)
{
    XdjSystemPerformance *s = &xdj_system_performance;
    if (!s->enabled) { return; }
    s->input_events++;
    /* Several changes can be carried by one reply. Measure from the earliest
     * pending event, so bursts cannot hide a long wait by replacing its time. */
    if (!s->pending_input_us) { s->pending_input_us = g_get_monotonic_time(); }
}

void xdj_perf_panel_reply(void)
{
    XdjSystemPerformance *s = &xdj_system_performance;
    if (!s->enabled) { return; }
    uint64_t now = g_get_monotonic_time();
    if (s->panel_last_us) {
        s->panel_gap_max_us = MAX(s->panel_gap_max_us, now - s->panel_last_us);
    }
    s->panel_last_us = now;
    if (s->pending_input_us) {
        uint64_t elapsed = now - s->pending_input_us;
        s->input_replies++;
        s->input_reply_us += elapsed;
        s->input_reply_max_us = MAX(s->input_reply_max_us, elapsed);
        s->pending_input_us = 0;
    }
}

void xdj_perf_vblank(int64_t late_ns)
{
    XdjSystemPerformance *s = &xdj_system_performance;
    if (!s->enabled) { return; }
    uint64_t late = MAX(late_ns, 0) / 1000;
    s->vblanks++;
    s->vblank_late_us += late;
    s->vblank_late_max_us = MAX(s->vblank_late_max_us, late);
}

void xdj_system_performance_properties(Object *machine)
{
    memset(&xdj_system_performance, 0, sizeof xdj_system_performance);
    object_property_add_bool(machine, "xdj-system-performance-enabled",
                             performance_enabled, performance_enable);
    object_property_add_str(machine, "xdj-system-performance-state", performance_state, NULL);
}
