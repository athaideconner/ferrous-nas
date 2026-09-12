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
declaratively. The config is in [`../os-image/`](../os-image/), and the app
is already built and staged into `mkosi.extra/` — the only step left is the
one that needs root.

**From an Arch/CachyOS host** (mkosi drives Debian's `apt` to populate the
rootfs, so that needs to be present on the host too):

```bash
sudo pacman -S --needed mkosi apt debian-archive-keyring qemu-system-x86
cd os-image
sudo mkosi build      # -> ferrous-nas.raw (bootable)
sudo mkosi qemu       # boot it in a VM; browse https://<vm-ip>:4200
```

**From a Debian/Ubuntu host:**

```bash
sudo apt install mkosi systemd-container qemu-system-x86 dosfstools zstd
cd os-image
sudo mkosi build
sudo mkosi qemu
```

If you're starting from a fresh checkout rather than one where the app is
already staged, build and stage it first:

```bash
./scripts/build.sh
install -Dm755 backend/target/release/ferrous-nasd \
  os-image/mkosi.extra/usr/local/bin/ferrous-nasd
mkdir -p os-image/mkosi.extra/usr/local/share/ferrous-nas/web
cp -r frontend/dist/* os-image/mkosi.extra/usr/local/share/ferrous-nas/web/
```

`mkosi.conf` selects Debian bookworm (with `contrib` enabled — `zfsutils-linux`
lives there, not in `main`, since ZFS's CDDL license doesn't qualify), installs
the real NAS services (`zfsutils-linux`, `samba`, `nfs-kernel-server`,
`docker.io`, `smartmontools`) plus `systemd-boot-efi` and
`linux-headers-amd64` (needed for a bootable image and for the ZFS kernel
module to actually build, respectively — both are easy to miss and only fail
partway through a real build). `mkosi.postinst` enables the `ferrous-nasd`
systemd unit, creates the service user, and wires the managed Samba include
line so `FERROUS_SHARES=linux` works without a manual `smb.conf` edit. No
admin is pre-seeded — visiting the dashboard on first boot runs its own setup
screen, same as any other install. Flash `ferrous-nas.raw` to a disk with `dd`
to run on real hardware.

The shipped systemd unit runs every subsystem in its default **mocked** mode
(same as running the binary directly) — add `Environment=FERROUS_TELEMETRY=linux`
etc. to `/etc/systemd/system/ferrous-nasd.service` after boot to turn on the
real backends you want; see the README's env var table. One caveat:
`FERROUS_POWER=systemd` calls `systemctl reboot`/`poweroff` as the
unprivileged `ferrous` account, which most systems will refuse via polkit
until a rule grants it that permission — not something this repo sets up.

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
   ✅ **Done** — set `FERROUS_SHARES=linux`.
4. **Pools/datasets** last — this is the destructive one; gate it hard.
   ✅ **Done** — `FERROUS_POOLS=zfs`, dry-run until
   `FERROUS_POOLS_DESTRUCTIVE=i-understand`. See ARCHITECTURE.md.

Also done, outside the original four: **power** (`FERROUS_POWER=systemd`) and
**users/groups** (`FERROUS_USERS=linux`, real Unix/Samba accounts). Every
subsystem this doc originally described as mocked now has a real backend.
