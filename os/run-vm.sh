#!/usr/bin/env bash
# Starts the Nexus OS USB stick in QEMU, as a USB stick, in a UEFI VM with Secure Boot, 6 GB of
# memory and a 64 GB disk to install on. `make os-vm` runs it:
#   os/run-vm.sh [ISO]       default nexus-os-x86_64.iso
# The disk (os-vm.qcow2) and the firmware's variables (os-vm-vars.fd) are made the first time
# and kept, so an install survives the next start; delete them to start over. Needs QEMU and
# Fedora's edk2-ovmf.
set -euo pipefail
iso=${1:-nexus-os-x86_64.iso}
ovmf=/usr/share/edk2/ovmf
[ -f os-vm.qcow2 ] || qemu-img create -f qcow2 os-vm.qcow2 64G
[ -f os-vm-vars.fd ] || cp "$ovmf/OVMF_VARS.secboot.fd" os-vm-vars.fd
exec qemu-system-x86_64 -enable-kvm -machine q35,smm=on -m 6G -smp 4 \
    -global driver=cfi.pflash01,property=secure,value=on \
    -drive if=pflash,format=raw,readonly=on,file="$ovmf/OVMF_CODE.secboot.fd" \
    -drive if=pflash,format=raw,file=os-vm-vars.fd \
    -device qemu-xhci -drive if=none,id=stick,format=raw,file="$iso" \
    -device usb-storage,drive=stick,bootindex=1 \
    -drive file=os-vm.qcow2,if=virtio -nic user,model=virtio
