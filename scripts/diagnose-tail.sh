#!/bin/bash
#
# One-shot p99.99 root cause diagnostic.
#
# Captures ALL system events on the benchmark CPU while running gap-detector,
# then cross-references outlier timestamps with kernel events.
#
# Usage:  sudo ./scripts/diagnose-tail.sh [CPU]
# Default CPU = 1

set -euo pipefail

CPU=${1:-1}
CPUMASK=$(printf '%x' $((1 << CPU)))
BIN=./target/release/matching-engine
OUT=/tmp/diagnose-tail

mkdir -p "$OUT"

if [ ! -x "$BIN" ]; then
    echo "Building release binary..."
    cargo build --release
fi

echo "=== p99.99 Root Cause Diagnostic ==="
echo "  benchmark CPU: $CPU"
echo "  output dir:    $OUT"
echo ""

# ── Phase 1: /proc/interrupts snapshot ──────────────────────────
cat /proc/interrupts > "$OUT/irq_before"

# ── Phase 2: Enable ftrace on benchmark CPU ─────────────────────
TRACEFS=/sys/kernel/debug/tracing
if [ ! -d "$TRACEFS" ]; then
    TRACEFS=/sys/kernel/tracing
fi

echo 0    > "$TRACEFS/tracing_on"
echo      > "$TRACEFS/trace"
echo nop  > "$TRACEFS/current_tracer"

# Capture: interrupts, softirqs, scheduler, power states, timers, NMI
echo 1 > "$TRACEFS/events/irq/irq_handler_entry/enable"   2>/dev/null || true
echo 1 > "$TRACEFS/events/irq/irq_handler_exit/enable"    2>/dev/null || true
echo 1 > "$TRACEFS/events/irq/softirq_entry/enable"       2>/dev/null || true
echo 1 > "$TRACEFS/events/irq/softirq_exit/enable"        2>/dev/null || true
echo 1 > "$TRACEFS/events/sched/sched_switch/enable"       2>/dev/null || true
echo 1 > "$TRACEFS/events/sched/sched_wakeup/enable"       2>/dev/null || true
echo 1 > "$TRACEFS/events/power/cpu_idle/enable"           2>/dev/null || true
echo 1 > "$TRACEFS/events/power/cpu_frequency/enable"      2>/dev/null || true
echo 1 > "$TRACEFS/events/timer/hrtimer_expire_entry/enable" 2>/dev/null || true
echo 1 > "$TRACEFS/events/timer/timer_expire_entry/enable" 2>/dev/null || true
echo 1 > "$TRACEFS/events/nmi/nmi_handler/enable"          2>/dev/null || true

# Filter to benchmark CPU only
echo "$CPUMASK" > "$TRACEFS/tracing_cpumask"

# Large buffer to avoid drops
echo 32768 > "$TRACEFS/buffer_size_kb" 2>/dev/null || true

echo 1 > "$TRACEFS/tracing_on"

# ── Phase 3: Run gap-detector ───────────────────────────────────
echo "Running gap-detector on CPU $CPU..."
echo ""
taskset -c "$CPU" "$BIN" bench --scenario gap-detector 2>&1 | tee "$OUT/bench_output.txt"
echo ""

# ── Phase 4: Stop tracing & collect ────────────────────────────
echo 0 > "$TRACEFS/tracing_on"

cat /proc/interrupts > "$OUT/irq_after"
cat "$TRACEFS/per_cpu/cpu${CPU}/trace" > "$OUT/ftrace_cpu${CPU}.txt"

# Disable tracepoints
for evt in \
    irq/irq_handler_entry irq/irq_handler_exit \
    irq/softirq_entry irq/softirq_exit \
    sched/sched_switch sched/sched_wakeup \
    power/cpu_idle power/cpu_frequency \
    timer/hrtimer_expire_entry timer/timer_expire_entry \
    nmi/nmi_handler; do
    echo 0 > "$TRACEFS/events/$evt/enable" 2>/dev/null || true
done

# Reset cpumask to all CPUs
echo "ffffffff" > "$TRACEFS/tracing_cpumask" 2>/dev/null || true

# ── Phase 5: Analyze ───────────────────────────────────────────
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo " RESULTS"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

echo ""
echo "── /proc/interrupts diff (CPU $CPU column) ──"
diff "$OUT/irq_before" "$OUT/irq_after" || true

echo ""
FTRACE_LINES=$(wc -l < "$OUT/ftrace_cpu${CPU}.txt")
echo "── ftrace events on CPU $CPU: $FTRACE_LINES lines ──"

# Count by event type
if [ "$FTRACE_LINES" -gt 5 ]; then
    echo ""
    echo "  Event type breakdown:"
    grep -oP '(?<=: )\w+' "$OUT/ftrace_cpu${CPU}.txt" | sort | uniq -c | sort -rn | head -20
    echo ""
    echo "  First 30 events:"
    grep -v '^#' "$OUT/ftrace_cpu${CPU}.txt" | head -30
else
    echo "  (no events captured — the CPU was clean during the run)"
fi

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo " INTERPRETATION"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo ""
echo "  gap-detector outliers + ftrace events → cross-reference the timestamps."
echo "  If outliers exist but ftrace is empty → hardware-level (microcode/thermal)."
echo "  If irq_handler_entry appears → that interrupt name is your root cause."
echo "  If cpu_idle appears → C-state transitions are the root cause."
echo "  If sched_switch appears → another process preempted you."
echo ""
echo "  Raw data saved to $OUT/"
echo "    bench_output.txt      - gap-detector output with outlier timestamps"
echo "    ftrace_cpu${CPU}.txt  - all kernel events on CPU $CPU"
echo "    irq_before / irq_after - /proc/interrupts snapshots"
