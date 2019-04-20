# Linux DTX Daemon for Surface Books

Linux User-Space Detachment System (DTX) Daemon for the Surface ACPI Driver (and Surface Books).
Currently only the Surface Book 2 is supported, due to lack of driver-support on the Surface Book 1.
This may change in the future.

## About this Package

This package contains two daemons.
A system daemon (`surface-dtx-daemon`) and a per-user daemon (`surface-dtx-userd`):

- The system daemon allows proper clipboard detachment on the Surface Book 2. It allows you to run commands before the clipboard is unlocked, after it has been re-attached, or when the unlocking-process has been aborted (e.g. by pressing the detach-button a second time).
See the configuration section below for details.
Furthermore, this daemon provides a d-bus interface via which you can query the current device mode (i.e. if the device is in tablet-, laptop- or studio-mode).

- The per-user daemon is responsible for desktop-notifications, i.e. it notifies you when the cliboard can be physically detached (i.e. the latch holding it in place is unlocked), and when the re-attachment process has been completed, i.e. indicating when it is fully usable again after re-attachment.
Running this daemon is completely optional, i.e. if you don't want any notifications, you are free to simply not run it.

The split into two daemons is required as notifications can only be sent on a per-user basis.

## Installation

If you have an Arch Linux or Debian based distributions, have a look at the [releases page][releases] for official packages.
After installation you may want to:
- enable the systemd service for the system daemon using `systemctl enable surface-dtx-daemon.service`.
- enable the systemd service for the per-user daemon using `systemctl enable --user surface-dtx-userd.service`.

Alternatively, you can build these packages yourself, using the provided `PKGBUILD` (Arch Linux) or `makedeb.sh` script in the respective `pkg` sub-directories.

## Configuration

The main configuration files can be found under

- `/etc/surface-dtx/surface-dtx-daemon.conf` for the system daemon configuration, and
- `/etc/surface-dtx/surface-dtx-userd.conf` for the per-user daemon configuration.

Here you can specify the handler-scripts for supported events and other options.
All options are explanined in these files, the configuration language is TOML, default attach and detach handler scripts are included. 

Furthermore, a per-user configuration for the user daemon can also be created under `$XDG_CONFIG_HOME/surface-dtx/surface-dtx-userd.conf` (if not set, `$XDG_CONFIG_HOME` defaults to `.config`).


[releases]: https://github.com/qzed/linux-surface-dtx-daemon/releases
