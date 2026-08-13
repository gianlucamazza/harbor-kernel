#!/usr/bin/env bash
# Capture runner facts relevant to QEMU starvation without changing any gate.
#
# Keep this deliberately read-only and secret-free: the output is uploaded as
# a CI artifact and is intended to explain an INDETERMINATE boot, not to dump
# the runner environment.
set -euo pipefail

phase="${1:---phase=unspecified}"
case "${phase}" in
	--phase=*) phase="${phase#--phase=}" ;;
	--phase)
		phase="${2:?usage: $0 --phase <name>}"
		;;
	*)
		echo "usage: $0 --phase <name>" >&2
		exit 2
		;;
esac

echo "ci-runner-diagnostics: phase=${phase}"
date --iso-8601=seconds
printf 'hostname='; hostname
printf 'kernel='; uname -a
printf 'nproc='; nproc

if command -v lscpu >/dev/null 2>&1; then
	lscpu | awk -F: '
		$1 ~ /^(CPU\(s\)|On-line CPU|Thread|Core|Socket|Model name)/ {
			gsub(/^[ \t]+|[ \t]+$/, "", $1)
			gsub(/^[ \t]+|[ \t]+$/, "", $2)
			print "lscpu_" $1 "=" $2
		}'
fi

printf 'loadavg='; cat /proc/loadavg
printf 'procstat='; sed -n '1p' /proc/stat

for file in \
	/sys/fs/cgroup/cpu.max \
	/sys/fs/cgroup/cpu.weight \
	/sys/fs/cgroup/cpu.stat \
	/sys/fs/cgroup/cpuset.cpus.effective \
	/sys/fs/cgroup/cgroup.controllers; do
	if [[ -r "${file}" ]]; then
		echo "--- ${file} ---"
		cat "${file}"
	fi
done

if command -v docker >/dev/null 2>&1; then
	echo '--- docker info ---'
	docker info --format 'server={{.ServerVersion}} os={{.OperatingSystem}} kernel={{.KernelVersion}} ncpu={{.NCPU}} cgroup={{.CgroupVersion}}/{{.CgroupDriver}} running={{.ContainersRunning}}' 2>&1 || true
	echo '--- docker version ---'
	docker version --format 'server={{.Server.Version}} client={{.Client.Version}} api={{.Server.APIVersion}}' 2>&1 || true
fi

if command -v ps >/dev/null 2>&1; then
	echo '--- top processes ---'
	ps -eo pid,ppid,comm,pcpu,stat --sort=-pcpu | head -n 16
fi
