/* SPDX-License-Identifier: GPL-2.0-or-later */
#include "xdj.h"
#include "performance.h"
#include "cdj_pnl_link.h"
#include "ui/input.h"
#include "qapi/visitor.h"
#include "qapi/error.h"
/* Board-specific panel peer. Host input changes serial-report fields; MAIN
 * interprets them and draws its own UI. No transport or guest RAM patching. */
#define XDJ_PANEL_FRAME 32
typedef struct XdjPanel {
    uint8_t keys[30], rotary;
    /* Report bytes 2 and 3: two 8-bit analogue readings MAIN stores beside
     * the pitch slider (0949ca94/0949ca9c -> 095427c8/095427dc). */
    uint8_t analog[2];
    /* Last complete frame MAIN sent; it carries the lamp states. */
    uint8_t main_frame[XDJ_PANEL_FRAME];
    uint64_t main_frames;
    uint16_t pitch, jog_count;
    int64_t jog_ns;
    bool jog_forward, jog_reverse;
    int manual_direction;
    int64_t manual_ns, manual_end_ns;
    uint16_t manual_period;
    unsigned x, y, down_frames, up_frames;
    bool want_down, latched, down;
} XdjPanel;
static XdjPanel *panel;

typedef struct XdjKey { int code; uint8_t offset, mask; } XdjKey;
static const XdjKey keys[] = {
    {Q_KEY_CODE_SPC,0x10,1}, {Q_KEY_CODE_C,0x10,2},
    {Q_KEY_CODE_E,0x10,4}, {Q_KEY_CODE_W,0x10,8}, {Q_KEY_CODE_Q,0x10,16},
    {Q_KEY_CODE_R,0x10,0x80},
    {Q_KEY_CODE_RET,0x11,1}, {Q_KEY_CODE_RIGHT,0x11,1},
    {Q_KEY_CODE_COMMA,0x12,1}, {Q_KEY_CODE_DOT,0x12,2},
    {Q_KEY_CODE_BRACKET_LEFT,0x12,4}, {Q_KEY_CODE_BRACKET_RIGHT,0x12,8},
    /* Native screen confirms byte 13 bit 0 is QUANTIZE. */
    {Q_KEY_CODE_Z,0x13,1}, {Q_KEY_CODE_D,0x13,2},
    /* MAIN key 11 reads 09542cfe (field 3c), dispatching BACK; key 12
     * reads 09542d12 (field 40), dispatching ADD_TAG. The other report
     * bits here include track-filter and shortcut controls. */
    {Q_KEY_CODE_ESC,0x13,4}, {Q_KEY_CODE_LEFT,0x13,4},
    {Q_KEY_CODE_T,0x13,8},
    /* MAIN 0949ccbe -> 0954240a records field 48, the SHORTCUT key. */
    {Q_KEY_CODE_S,0x13,0x10},
    /* MAIN 0949ccd6 -> 095423f6 stores the field between TAG (095423e2)
     * and SHORTCUT (0954240a): the TRACK FILTER key. */
    {Q_KEY_CODE_F,0x13,0x20},
    /* MAIN 0949cd06/0949cd1e; stock screen shows range/MT changes. */
    {Q_KEY_CODE_V,0x14,1}, {Q_KEY_CODE_P,0x14,2}, {Q_KEY_CODE_K,0x14,4},
};

/* Report cumulative optical-encoder ticks and their period, not host audio
 * position. MAIN 09560fb2/095610xx converts these native fields into jog
 * motion. One keyboard-held tick every 2 ms; reverse wraps the 16-bit count. */
static void jog_update(XdjPanel *s)
{
    int64_t now = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
    int64_t ticks = (now - s->jog_ns) / 2000000;
    int direction = (int)s->jog_forward - (int)s->jog_reverse;
    if (direction && ticks > 0) {
        s->jog_count += direction * ticks;
        s->jog_ns += ticks * 2000000;
    } else if (!direction) {
        s->jog_ns = now;
    }
}

static void panel_event(DeviceState *dev, QemuConsole *src, InputEvent *evt)
{
    XdjPanel *s = panel;
    unsigned i;
    if (!s) { return; }
    xdj_perf_input();
    if (evt->type == INPUT_EVENT_KIND_KEY) {
        InputKeyEvent *k = evt->u.key.data;
        int code = qemu_input_key_value_to_qcode(k->key);
        jog_update(s);
        if (code == Q_KEY_CODE_H) { s->jog_forward = k->down; }
        if (code == Q_KEY_CODE_N) { s->jog_reverse = k->down; }
        if (code == Q_KEY_CODE_J) {
            /* PANEL c6c0c: P7_3 contact -> immediate bit 4 and debounced
             * bit 5. The host key supplies an already stable contact. */
            if (k->down) { s->keys[0x0f] |= 0x30; }
            else { s->keys[0x0f] &= ~0x30; }
        }
        /* USB STOP is not a panel key: it is MAIN's own PTH4 input. */
        if (code == Q_KEY_CODE_U) { xdj_pfc_usb_stop(k->down); }
        if (code == Q_KEY_CODE_R) {
            /* PANEL c7dfa/c5ed0: REV P6_1 appears both in byte 10 bit 7
             * and as an inverted copy in byte 0f bit 1. */
            if (k->down) { s->keys[0x0f] &= ~2; }
            else { s->keys[0x0f] |= 2; }
        }
        if (k->down) {
            if (code == Q_KEY_CODE_PGUP) { s->pitch = MIN(s->pitch + 512, 65535); }
            if (code == Q_KEY_CODE_PGDN) { s->pitch = MAX((int)s->pitch - 512, 0); }
            if (code == Q_KEY_CODE_HOME) { s->pitch = 0x8000; }
        }
        for (i = 0; i < ARRAY_SIZE(keys); i++) {
            if (keys[i].code == code) {
                if (k->down) { s->keys[keys[i].offset] |= keys[i].mask; }
                else { s->keys[keys[i].offset] &= ~keys[i].mask; }
            }
        }
        if (k->down && (code == Q_KEY_CODE_UP || code == Q_KEY_CODE_DOWN)) {
            s->rotary += code == Q_KEY_CODE_UP ? -1 : 1;
        }
    } else if (evt->type == INPUT_EVENT_KIND_ABS) {
        InputMoveEvent *m = evt->u.abs.data;
        if (m->axis == INPUT_AXIS_X) {
            s->x = qemu_input_scale_axis(m->value, INPUT_EVENT_ABS_MIN, INPUT_EVENT_ABS_MAX, 0, 799);
        } else if (m->axis == INPUT_AXIS_Y) {
            s->y = qemu_input_scale_axis(m->value, INPUT_EVENT_ABS_MIN, INPUT_EVENT_ABS_MAX, 0, 479);
        }
    } else if (evt->type == INPUT_EVENT_KIND_BTN) {
        InputBtnEvent *b = evt->u.btn.data;
        if (b->button == INPUT_BUTTON_LEFT) {
            s->latched |= b->down && !s->want_down;
            s->want_down = b->down;
        }
    }
}

static void panel_defaults(CdjPnlLinkState *link, void *extra, uint8_t *rx)
{
    XDJ_PERF_SCOPE(panel);
    xdj_perf_panel_reply();
    XdjPanel *s = extra;
    unsigned i;
    for (i = 0; i < sizeof s->keys; i++) { rx[i] = s->keys[i]; }
    /* Original PANEL c5e88/c6016: response header 81 00. MAIN 0949c396
     * removes the response bit before recording the header. The link
     * supplies the checksum and trailing 8f delimiter. */
    rx[0] = 0x81;
    rx[1] = 0;
    rx[2] = s->analog[0];
    rx[3] = s->analog[1];
    jog_update(s);
    stw_be_p(rx + 8, s->jog_count);
    int direction = (int)s->jog_forward - (int)s->jog_reverse;
    bool manual = !direction && qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL) < s->manual_end_ns;
    if (direction || manual) {
        /* PANEL c6c80/c8d7a: bit 6 is negative direction, bit 7 motion.
         * Bit 3 latches a touch-triggered movement until release. */
        rx[0x0f] |= 0x80 | ((manual ? s->manual_direction < 0 : direction < 0) ? 0x40 : 0);
        if (s->keys[0x0f] & 0x20) { s->keys[0x0f] |= 8; rx[0x0f] |= 8; }
        /* Native parser divides the period by two. */
        stw_be_p(rx + 10, manual ? s->manual_period : 4000);
    } else {
        stw_be_p(rx + 10, 0xffff);
        if (!(s->keys[0x0f] & 0x20)) { s->keys[0x0f] &= ~8; rx[0x0f] &= ~8; }
    }
    stw_be_p(rx + 4, s->pitch);
    stw_be_p(rx + 6, 0x8000); /* centered reference */
    rx[0x0e] = s->rotary;
    if (s->down && !s->want_down && s->down_frames >= 8) {
        s->down = false; s->up_frames = 0;
    } else if (!s->down && (s->want_down || s->latched) && s->up_frames >= 16) {
        s->down = true; s->latched = false; s->down_frames = 0;
    }
    if (s->down) {
        /* Same 10-bit touch interface as the related NXS2 panel. The
         * inverse default calibration is checked against stock guest input. */
        unsigned x = (23700 + 961 * (794-(int)s->x)) / 790;
        unsigned y = (18760 + 921 * ((int)s->y-5)) / 469;
        stw_be_p(rx+0x16, MAX(x,32)); stw_be_p(rx+0x18, MAX(y,32));
        if (s->down_frames < UINT_MAX) { s->down_frames++; }
    } else if (s->up_frames < UINT_MAX) { s->up_frames++; }
}

static void panel_jog_step(Object *obj, Visitor *visitor, const char *name,
                           void *opaque, Error **errp)
{
    int64_t delta, now, elapsed;
    if (!visit_type_int(visitor, name, &delta, errp)) { return; }
    if (delta < -4096 || delta > 4096) {
        error_setg(errp, "Jog movement must be between -4096 and 4096 ticks");
        return;
    }
    if (!delta) { return; }
    now = qemu_clock_get_ns(QEMU_CLOCK_VIRTUAL);
    elapsed = now - panel->manual_ns;
    jog_update(panel);
    panel->jog_count += delta;
    /* The original PANEL measures pulse period in 2MHz timer units.
     * The GUI supplies optical ticks; MAIN still performs transport/DSP work. */
    panel->manual_period = elapsed > 0 && elapsed < 100000000 ?
        MIN(65535, MAX(2, elapsed / (500 * llabs(delta)))) : 4000;
    panel->manual_ns = now;
    panel->manual_end_ns = now + 100000000;
    panel->manual_direction = delta < 0 ? -1 : 1;
}

static void panel_frame(CdjPnlLinkState *link, void *extra, const uint8_t *frame)
{
    XdjPanel *s = extra;
    memcpy(s->main_frame, frame, sizeof s->main_frame);
    s->main_frames++;
}

/* Raw wire data only: decoding lamp bits is left to the host bridge, so a
 * corrected bit table never needs a new engine. */
static char *panel_main_frame(Object *obj, Error **errp)
{
    GString *json = g_string_new(NULL);
    unsigned i;
    g_string_append_printf(json, "{\"frames\":%" PRIu64 ",\"frame\":\"", panel->main_frames);
    for (i = 0; i < sizeof panel->main_frame; i++) {
        g_string_append_printf(json, "%02x", panel->main_frame[i]);
    }
    g_string_append(json, "\"}");
    return g_string_free(json, FALSE);
}

void xdj_panel_properties(Object *machine)
{
    object_property_add_str(machine, "xdj-panel-frame", panel_main_frame, NULL);
    object_property_add_uint8_ptr(machine, "xdj-panel-analog0", &panel->analog[0],
                                 OBJ_PROP_FLAG_READWRITE);
    object_property_add_uint8_ptr(machine, "xdj-panel-analog1", &panel->analog[1],
                                 OBJ_PROP_FLAG_READWRITE);
    object_property_add_uint16_ptr(machine, "xdj-pitch-slider", &panel->pitch,
                                  OBJ_PROP_FLAG_READWRITE);
    object_property_add(machine, "xdj-jog-step", "int", NULL, panel_jog_step,
                        NULL, NULL);
}

static QemuInputHandler panel_input = {
    .name = "XDJ-1000MK2 panel",
    .mask = INPUT_EVENT_MASK_KEY | INPUT_EVENT_MASK_ABS | INPUT_EVENT_MASK_BTN,
    .event = panel_event,
};

void xdj_panel_init(MemoryRegion *sysmem)
{
    static const CdjPnlLinkHooks hooks = { .on_frame = panel_frame, .build_defaults = panel_defaults };
    panel = cdj_pnl_link_init(sysmem,0xffe20000,"xdj.panel",XDJ_PANEL_FRAME,0x8f,&hooks,sizeof(*panel));
    /* Original PANEL: the reverse-switch copy is active-low, idle high. */
    panel->keys[0x0f] = 2;
    panel->pitch = 0x8000;
    panel->up_frames = 16;
    qemu_input_handler_register(NULL, &panel_input);
}
