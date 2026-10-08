/* SPDX-License-Identifier: GPL-2.0-or-later */
#ifndef CDJ_DSP_TIMELINE_H
#define CDJ_DSP_TIMELINE_H
#include <stdint.h>
#include <string.h>
#define CDJ_TIMELINE_CAP 1024
/* Caller owns the corresponding DSP or PCM mutex. No allocation, I/O or
 * extra locks on the producer/callback path. Overwritten history is expected;
 * the first observed failure freezes the ring and preserves its predecessors. */
typedef struct CdjTimelineEvent {
    uint64_t host_us, guest_ns, a, b;
    uint32_t kind, pc;
} CdjTimelineEvent;
typedef struct CdjTimeline {
    CdjTimelineEvent event[CDJ_TIMELINE_CAP];
    uint64_t total;
    unsigned next, count, enabled, frozen;
} CdjTimeline;
static inline void cdj_timeline_reset(CdjTimeline *t, unsigned enabled)
{
    memset(t,0,sizeof(*t));t->enabled=enabled;
}
static inline void cdj_timeline_add(CdjTimeline *t,uint64_t host_us,
    uint64_t guest_ns,uint32_t kind,uint32_t pc,uint64_t a,uint64_t b,unsigned freeze)
{
    if(!t->enabled || t->frozen) return;
    t->event[t->next]=(CdjTimelineEvent){host_us,guest_ns,a,b,kind,pc};
    t->next=(t->next+1)%CDJ_TIMELINE_CAP;
    if(t->count<CDJ_TIMELINE_CAP)t->count++;
    t->total++;t->frozen=!!freeze;
}
#endif
