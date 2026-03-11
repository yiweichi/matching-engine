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

## Core Isolation (bare metal only)

Remove a core from the kernel scheduler entirely so only `taskset` can use it.

```bash
# /etc/default/grub
GRUB_CMDLINE_LINUX="isolcpus=3 nohz_full=3 rcu_nocbs=3"
```

- `isolcpus=3` — scheduler won't place any task on CPU 3
- `nohz_full=3` — disable timer tick on CPU 3 when it has ≤1 runnable task
- `rcu_nocbs=3` — offload RCU callbacks to other cores

After editing, run `update-grub && reboot`.

## Disable Hyper-Threading Sibling

If pinning to a physical core, ensure its HT sibling is idle or offline.

```bash
# Find HT pairs
cat /sys/devices/system/cpu/cpu*/topology/thread_siblings_list | sort -u

# Offline the sibling (e.g. CPU 19 is sibling of CPU 3)
echo 0 > /sys/devices/system/cpu/cpu19/online
```

## Fixed CPU Frequency

Turbo boost and frequency scaling introduce measurement variance.

```bash
# Disable turbo boost (Intel)
echo 1 > /sys/devices/system/cpu/intel_pstate/no_turbo

# Or set performance governor
for f in /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor; do
  echo performance > "$f"
done
```

## Memory Locking

Prevent page faults during execution by locking pages into RAM.

```rust
// In main.rs, before benchmark loop:
unsafe { libc::mlockall(libc::MCL_CURRENT | libc::MCL_FUTURE); }
```

Or from the shell:

```bash
ulimit -l unlimited
```

## NUMA Affinity

On multi-socket machines, ensure memory is allocated on the same NUMA node as the pinned core.

```bash
# Check topology
numactl --hardware

# Run on node 0, pinned to CPU 3
numactl --membind=0 taskset -c 3 target/release/matching-engine
```

## Interrupt Affinity

Move hardware interrupts (NIC, disk) away from the benchmark core.

```bash
# Check current IRQ distribution
cat /proc/interrupts

# Move IRQ 42 to CPU 0 only
echo 1 > /proc/irq/42/smp_affinity
```

## Transparent Huge Pages

THP can cause latency spikes from compaction. Disable for deterministic behavior.

```bash
echo never > /sys/kernel/mm/transparent_hugepage/enabled
echo never > /sys/kernel/mm/transparent_hugepage/defrag
```

## Recommended Hardware

| Tier | Machine | Notes |
|------|---------|-------|
| Budget | AWS c7i.large (2 vCPU) | Good for development, noisy due to virtualization |
| Mid | Hetzner AX42 (bare metal) | Dedicated hardware, affordable |
| Best | AWS c7i.metal (128 vCPU) | Full bare metal, isolcpus, no hypervisor overhead |

## Quick Checklist

```
[ ] CPU pinned (taskset / isolcpus)
[ ] HT sibling offlined or idle
[ ] CPU frequency fixed (performance governor, no turbo)
[ ] Memory locked (mlockall)
[ ] NUMA-local allocation
[ ] IRQs steered away from benchmark core
[ ] THP disabled
```
