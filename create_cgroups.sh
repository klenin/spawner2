#!/bin/sh

CONTROL_GROUPS="sp"
SUBS="blkio cpuacct memory pids freezer"

sudo -v

for cgroup in ${CONTROL_GROUPS}
do
	for sub in ${SUBS}
	do	
		sudo mkdir /sys/fs/cgroup/$sub/$cgroup/
		sudo chown -R ${USER} /sys/fs/cgroup/$sub/$cgroup/
	done
done
