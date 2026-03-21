Profile with perf:
```
perf stat -d target/release/matching-engine profile --scenario passive-insert --depth 100000 --repeat 2000
perf record -F 999 -g -- target/release/matching-engine profile --scenario passive-insert --depth 100000 --repeat 2000
perf report
```


Profile with flamegraph:
```
CARGO_PROFILE_RELEASE_STRIP=none \
CARGO_PROFILE_RELEASE_DEBUG=1 \
RUSTFLAGS="-C force-frame-pointers=yes" \
cargo flamegraph --bin matching-engine -- profile --scenario passive-insert --depth 100000 --repeat 2000
```


Perf stat output:
```
perf stat -e cycles,instructions,branches,branch-misses,L1-dcache-loads,L1-dcache-load-misses \
  target/release/matching-engine profile --scenario passive-insert --depth 100000 --repeat 2000
profile complete: scenario=passive-insert repeat=2000 elapsed=10.59s

 Performance counter stats for 'target/release/matching-engine profile --scenario passive-insert --depth 100000 --repeat 2000':

    46,434,468,452      cpu_atom/cycles/
   197,409,168,379      cpu_atom/instructions/           #    4.25  insn per cycles
    33,766,087,977      cpu_atom/branches/
           630,742      cpu_atom/branch-misses/          #    0.00% of all branches
    36,424,151,050      cpu_atom/L1-dcache-loads/
    36,424,151,050      cpu_core/L1-dcache-loads/
       790,374,029      cpu_atom/L1-dcache-load-misses/  #    2.17% of all L1-dcache accesses
       790,374,029      cpu_core/L1-dcache-load-misses/  #    2.17% of all L1-dcache accesses

      10.692356501 seconds time elapsed

      10.590205000 seconds user
       0.100992000 seconds sys
```


Profile memory allocation with bpftrace to debug high p99 latency:
```
CARGO_PROFILE_RELEASE_STRIP=none cargo build --release
nm -C target/release/matching-engine | rg '__rust_(alloc|realloc|dealloc)|mi_(malloc|realloc|free)'

sudo bpftrace -c './target/release/matching-engine profile --scenario passive-insert --depth 100000 --repeat 2000' -e '
uprobe:./target/release/matching-engine:__rust_alloc   { @alloc = count(); }
uprobe:./target/release/matching-engine:__rust_realloc { @realloc = count(); }
uprobe:./target/release/matching-engine:__rust_dealloc { @dealloc = count(); }
END {
  print(@alloc);
  print(@realloc);
  print(@dealloc);
}'

sudo bpftrace -c './target/release/matching-engine profile --scenario passive-insert --depth 100000 --repeat 2000' -e '
tracepoint:syscalls:sys_enter_mmap    /pid == cpid/ { @mmap = count(); }
tracepoint:syscalls:sys_enter_munmap  /pid == cpid/ { @munmap = count(); }
tracepoint:syscalls:sys_enter_mremap  /pid == cpid/ { @mremap = count(); }
tracepoint:syscalls:sys_enter_brk     /pid == cpid/ { @brk = count(); }
tracepoint:syscalls:sys_enter_madvise /pid == cpid/ { @madvise = count(); }'
Attaching 5 probes...
profile complete: scenario=passive-insert repeat=2000 elapsed=10.87s


@brk: 3
@madvise: 2
@mmap: 14
@mremap: 0
@munmap: 2

```