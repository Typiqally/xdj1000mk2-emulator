"""Cable edges reach the maintained EtherC interrupt/status implementation."""
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class EtherLinkTests(unittest.TestCase):
    def test_cable_edges_latch_once_and_update_mac_and_phy_status(self):
        source = (ROOT / "native/qemu/hw/cdj/common/cdj_ether.c").read_text()
        constants = source[source.index("#define CDJ_ETHER_SIZE"):source.index("typedef struct {")]
        phy = source[source.index("#define MII_BMCR"):source.index("/*\n * Lead-in bits")]
        phy_read = source[source.index("static void cdj_ether_switch_update_irq("):source.index("static void cdj_ether_mdio_bit(")]
        # Strip the following comment, but compile complete maintained functions.
        phy_read = phy_read[:phy_read.rfind("}")+1]
        irq = source[source.index("static void cdj_ether_update_irq("):source.index("static void cdj_ether_raise(")]
        callback = source[source.index("static void cdj_ether_link_status_changed("):source.index("static NetClientInfo cdj_ether_net_info")]
        poll = source[source.index("static void cdj_ether_link_poll("):source.index("/* CDJ_ETHER_UNBACKED=1")]
        read = source[source.index("static uint64_t cdj_ether_read("):source.index("/* Software reset.")]
        code = r'''
#include <assert.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
typedef uint64_t hwaddr;
''' + constants + phy + r'''
typedef struct {uint32_t reg[CDJ_ETHER_SIZE/4];uint16_t phy[32];bool link_up,mdi,switch_pending;int irq,switch_irq;void *nic,*link_poll;} CdjEtherState;
typedef struct NetClientState {bool link_down;struct NetClientState *peer;CdjEtherState *opaque;} NetClientState;
#define QEMU_CLOCK_VIRTUAL 0
static NetClientState *qemu_get_queue(void *nic){return nic;}
static uint64_t qemu_clock_get_ns(int clock){return 0;}
static void timer_mod(void *timer,uint64_t deadline){}
static int irq_level;
static int switch_level;
static void qemu_set_irq(int irq,int level){if(irq==2)switch_level=level;else irq_level=level;}
static void *qemu_get_nic_opaque(NetClientState *nc){return nc->opaque;}
static bool cdj_ether_backed(void){return true;}
static const uint8_t *cdj_ether_mac_override(void){return NULL;}
static void cdj_ether_trace(CdjEtherState *s,hwaddr off,uint32_t val,bool write){}
''' + phy_read + irq + callback + poll + read + r'''
int main(void){
    CdjEtherState s={.link_up=true,.irq=1};
    NetClientState peer={0},nc={.peer=&peer,.opaque=&s};
    s.reg[CDJ_ETHER_ECSIPR/4]=CDJ_ECSR_LCHNG;
    s.reg[CDJ_ETHER_EESIPR/4]=CDJ_EESR_ECI;
    assert(cdj_ether_read(&s,CDJ_ETHER_PSR,4)==1);
    assert((cdj_ether_phy_read(&s,5,MII_BMSR)&0x24)==0x24);
    nc.link_down=true;cdj_ether_link_status_changed(&nc);
    assert(!s.link_up && irq_level==1);
    assert(cdj_ether_read(&s,CDJ_ETHER_PSR,4)==0);
    assert((cdj_ether_phy_read(&s,5,MII_BMSR)&0x24)==0);
    s.reg[CDJ_ETHER_ECSR/4]=0;cdj_ether_update_irq(&s);
    cdj_ether_link_status_changed(&nc);assert(irq_level==0);
    nc.link_down=false;cdj_ether_link_status_changed(&nc);
    assert(s.link_up && irq_level==1);
    s.reg[CDJ_ETHER_ECSR/4]=0;cdj_ether_update_irq(&s);
    cdj_ether_link_status_changed(&nc);assert(irq_level==0);
    peer.link_down=true;cdj_ether_link_status_changed(&nc);
    assert(!s.link_up && irq_level==1);
    s.switch_irq=2;s.phy[16]=0x8000;s.switch_pending=true;
    cdj_ether_switch_update_irq(&s);assert(switch_level==0);
    s.phy[16]=2;cdj_ether_switch_update_irq(&s);assert(switch_level==1);
    cdj_ether_phy_read(&s,4,22);assert(!s.switch_pending && switch_level==0);
    peer.link_down=false;cdj_ether_link_status_changed(&nc);
    assert(s.switch_pending && switch_level==1);
    s.nic=&nc;cdj_ether_phy_read(&s,4,22);
    peer.link_down=true;cdj_ether_link_poll(&s);
    assert(!s.link_up && s.switch_pending && switch_level==1);
    cdj_ether_phy_read(&s,4,22);cdj_ether_link_poll(&s);
    assert(!s.switch_pending && switch_level==0);
    peer.link_down=false;cdj_ether_link_poll(&s);
    assert(s.link_up && s.switch_pending && switch_level==1);
    return 0;
}
'''
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)
            (path / "test.c").write_text(code)
            subprocess.run(["cc", "-std=c11", str(path / "test.c"), "-o", str(path / "test")], check=True)
            subprocess.run([str(path / "test")], check=True)
