"""The panel keeps its 60 Hz and the display path adds no pacing of its own."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[2]
LCDC=ROOT/'native/qemu/hw/cdj/boards/xdj1000mk2/lcdc.c'


class DisplayPacingTests(unittest.TestCase):
    def test_vblank_rearms_from_its_deadline_and_resynchronises_after_a_lost_frame(self):
        source=LCDC.read_text()
        body=source[source.index('static void lcdc_vblank('):source.index('static uint64_t lcdc_read(')]
        code=r'''
#include <assert.h>
#include <stdint.h>
#include <stdbool.h>
#define XDJ_VBLANK_NS 16666667
#define QEMU_CLOCK_VIRTUAL 0
typedef struct XdjLcdc { uint32_t reg[0x3000/4]; unsigned bank; uint64_t frames; int vblank; int64_t next_vblank_ns; } XdjLcdc;
static struct { bool enabled; } xdj_system_performance;
static void xdj_perf_vblank(int64_t late) {}
static void lcdc_bank(XdjLcdc *s,unsigned bank) { s->bank=bank&1; }
static void lcdc_irq(XdjLcdc *s) {}
static int64_t now, armed;
static int64_t qemu_clock_get_ns(int clock) { return now; }
static void timer_mod(int timer,int64_t deadline) { armed=deadline; }
''' + body + r'''
int main(void) {
    XdjLcdc s={0}; s.reg[0x474/4]=1; s.next_vblank_ns=XDJ_VBLANK_NS;
    /* Sixty callbacks, each 1.5 ms late, still span exactly sixty periods:
     * lateness must not accumulate into a slower panel. */
    for(int i=1;i<=60;i++) {
        now=(int64_t)i*XDJ_VBLANK_NS+1500000; lcdc_vblank(&s);
        assert(armed==(int64_t)(i+1)*XDJ_VBLANK_NS && s.next_vblank_ns==armed);
    }
    assert(s.frames==60);
    /* A stall longer than a frame drops the missed vblanks instead of
     * firing them in a burst. */
    now=armed+5*XDJ_VBLANK_NS+123; lcdc_vblank(&s);
    assert(armed==now+XDJ_VBLANK_NS && s.frames==61);
    return 0;
}
'''
        with tempfile.TemporaryDirectory() as d:
            src=Path(d)/'vblank.c';src.write_text(code);binary=Path(d)/'vblank'
            build=subprocess.run(['cc','-std=c11','-O2',str(src),'-o',str(binary)],capture_output=True,text=True)
            self.assertEqual(build.returncode,0,build.stderr)
            result=subprocess.run([str(binary)],capture_output=True,text=True)
            self.assertEqual(result.returncode,0,result.stderr)

    def test_scanout_reports_only_rows_the_guest_changed(self):
        source=LCDC.read_text()
        update=source[source.index('static void lcdc_update('):source.index('static const GraphicHwOps')]
        self.assertIn('if (!repaint && !memcmp(row, seen, row_bytes))',update)
        self.assertIn('dpy_gfx_update(s->console, 0, first, width, last - first + 1);',update)
        self.assertNotIn('dpy_gfx_update(s->console, 0, 0, width, height);',update)

    def test_display_server_does_not_throttle_frames(self):
        patch=(ROOT/'native/qemu/patches/ui_vnc.c.patch').read_text()
        self.assertIn('+#define VNC_REFRESH_INTERVAL_BASE 16',patch)
        self.assertIn('-        vd->dcl.update_interval += VNC_REFRESH_INTERVAL_INC;',patch)
        self.assertIn('+    vd->dcl.update_interval = VNC_REFRESH_INTERVAL_BASE;',patch)


if __name__=='__main__':unittest.main()
