#!/usr/bin/env sh

# unmount all USB devices
for usb_dev in /dev/disk/by-id/usb-*
do
    dev=$(readlink -f $usb_dev)
    mount -l | grep -q "^$dev\s" && umount "$dev"
done

# signal commence
exit $EXIT_DETACH_COMMENCE
