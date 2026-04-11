# p99.99 Tail Latency Investigation

Status: **paused** — strongest remaining hypothesis is USB Legacy Emulation SMI.

## Problem Statement

The matching engine benchmark shows persistent **~18µs outliers at p99.99** in a tight
`rdtsc` loop (`gap-detector` scenario), even after aggressive OS-level tuning. The median
latency is 10ns (back-to-back `rdtsc`), so the outlier is ~1800x the median.

The outliers appear at **exactly 1ms intervals** (1kHz), with a secondary ~15-16µs spike
trailing ~33-37µs after each primary spike.

## Environment

- **CPU:** 4-core, 2592 MHz TSC frequency
- **Kernel boot params:** `isolcpus=domain,managed_irq,1 nohz_full=1 rcu_nocbs=1`
- **Benchmark CPU:** 1 (isolated)
- **Pinning:** `taskset -c 1`

## What Was Eliminated

### 1. SMI counter (MSR 0x34)

```bash
sudo rdmsr -p 1 0x34   # before
# run benchmark
sudo rdmsr -p 1 0x34   # after
```

**Result:** No increase. However, not all BIOS implementations increment this counter
for every SMI type (notably USB legacy SMI often does not).

### 2. NMI

```bash
grep NMI /proc/interrupts   # before and after
```

**Result:** No increase on CPU 1.

### 3. TLB shootdowns

```bash
grep TLB /proc/interrupts   # before and after
```

**Result:** No increase on CPU 1.

### 4. ftrace — all OS-visible events on CPU 1

Ran `scripts/diagnose-tail.sh` which enables these tracepoints filtered to CPU 1:

| Tracepoint | What it covers |
|---|---|
| `irq:irq_handler_entry/exit` | All hardware interrupts |
| `irq:softirq_entry/exit` | All software interrupts |
| `sched:sched_switch/wakeup` | Scheduler preemption |
| `power:cpu_idle/cpu_frequency` | C-state and P-state transitions |
| `timer:hrtimer_expire_entry` | High-resolution timers |
| `timer:timer_expire_entry` | Timer wheel timers |
| `nmi:nmi_handler` | Non-maskable interrupts |

**Result:** Zero events on CPU 1 during the measurement window. The CPU was clean.

### 5. Local APIC timer (irq_vectors:local_timer_entry)

Added a dedicated ftrace run with `irq_vectors:local_timer_entry` to catch scheduler
ticks that don't go through the normal `irq_handler_entry` path.

**Result:** `local_timer_entry` events stopped ~4ms into the `wait_for_nohz()` busy-spin
period, confirming `nohz_full` was engaging before measurement. Zero tick events during
the measurement window itself.

### 6. LOC interrupt delta (definitive)

```bash
grep LOC /proc/interrupts   # before
taskset -c 1 ./target/release/matching-engine bench --scenario gap-detector
grep LOC /proc/interrupts   # after
```

**Result (CPU 1):** LOC delta = **11** over 530ms process lifetime (~89ms measurement
window). But there were **~89 outlier events** at 1ms intervals during measurement.

**Conclusion:** LOC (scheduler tick) accounts for at most 2 of the ~89 periodic outliers.
The 1ms periodicity is NOT caused by the local timer interrupt.

### 7. LOC comparison on CPU 0 (non-isolated)

Ran the same benchmark pinned to CPU 0 (housekeeping CPU, not isolated).

**Result:** CPU 0 also shows 1ms periodic outliers, but with different characteristics:
- 1ms tick itself: ~3-5µs (lighter, just the LOC overhead)
- Additional ~18µs events appear at a subset of 1ms boundaries
- Also has much larger spikes (40-150µs) from other system activity

Both isolated and non-isolated CPUs see the same ~18µs events, confirming the source
is external to the OS scheduler.

### 8. Kernel threads on CPU 1

```
ps -eo pid,psr,comm | awk '$2 == 1'
     23   1 cpuhp/1
     24   1 idle_inject/1
     25   1 migration/1
     26   1 ksoftirqd/1
     27   1 kworker/1:0-events
     28   1 kworker/1:0H-events_highpri
    100   1 kworker/1:1-mm_percpu_wq
   3895   1 kworker/1:1H-kblockd
```

These are per-CPU kernel threads that are bound to CPU 1 and cannot be migrated. ftrace
confirmed they were not scheduled during the measurement window.

### 9. git subprocess interference

Early investigation revealed `git` subprocesses (called for version tagging) inherited
CPU affinity and ran on CPU 1 during benchmark setup. This was fixed by removing the
`git_version()` call entirely. The 18µs outliers persisted after removal.

### 10. wait_for_nohz() duration

Tested busy-spin durations of 3ms, 10ms, and 100ms before measurement to let `nohz_full`
quiesce the tick. No effect on outliers — the tick was already stopping, and the outliers
have a different root cause.

### 11. TSC vs MPERF comparison

```bash
sudo rdmsr -p 1 0x10   # IA32_TSC before
sudo rdmsr -p 1 0xE7   # IA32_MPERF before
# run benchmark
sudo rdmsr -p 1 0x10   # after
sudo rdmsr -p 1 0xE7   # after
```

**Result:** TSC delta ≈ 1,972M cycles, MPERF delta ≈ 1,873M cycles, gap ≈ 99M cycles
(38ms). However, this gap is dominated by C-state idle time outside the benchmark
(shell overhead between `rdmsr` calls), making it impossible to isolate SMI time.

**Verdict:** Inconclusive for SMI detection. Too coarse.

### 12. perf stat (cycles vs ref-cycles)

```bash
perf stat -C 1 -e cycles,ref-cycles,instructions -- taskset -c 1 ... gap-detector
```

**Result:** `cycles > ref-cycles` (515M vs 388M), indicating turbo boost is active.
Aggregate counters cannot isolate individual 18µs spikes. Inconclusive.

## Key Evidence Summary

| Evidence | Value |
|---|---|
| Outlier period | Exactly 1.000ms ± 0.002ms |
| Outlier duration | ~18µs (very stable: 17,900–18,400ns) |
| Secondary spike | ~15-16µs, appears ~33-37µs after primary |
| Affects all CPUs | Yes — both isolated CPU 1 and housekeeping CPU 0 |
| LOC delta on CPU 1 | 11 (vs ~89 outlier events) — **not LOC** |
| ftrace events on CPU 1 | 0 during measurement — **invisible to OS** |
| SMI counter (MSR 0x34) | No increase — but not reliable for all SMI types |
| NMI, TLB shootdowns | No increase |

## Leading Hypothesis: USB Legacy Emulation SMI

All evidence points to **System Management Interrupts generated by USB legacy emulation**
in the BIOS:

1. **1ms period** matches the USB 2.0 frame rate exactly (1kHz)
2. **~18µs duration** is typical for a USB legacy SMI handler
3. **Broadcast to all CPUs** — SMI is non-maskable and enters SMM on every core
4. **Invisible to OS** — SMI suspends the CPU, OS cannot observe it
5. **MSR_SMI_COUNT not updated** — common for USB legacy SMI on many BIOS implementations

USB Legacy Support allows USB keyboards/mice to work in pre-OS environments (BIOS setup,
GRUB). The BIOS SMM handler polls USB controllers via SMI at the USB frame rate. Linux
has its own USB drivers and does not need this mechanism.

## Next Steps (when resuming)

### Step 1: Disable USB Legacy Support in BIOS (highest priority)

1. Reboot → enter BIOS Setup (DEL or F2)
2. Navigate to **Advanced → USB Configuration** (varies by vendor)
3. Set `USB Legacy Support` → **Disabled**
4. Save and exit

Then retest:

```bash
taskset -c 1 ./target/release/matching-engine bench --scenario gap-detector
```

If the 1ms/18µs pattern disappears → **root cause confirmed**.

Disabling USB Legacy does NOT affect USB devices under Linux — the kernel has native
USB drivers (EHCI/xHCI) that work independently of BIOS SMM polling.

### Step 2: If outliers persist after disabling USB Legacy

Try disabling the NVIDIA GPU driver to rule out PCIe DMA interference:

```bash
sudo systemctl stop gdm
sleep 3
sudo modprobe -r nvidia_drm nvidia_modeset nvidia_uvm nvidia
taskset -c 1 ./target/release/matching-engine bench --scenario gap-detector
sudo modprobe nvidia
sudo systemctl start gdm
```

### Step 3: If outliers still persist

The remaining noise floor is likely DRAM refresh or microarchitectural stalls — inherent
to consumer hardware. Server-grade hardware with tuned BIOS (disabled C-states, disabled
SMI sources, memory interleaving) would be the next step.

## Tools Built During Investigation

- **`gap-detector` scenario** (`src/bench/scenarios.rs`): Tight rdtsc loop that stores
  raw timestamps and reports outliers with time offsets for correlation with external traces.
- **`scripts/diagnose-tail.sh`**: One-shot diagnostic that enables ftrace on the benchmark
  CPU, runs gap-detector, and cross-references outlier timestamps with kernel events.
- **`wait_for_nohz()`**: Busy-spin before measurement to let `nohz_full` stop the tick.
