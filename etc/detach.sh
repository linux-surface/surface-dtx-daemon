#!/usr/bin/env sh
# surface-dtx detachment handler

# unmount all USB devices
for usb_dev in /dev/disk/by-id/usb-*
do
    dev=$(readlink -f $usb_dev)
    mount -l | grep -q "^$dev\s" && umount "$dev"
done

# signal commence
exit $EXIT_DETACH_COMMENCE
# The exit signal determines the continuation of the detachment-procedure. A
# value of EXIT_DETACH_COMMENCE (0/success), causes the detachment procedure
# to open the latch, while a value of EXIT_DETACH_ABORT (1, or any other
# non-zero value) will cause the detachment-procedure to be aborted. On an
# abort caused by this script, the detach_abort handler will _not_ be
# executed. It is therefore the the responsibility of this handler-executable
# to ensure the device state is properly reset to the state before its
# execution, if required.
