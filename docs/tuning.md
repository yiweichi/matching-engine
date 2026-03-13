# Deployment Tuning Guide

Runtime and OS-level optimizations for low-latency benchmarking and production.

## CPU Pinning

Pin the process to a single core to avoid scheduler migration and cache thrashing.

```bash
# Pin to CPU 1 (leave CPU 0 for kernel housekeeping)
taskset -c 1 target/release/matching-engine

# Or via Makefile
make bench-pin 1
```

**Why avoid CPU 0:** Linux routes timer ticks, hardware IRQs, RCU callbacks, and softirqs
to CPU 0 by default. Pinning your workload elsewhere reduces interrupt-driven tail latency.

## Core Isolation

Remove a core from the kernel scheduler entirely so only `taskset` can use it.
Bare metal only — most cloud VMs don't expose kernel boot params.

### isolcpus (boot parameter)

```bash
# /etc/default/grub
GRUB_CMDLINE_LINUX="isolcpus=2,3"
```

After this, CPUs 2 and 3 are invisible to the scheduler. No process will be placed
on them unless explicitly pinned via `taskset` or `sched_setaffinity`.

Verify after reboot:

```bash
cat /sys/devices/system/cpu/isolated
# expected: 2-3
```

## Tickless Kernel (nohz_full)

By default, Linux fires a timer interrupt (scheduler tick) on every core at HZ frequency
(typically 250 or 1000 Hz). Each tick interrupts your process for ~1-5 μs.

`nohz_full` puts specified cores into adaptive-tick mode: when a core has exactly
1 runnable task, the kernel stops sending ticks to it entirely.

```bash
# /etc/default/grub
GRUB_CMDLINE_LINUX="nohz_full=2,3"
```

Verify:

```bash
cat /sys/devices/system/cpu/nohz_full
# expected: 2-3
```

**Requirements:**
- At least 1 CPU must remain outside `nohz_full` (housekeeping CPU, usually CPU 0)
- Works best combined with `isolcpus` on the same cores
- Only takes effect when exactly 1 task is running on the core

**What it eliminates:**
- Scheduler tick (250-1000 Hz interrupt)
- Load balancing attempts
- Timer wheel processing on that core

## RCU Callback Offloading (rcu_nocbs)

RCU (Read-Copy-Update) is a kernel synchronization mechanism. Periodically, each core
must process RCU callbacks (deferred memory frees). This causes unpredictable latency
spikes of 5-50 μs.

`rcu_nocbs` offloads these callbacks to dedicated kernel threads on other cores.

```bash
# /etc/default/grub
GRUB_CMDLINE_LINUX="rcu_nocbs=2,3"
```

Verify:

```bash
cat /sys/kernel/rcu_nocbs
# expected: 2-3

# Check the offload threads exist
ps -eo pid,comm | grep rcuo
```

**Combined boot parameters** (recommended full setup):

```bash
GRUB_CMDLINE_LINUX="isolcpus=2,3 nohz_full=2,3 rcu_nocbs=2,3"
```

After editing, apply and reboot:

```bash
sudo update-grub && sudo reboot
```

## IRQ Affinity

Hardware interrupts (NIC, disk, timer) preempt user-space code. Steer them away
from the benchmark core.

### Check current distribution

```bash
# Show interrupt counts per CPU
cat /proc/interrupts

# Show affinity for a specific IRQ
cat /proc/irq/42/smp_affinity_list
```

### Set affinity

```bash
# Move IRQ 42 to CPU 0 only (bitmask: 0x1)
echo 1 > /proc/irq/42/smp_affinity

# Or using CPU list format
echo 0 > /proc/irq/42/smp_affinity_list
```

### Bulk move all IRQs off benchmark cores

```bash
# Move all IRQs to CPU 0
for irq in $(ls /proc/irq/); do
  [ -w "/proc/irq/$irq/smp_affinity_list" ] && \
    echo 0 > "/proc/irq/$irq/smp_affinity_list" 2>/dev/null
done
```

### irqbalance

The `irqbalance` daemon redistributes IRQs dynamically. Disable it to prevent
it from overriding your manual affinity settings.

```bash
sudo systemctl stop irqbalance
sudo systemctl disable irqbalance
```

### Practical rule

For low-latency runs:

- Disable `irqbalance`
- Manually pin device IRQs to housekeeping CPUs
- Keep the benchmark core out of the IRQ mask

You can move most device interrupts away, but not eliminate all interrupts on
that core. Local timer events, IPIs, NMIs, and other per-CPU kernel work still
exist. The goal is "almost no device IRQ noise", not a literally interrupt-free
Linux core.

## Disable Hyper-Threading Sibling

Two HT threads on the same physical core share execution units, L1/L2 cache,
and TLB. If the sibling is active, it steals resources from your benchmark.

```bash
# Find HT pairs
cat /sys/devices/system/cpu/cpu*/topology/thread_siblings_list | sort -u
# e.g. "0,16" means CPU 0 and CPU 16 share a physical core

# Offline the sibling (e.g. CPU 19 is sibling of CPU 3)
echo 0 > /sys/devices/system/cpu/cpu19/online
```

Or disable HT system-wide via BIOS / kernel boot parameter:

```bash
GRUB_CMDLINE_LINUX="nosmt"
```

Use this selectively. Disabling SMT often helps tail latency when the sibling
thread is busy, but it reduces total throughput and is not always necessary if
the sibling is already idle or isolated.

## Fixed CPU Frequency

Turbo boost and frequency scaling introduce measurement variance.
A single turbo ramp-up/ramp-down can cause 10-20 μs of latency jitter.

```bash
# Disable turbo boost (Intel pstate driver)
echo 1 > /sys/devices/system/cpu/intel_pstate/no_turbo

# Disable turbo boost (generic cpufreq)
echo 0 > /sys/devices/system/cpu/cpufreq/boost

# Set performance governor (lock to max non-turbo frequency)
for f in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do
  echo performance > "$f"
done

# Verify frequency is fixed
watch -n1 "cat /proc/cpuinfo | grep 'cpu MHz'"
```

## Memory Locking (mlockall)

Prevent page faults during execution by locking all pages into RAM.
A single minor page fault costs ~1-5 μs; a major fault (disk I/O) costs milliseconds.

```rust
// In main.rs, before benchmark loop:
unsafe { libc::mlockall(libc::MCL_CURRENT | libc::MCL_FUTURE); }
```

Prerequisites:

```bash
# Allow the process to lock memory
ulimit -l unlimited

# Or set system-wide in /etc/security/limits.conf
*  hard  memlock  unlimited
*  soft  memlock  unlimited
```

## Transparent Huge Pages (THP)

THP can cause latency spikes from compaction (the kernel rearranging physical memory
to form contiguous 2MB pages). A single compaction event can stall for 100+ μs.

```bash
# Disable THP
echo never > /sys/kernel/mm/transparent_hugepage/enabled
echo never > /sys/kernel/mm/transparent_hugepage/defrag

# Verify
cat /sys/kernel/mm/transparent_hugepage/enabled
# expected: always madvise [never]
```

To persist across reboots, add to `/etc/rc.local` or a systemd unit.

## Explicit Huge Pages

Usually not worth enabling for this matching engine.

- The hot path is pointer-heavy (`HashMap`, `BTreeMap`, arena linked list), not a large contiguous memory scan
- The working set is typically small enough that TLB pressure is not the main bottleneck
- Huge pages can increase memory waste and operational complexity

Recommendation:

- Disable `THP` for better latency stability
- Do not add explicit huge page allocation unless profiling shows real TLB-miss pressure

## NUMA Affinity

On multi-socket machines, each socket has its own memory controller.
Cross-NUMA memory access adds 40-100 ns latency per access.

```bash
# Check topology
numactl --hardware

# Run on node 0's memory, pinned to CPU 3
numactl --membind=0 taskset -c 3 target/release/matching-engine

# Or bind both CPU and memory to node 0
numactl --cpunodebind=0 --membind=0 target/release/matching-engine
```

## Recommended Hardware

| Tier | Machine | Notes |
|------|---------|-------|
| Budget | AWS c7i.large (2 vCPU) | Good for development, noisy due to virtualization |
| Mid | Hetzner AX42 (bare metal) | Dedicated hardware, affordable |
| Best | AWS c7i.metal (128 vCPU) | Full bare metal, isolcpus, no hypervisor overhead |

## Quick Checklist

```
[ ] CPU pinned (taskset -c / cpuset cgroup)
[ ] Core isolated (isolcpus or cpuset partition)
[ ] Tickless (nohz_full on benchmark core)
[ ] RCU offloaded (rcu_nocbs on benchmark core)
[ ] IRQs steered to CPU 0 (smp_affinity, irqbalance off)
[ ] HT sibling offlined or nosmt
[ ] CPU frequency fixed (performance governor, no turbo)
[ ] Memory locked (mlockall, ulimit -l unlimited)
[ ] NUMA-local allocation (numactl --membind)
[ ] THP disabled
```

## One-Shot Setup Script (bare metal Linux)

```bash
#!/bin/bash
set -e
CPU=3                # benchmark core
SIBLING=19           # HT sibling of $CPU (check topology first)

# Offline HT sibling
echo 0 > /sys/devices/system/cpu/cpu${SIBLING}/online

# Performance governor
for f in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do
  echo performance > "$f"
done

# Disable turbo
echo 1 > /sys/devices/system/cpu/intel_pstate/no_turbo 2>/dev/null || \
echo 0 > /sys/devices/system/cpu/cpufreq/boost 2>/dev/null || true

# Disable THP
echo never > /sys/kernel/mm/transparent_hugepage/enabled
echo never > /sys/kernel/mm/transparent_hugepage/defrag

# Move all IRQs to CPU 0
for irq in $(ls /proc/irq/); do
  [ -w "/proc/irq/$irq/smp_affinity_list" ] && \
    echo 0 > "/proc/irq/$irq/smp_affinity_list" 2>/dev/null
done

# Stop irqbalance
systemctl stop irqbalance 2>/dev/null || true

# Unlock memory
ulimit -l unlimited

echo "Ready. Run: taskset -c $CPU target/release/matching-engine"
```
