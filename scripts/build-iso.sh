#!/usr/bin/env bash
# Wrap the FerrousNAS disk image in a bootable installer ISO.
#
# Prerequisite: the disk image itself, built with
#     cd os-image && sudo mkosi -f build
#
# Then, from the repo root:
#     sudo ./scripts/build-iso.sh
#
# Produces os-image/ferrous-nas_<version>_installer.iso: a UEFI-bootable
# hybrid ISO (burn it, or write it to a USB stick with Rufus/Etcher/dd). It
# boots a small RAM-only installer (os-image/installer/) that writes the disk
# image to a disk you choose and grows it to fill that disk.
#
# Host packages: mkosi xorriso mtools dosfstools zstd
set -euo pipefail
cd "$(dirname "$0")/.."

ISO_LABEL=FERROUS_NAS   # must match ISO_LABEL in installer/.../ferrous-install

PATH=$PATH:/usr/sbin:/sbin   # mkfs.fat lives in sbin
for tool in mkosi xorriso mcopy mmd mkfs.fat zstd; do
    command -v "$tool" >/dev/null || {
        echo "missing $tool — install: sudo apt install mkosi xorriso mtools dosfstools zstd" >&2
        exit 1
    }
done

# Newest os-image/ferrous-nas_<version>.raw, unless IMAGE= names one.
IMAGE=${IMAGE:-$(ls -t os-image/ferrous-nas_*.raw 2>/dev/null | head -n1 || true)}
[ -n "$IMAGE" ] || {
    echo "no os-image/ferrous-nas_*.raw — build it first: cd os-image && sudo mkosi -f build" >&2
    exit 1
}
VERSION=$(basename "$IMAGE" .raw); VERSION=${VERSION#ferrous-nas_}
OUT=os-image/ferrous-nas_${VERSION}_installer.iso

echo "==> Building the installer environment (os-image/installer/)"
mkosi -C os-image/installer -f build
UKI=$(ls -t os-image/installer/ferrous-nas-installer_*.efi | head -n1)

# On disk next to the outputs, not /tmp: the compressed image is ~1 GB and
# /tmp is often RAM-backed.
WORK=$(mktemp -d -p os-image .iso-work.XXXXXX)
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$WORK/iso"

echo "==> Compressing $IMAGE"
zstd -T0 -6 -q -o "$WORK/iso/ferrous-nas.raw.zst" "$IMAGE"
stat -c %s "$IMAGE" > "$WORK/iso/ferrous-nas.size"
echo "$VERSION" > "$WORK/iso/VERSION"

echo "==> Building the EFI boot partition"
# FAT image holding the installer UKI at the removable-media fallback path,
# so any UEFI firmware boots it without an NVRAM entry. Sized to the UKI plus
# headroom; FAT32 needs at least ~33 MiB.
uki_mib=$(( ($(stat -c %s "$UKI") + 1048575) / 1048576 ))
efi_mib=$(( uki_mib + 16 < 40 ? 40 : uki_mib + 16 ))
mkdir -p "$WORK/iso/boot"
mkfs.fat -C -F 32 -n FERROUS_EFI "$WORK/iso/boot/efiboot.img" $((efi_mib * 1024)) >/dev/null
mmd -i "$WORK/iso/boot/efiboot.img" ::/EFI ::/EFI/BOOT
mcopy -i "$WORK/iso/boot/efiboot.img" "$UKI" ::/EFI/BOOT/BOOTX64.EFI

echo "==> Writing $OUT"
# UEFI-only hybrid ISO (the installed image is UEFI-only too — it boots via
# systemd-boot). The FAT image is the El Torito EFI boot image for optical
# boot, and -efi-boot-part --efi-boot-image also exposes it (without a
# second copy) as a GPT EFI system partition, so the same ISO boots from a
# USB stick.
#
# It must live INSIDE the ISO filesystem, not be appended after it: at ~190
# MB it exceeds El Torito's 32 MB size field, which is then written as 0, and
# firmware (OVMF, at least) falls back to "from the image to the end of the
# ISO volume" — which is 0 bytes for an appended image, so nothing boots.
xorriso -as mkisofs -quiet \
    -iso-level 3 -joliet -rational-rock \
    -volid "$ISO_LABEL" \
    -e boot/efiboot.img -no-emul-boot -efi-boot-part --efi-boot-image \
    -o "$OUT" "$WORK/iso"
echo
echo "Done: $OUT ($(du -h "$OUT" | cut -f1))"
echo "  Write to USB:  Rufus/balenaEtcher, or  sudo dd if=$OUT of=/dev/sdX bs=4M conv=fsync"
echo "  Try in a VM:   UEFI firmware, Secure Boot off, attach as CD/DVD plus an empty disk"
