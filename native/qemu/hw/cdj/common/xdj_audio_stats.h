/* SPDX-License-Identifier: GPL-2.0-or-later */
#ifndef XDJ_AUDIO_STATS_H
#define XDJ_AUDIO_STATS_H
#include <stdint.h>

#define XDJ_AUDIO_STATS_BINS 2048
typedef struct XdjAudioBin {
    uint64_t callbacks, frames, underruns, missed_frames;
} XdjAudioBin;
typedef struct XdjAudioStats {
    XdjAudioBin bins[XDJ_AUDIO_STATS_BINS];
    uint64_t outside_bins;
} XdjAudioStats;

/* Called under the CoreAudio buffer mutex; no allocation, I/O or BQL. */
static inline void xdj_audio_stats_add(XdjAudioStats *stats, uint64_t elapsed_us,
                                     uint32_t requested, uint32_t pending)
{
    uint64_t second = elapsed_us / 1000000;
    if (second >= XDJ_AUDIO_STATS_BINS) {
        stats->outside_bins++;
        return;
    }
    XdjAudioBin *bin = &stats->bins[second];
    bin->callbacks++;
    bin->frames += requested;
    if (pending < requested) {
        bin->underruns++;
        bin->missed_frames += requested;
    }
}
#endif
