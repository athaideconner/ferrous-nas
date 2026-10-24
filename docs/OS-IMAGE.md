# Building a bootable FerrousNAS image

You picked "full OS image." Here's the honest picture and the real path to one.

## Why the image is a host-level build

(Once the image is built, `scripts/build-iso.sh` wraps it in an installer
ISO — see [Installer ISO](#installer-iso).)

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
sudo pacman -S --needed mkosi apt debian-archive-keyring qemu-system-x86 python-pefile
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

`mkosi.conf` selects Debian trixie (with `contrib` enabled — `zfsutils-linux`
lives there, not in `main`, since ZFS's CDDL license doesn't qualify), installs
the real NAS services (`zfsutils-linux`, `samba`, `nfs-kernel-server`,
`docker.io`, `smartmontools`) plus `systemd-boot-efi`, `zfs-dkms` and
`linux-headers-amd64` (needed for a bootable image, for the ZFS kernel module
itself, and for that module to build, respectively — all easy to miss).
`zfs-dkms` in particular fails silently: `zfsutils-linux` only *recommends*
it and mkosi skips recommends, so without it the image builds fine and ships
`zpool`, but there is no `zfs.ko` and every ZFS command fails at runtime. `mkosi.postinst` enables the `ferrous-nasd`
systemd unit, creates the service user, and wires the managed Samba include
line so `FERROUS_SHARES=linux` works without a manual `smb.conf` edit.
Networking is DHCP on every wired interface (systemd-networkd, via
`mkosi.extra/etc/systemd/network/89-ethernet.network`), with
`systemd-resolved` applying the DHCP-provided DNS servers; edit or add
`.network` files there for a static address.

**Why trixie, not bookworm:** the staged `ferrous-nasd` is built on your host
and links dynamically against the host's glibc. A current build host (Ubuntu
24.04 or newer, Arch, etc.) produces a binary needing glibc 2.39, and
bookworm only ships 2.36 — the image would boot but the daemon would fail with
`GLIBC_2.39 not found`. trixie ships 2.41. Check what your build needs with:

```bash
objdump -T backend/target/release/ferrous-nasd | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1
```

It must be 2.41 or lower.

No admin is pre-seeded — visiting the dashboard on first boot runs its own setup
screen, same as any other install. Flash `ferrous-nas.raw` to a disk with `dd`
to run on real hardware.

By default the image has no Linux login account either (console or SSH). To
get one, in the `sudo` group, pass it at build time — only a password hash
goes into the image, and nothing goes into git:

```bash
sudo mkosi --environment=FERROUS_ADMIN_USER=<name> \
           --environment=FERROUS_ADMIN_PASSWORD=<password> -f build
```

`ferrous-console-user.service` creates the account on first boot.

The shipped systemd unit runs every subsystem in its default **mocked** mode
(same as running the binary directly) — add `Environment=FERROUS_TELEMETRY=linux`
etc. to `/etc/systemd/system/ferrous-nasd.service` after boot to turn on the
real backends you want; see the README's env var table. One caveat:
`FERROUS_POWER=systemd` calls `systemctl reboot`/`poweroff` as the
unprivileged `ferrous` account, which most systems will refuse via polkit
until a rule grants it that permission — not something this repo sets up.

### Installer ISO

To install onto a machine the way you would TrueNAS (boot a USB stick or CD,
pick a disk, install), wrap the disk image in an installer ISO:

```bash
sudo apt install xorriso mtools squashfs-tools  # plus the mkosi/dosfstools/zstd from above
cd os-image && sudo mkosi -f build  # the disk image, if not already built
cd .. && sudo ./scripts/build-iso.sh
# -> os-image/ferrous-nas_<version>_installer.iso
```

Use mkosi 25 or newer: older releases (Ubuntu 24.04 ships 20.2) fail on
trixie partway through the build.

Write the ISO to a USB stick with Rufus, balenaEtcher or `dd`, copy it onto a
Ventoy stick, or attach it to a VM as a CD/DVD alongside an empty disk. It is **UEFI-only** (so is the
installed system, which boots with systemd-boot), and unsigned, so turn
**Secure Boot off**.

How it works:

- [`os-image/installer/`](../os-image/installer/) is a second, tiny mkosi
  image built as a UKI — the kernel and the whole installer rootfs in one EFI
  file, running entirely from RAM. `build-iso.sh` places it at
  `EFI/BOOT/BOOTX64.EFI` on an EFI partition inside the ISO, and again at
  the same path in the ISO filesystem — Ventoy and Rufus only look there
  when deciding whether an ISO supports UEFI x64.
- Booted from Ventoy, the ISO is a file on the stick, not a device; the
  installer then finds the `.iso` on the stick's partitions and loop-mounts it.
- The ISO also carries `ferrous-nas.raw.zst`, the compressed disk image, and
  `gui.sqfs`, the graphical installer: a separate mkosi image,
  [`os-image/installer-gui/`](../os-image/installer-gui/), with GTK 4,
  libadwaita and the cage kiosk compositor. It's kept out of the UKI because
  firmware loads the UKI whole into RAM.
- On boot, `ferrous-gui` mounts `gui.sqfs` from the medium and runs the
  wizard full-screen — Welcome, Disk, Account, Summary, Install, Finish, in
  the style of Calamares. It lists the disks (never the one the medium is
  on, nor ones too small), asks for a login username, password and hostname,
  confirms before erasing anything, then writes the image, moves the backup
  GPT header to the end of the disk and grows the root partition and its
  ext4 filesystem to fill it. The installed system creates that login (in
  the `sudo` group) on its first boot.
- The wizard needs no GPU driver: it draws in software on the firmware
  framebuffer. If it can't start anyway, or you pick "Use text installer",
  the same install runs as a text menu on tty1 (`ferrous-install`, which
  also does the actual work for the wizard).
- The installed system is byte-for-byte the mkosi disk image, so everything
  above (first-boot admin setup, mocked-by-default backends) applies
  unchanged. The text menu also has a rescue shell.

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
