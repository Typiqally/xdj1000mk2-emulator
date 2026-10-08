/* SPDX-License-Identifier: GPL-2.0-or-later */
#include "qemu/osdep.h"
#include "hw/usb.h"
#include "hw/usb/msd.h"
/* A ready USB flash medium, rather than QEMU's power-on SCSI hard disk.
 * 1.45's initial TUR rejects RESET UA (key 6 / ASC 29); no sector I/O has
 * happened at that point. USB reset resets BOT, while this flash medium
 * remains ready. Other sense conditions and all block errors are retained. */
static void flash_reset(USBDevice *dev)
{
    MSDState *s = USB_STORAGE_DEV(dev);
    usb_msd_handle_reset(dev);
    if (s->scsi_dev && s->scsi_dev->unit_attention.key == 6 &&
        s->scsi_dev->unit_attention.asc == 0x29) {
        memset(&s->scsi_dev->unit_attention, 0, sizeof(SCSISense));
    }
    if (s->bus.unit_attention.key == 6 && s->bus.unit_attention.asc == 0x29) {
        memset(&s->bus.unit_attention, 0, sizeof(SCSISense));
    }
}
static void flash_class_init(ObjectClass *klass, void *unused)
{
    USB_DEVICE_CLASS(klass)->handle_reset = flash_reset;
}
static const TypeInfo flash_type = {
    .name = "xdj-usb-flash", .parent = "usb-storage",
    .class_init = flash_class_init,
};
static void flash_register_types(void) { type_register_static(&flash_type); }
type_init(flash_register_types)
