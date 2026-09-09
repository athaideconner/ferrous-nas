# Building a bootable FerrousNAS image

You picked "full OS image." Here's the honest picture and the real path to one.

## Why the repo doesn't just spit out an ISO

A bootable OS image is a **host-level** artifact, not an application build:

- It needs **root** and **loopback / device-mapper** access to partition a raw
  image, install a bootloader, and populate a root filesystem.
- It bundles a **kernel, init system, and bootloader** — you assemble an
  existing distro (Debian, here), you don't compile an OS from this repo.
- Restricted/CI sandboxes (including the one this project was scaffolded in)
  don't grant those privileges, so the image is built on your own Linux host.

So FerrousNAS follows how TrueNAS SCALE and CasaOS actually ship: TrueNAS builds
a **Debian-based image** with its middleware baked in; CasaOS is an **installer
script** layered onto an existing Linux. This repo gives you both options.

## Option A — bake an appliance image with mkosi (recommended)

[`mkosi`](https://github.com/systemd/mkosi) builds a bootable Debian image
declaratively. The config is in [`../os-image/`](../os-image/).

On a Linux host (a VM is fine):

```bash
sudo apt install mkosi systemd-container qemu-system-x86 dosfstools zstd
# 1) build the app first, so the binary + web assets exist to copy in
./scripts/build.sh
# 2) stage them into the image overlay
install -Dm755 backend/target/release/ferrous-nasd \
  os-image/mkosi.extra/usr/local/bin/ferrous-nasd
mkdir -p os-image/mkosi.extra/usr/local/share/ferrous-nas/web
cp -r frontend/dist/* os-image/mkosi.extra/usr/local/share/ferrous-nas/web/
# 3) build + boot the image
cd os-image
sudo mkosi build      # -> ferrous-nas.raw (bootable)
sudo mkosi qemu       # boot it in a VM; browse http://<vm-ip>:4200
```

`mkosi.conf` selects Debian bookworm, installs the real NAS services
(`zfsutils-linux`, `samba`, `nfs-kernel-server`, `docker.io`, `smartmontools`),
and `mkosi.postinst` enables the `ferrous-nasd` systemd unit and creates the
service user. Flash `ferrous-nas.raw` to a disk with `dd` to run on real
hardware.

## Option B — the CasaOS-style installer

Skip images entirely: install onto an existing Debian/Ubuntu box.

```bash
./scripts/build.sh
sudo ./scripts/install.sh    # installs the binary, web assets, systemd unit
```

This is the fastest way to run FerrousNAS "like an appliance" on hardware you
already have (an old PC, a Pi with Debian, a VM).

## Turning the mock into a real NAS OS

The image installs the real services; the daemon just doesn't drive them yet.
The migration path — one subsystem at a time, behind a backend trait — is in
[ARCHITECTURE.md](ARCHITECTURE.md#from-mock-to-real). Recommended order:

1. **Read-only real telemetry** — system stats, `lsblk`, `smartctl` (safe).
   ✅ **Done** — set `FERROUS_TELEMETRY=linux`. See ARCHITECTURE.md.
2. **Docker apps** via the Docker socket (high value, low risk).
   ✅ **Done** — set `FERROUS_APPS=docker`.
3. **Shares** — generate `smb.conf` / `exports`, reload the services.
4. **Pools/datasets** last — this is the destructive one; gate it hard.
