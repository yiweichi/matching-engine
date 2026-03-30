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

Profile whether we have brk or mmap on add_order hot path:
```
sudo bpftrace -c './target/release/matching-engine profile --scenario passive-insert --depth 100000 --repeat 2000' -e '
tracepoint:syscalls:sys_enter_mmap /pid == cpid/ {
  printf("\n== mmap ==\n");
  print(ustack(20));
}
tracepoint:syscalls:sys_enter_brk /pid == cpid/ {
  printf("\n== brk ==\n");
  print(ustack(20));
}
tracepoint:syscalls:sys_enter_madvise /pid == cpid/ {
  printf("\n== madvise ==\n");
  print(ustack(20));
}'
[sudo] yege 的密码： 
Attaching 3 probes...

== brk ==

        brk+11
        _dl_start+1422
        0x74f8c15b1548


== mmap ==

        mmap64+44
        __minimal_malloc+177
        _dl_init_paths+140
        dl_main+5767
        _dl_sysdep_start+134
        _dl_start+1422
        0x74f8c15b1548


== mmap ==

        mmap64+44
        _dl_sysdep_read_whole_file+157
        _dl_load_cache_lookup+1304
        _dl_map_object+1243
        openaux+61
        _dl_catch_exception+156
        _dl_map_object_deps+1063
        dl_main+6444
        _dl_sysdep_start+134
        _dl_start+1422
        0x74f8c15b1548


== mmap ==

        mmap64+44
        _dl_map_object_from_fd+1016
        _dl_map_object+569
        openaux+61
        _dl_catch_exception+156
        _dl_map_object_deps+1063
        dl_main+6444
        _dl_sysdep_start+134
        _dl_start+1422
        0x74f8c15b1548


== mmap ==

        mmap64+44
        _dl_map_object_from_fd+1369
        _dl_map_object+569
        openaux+61
        _dl_catch_exception+156
        _dl_map_object_deps+1063
        dl_main+6444
        _dl_sysdep_start+134
        _dl_start+1422
        0x74f8c15b1548


== mmap ==

        mmap64+44
        _dl_map_object_from_fd+1369
        _dl_map_object+569
        openaux+61
        _dl_catch_exception+156
        _dl_map_object_deps+1063
        dl_main+6444
        _dl_sysdep_start+134
        _dl_start+1422
        0x74f8c15b1548


== mmap ==

        mmap64+44
        _dl_map_object_from_fd+1369
        _dl_map_object+569
        openaux+61
        _dl_catch_exception+156
        _dl_map_object_deps+1063
        dl_main+6444
        _dl_sysdep_start+134
        _dl_start+1422
        0x74f8c15b1548


== mmap ==

        mmap64+44
        _dl_map_object_from_fd+1016
        _dl_map_object+569
        openaux+61
        _dl_catch_exception+156
        _dl_map_object_deps+1063
        dl_main+6444
        _dl_sysdep_start+134
        _dl_start+1422
        0x74f8c15b1548


== mmap ==

        mmap64+44
        _dl_map_object_from_fd+1369
        _dl_map_object+569
        openaux+61
        _dl_catch_exception+156
        _dl_map_object_deps+1063
        dl_main+6444
        _dl_sysdep_start+134
        _dl_start+1422
        0x74f8c15b1548


== mmap ==

        mmap64+44
        _dl_map_object_from_fd+1369
        _dl_map_object+569
        openaux+61
        _dl_catch_exception+156
        _dl_map_object_deps+1063
        dl_main+6444
        _dl_sysdep_start+134
        _dl_start+1422
        0x74f8c15b1548


== mmap ==

        mmap64+44
        _dl_map_object_from_fd+1369
        _dl_map_object+569
        openaux+61
        _dl_catch_exception+156
        _dl_map_object_deps+1063
        dl_main+6444
        _dl_sysdep_start+134
        _dl_start+1422
        0x74f8c15b1548


== mmap ==

        mmap64+44
        _dl_map_object_from_fd+4650
        _dl_map_object+569
        openaux+61
        _dl_catch_exception+156
        _dl_map_object_deps+1063
        dl_main+6444
        _dl_sysdep_start+134
        _dl_start+1422
        0x74f8c15b1548


== mmap ==

        mmap64+44
        __minimal_malloc+177
        _dl_allocate_tls_storage+42
        init_tls+192
        dl_main+10731
        _dl_sysdep_start+134
        _dl_start+1422
        0x74f8c15b1548


== brk ==

        brk+11
        __default_morecore+22
        _int_malloc+5767
        tcache_init.part.0+55
        __libc_malloc+294
        _IO_fopen64+31
        pthread_getattr_np+614
        main+551
        __libc_start_call_main+122
        __libc_start_main@GLIBC_2.2.5+139
        _start+37


== brk ==

        brk+11
        __default_morecore+22
        _int_malloc+5767
        tcache_init.part.0+55
        __libc_malloc+294
        _IO_fopen64+31
        pthread_getattr_np+614
        main+551
        __libc_start_call_main+122
        __libc_start_main@GLIBC_2.2.5+139
        _start+37


== mmap ==

        __GI___mmap+44
        std::sys::pal::unix::stack_overflow::imp::make_handler::hc47c11c37bb55b35+132
        main+948
        __libc_start_call_main+122
        __libc_start_main@GLIBC_2.2.5+139
        _start+37


== mmap ==

        __GI___mmap+44
        unix_mmap_prim_aligned.constprop.0+185
        _mi_prim_alloc+162
        mi_os_prim_alloc_at.constprop.0+115
        _mi_os_alloc_aligned+287
        mi_reserve_os_memory_ex+106
        _mi_arena_alloc_aligned+637
        mi_segment_alloc+236
        _mi_segment_page_alloc+505
        mi_page_fresh_alloc+55
        mi_page_queue_find_free_ex+1315
        _mi_malloc_generic+100
        mi_heap_malloc_zero_aligned_at_generic+261
        std::sys::pal::unix::stack_overflow::thread_info::set_current_info::hc81d7a1f277cf2ed+120
        main+971
        __libc_start_call_main+122
        __libc_start_main@GLIBC_2.2.5+139
        _start+37


== madvise ==

        __GI_madvise+11
        mi_os_prim_alloc_at.constprop.0+115
        _mi_os_alloc_aligned+287
        mi_reserve_os_memory_ex+106
        _mi_arena_alloc_aligned+637
        mi_segment_alloc+236
        _mi_segment_page_alloc+505
        mi_page_fresh_alloc+55
        mi_page_queue_find_free_ex+1315
        _mi_malloc_generic+100
        mi_heap_malloc_zero_aligned_at_generic+261
        std::sys::pal::unix::stack_overflow::thread_info::set_current_info::hc81d7a1f277cf2ed+120
        main+971
        __libc_start_call_main+122
        __libc_start_main@GLIBC_2.2.5+139
        _start+37

profile complete: scenario=passive-insert repeat=2000 elapsed=10.80s

== madvise ==

        0x74f8c13250eb
        0x5fa6cdb2be5e
        0x5fa6cdb2c0a7
        0x5fa6cdb2c241
        0x5fa6cdb32dd3
        0x5fa6cdb3601d
        0x74f8c15930f2
        0x74f8c1597578
        0x74f8c1247a76
        0x74f8c1247bbe
        0x74f8c122a1d1
        0x74f8c122a28b
        0x5fa6cda82315

```