# Marketplace spike findings

The feasibility spikes for the plugin marketplace (design
`docs/marketplace/architecture.md`, PR #191) lived in
`experiments/marketplace-spikes/` (`runtime/`, spikes 1 to 5, and `trust/`,
spikes 4 and 5 of the trust model) with their own CI workflow
`marketplace-spikes.yml`. Their code was promoted to `plugins/runtime/`,
`crates/deckpilot-plugin-package` and `crates/deckpilot-plugin-registry`,
and the spikes were then removed. The last commit that contains them is
`8cb5ace` (`git show 8cb5ace:experiments/marketplace-spikes/runtime/RESULTS.md`).

This page keeps what is not recorded anywhere else: measurements, findings
and rejected alternatives. It does not repeat:

- `plugins/runtime/README.md`: the fixes relative to the spike (exit after
  any trap, epoch reset before re-entry, no file access in the host,
  self-applied sandbox, Linux and Windows sandboxes), the verification
  status and the QuickJS check (`rquickjs` 0.14.0 for `wasm32-wasip1`,
  788 KB, eight WASI imports, 517 ms compile, 0.25 to 0.5 ms per
  evaluation).
- `crates/deckpilot-plugin-registry/README.md`, "Decisions on the spike's
  findings": index expiry, freeze window, lifting revocations, key set
  deployment order, `seq_floor`, root replacement, clock resets,
  unauthenticated local state.
- `crates/deckpilot-plugin-package/README.md`: determinism and the reader
  checks.

All spike numbers are from one machine (Apple M4 Pro, macOS 27.2, Rust
1.99.1, release builds, wasmtime 50.0.1) and one run each: orders of
magnitude, not benchmarks.

## Runtime

### Measurements (spike)

The spike guest was a 16 KB Rust module; the spike host started a fresh
Store per invocation and was launched through `sandbox-exec`.

| | Value |
| --- | --- |
| Guest compile (Cranelift, 16 KB module) | 16-18 ms |
| Fresh Store + instantiate + one call, in process | 11 µs median, 17 µs p99 |
| guest → `host_call` → guest, in process | 0.2 µs per call |
| Epoch deadline 250 ms (10 ms ticker, 25 ticks), observed stop | 300-305 ms |
| Fuel, 50 M units, infinite loop stopped after | about 2 ms |
| Memory bomb (1 MiB chunks) stopped at the 64 MiB cap | 9-11 ms, at 63 MiB of linear memory |
| Host process RSS after ready / after 3,000 invocations and a bomb | 12.8 MiB / 12.8 MiB |
| Host release binary (unstripped, with Cranelift) | 13 MB |

Round trips over the pipes (1,000 iterations each):

| Round trip | p50 | p95 | p99 |
| --- | --- | --- | --- |
| ping/pong | 9 µs | 13-20 µs | 16-64 µs |
| `invoke add()` (new Store per invocation) | 26-28 µs | 32-46 µs | 42-99 µs |
| guest → host → broker → file read → back (allowed) | 81-83 µs | 127-130 µs | 180-190 µs |
| guest → host → broker, `http.fetch` denied | 44-46 µs | 53-55 µs | 63-73 µs |
| one capability call inside an invocation (1,000 per invocation) | 44-49 µs mean | | |
| spawn until `ready`, unsandboxed (20 spawns) | 17-24 ms | 18-26 ms | first spawn 200-340 ms |
| spawn until `ready` under `sandbox-exec` (20 spawns) | 25-36 ms | 28-50 ms | 28-50 ms |

The first spawn of a freshly built binary was an outlier (200-340 ms,
probably the system's first-launch scan). Start-up was dominated by
compiling the guest; precompiling at build time removes most of it. The
sandbox added about 8-12 ms per spawn.

### Measurements (production runtime)

The spike's `ipc-bench` binary is replaced by an ignored test that drives
the real host, supervisor and broker:

```sh
cd plugins/runtime
cargo test --locked --release -p deckpilot-plugin-host --test ipc_bench -- --ignored --nocapture
```

One run (same machine, 2026-11-15, host sandboxed with `macos-seatbelt`,
`full`; the 118 KB test guest; stub executor, no network):

| Round trip | mean | p50 | p95 | p99 |
| --- | --- | --- | --- | --- |
| spawn + sandbox + load (compile) until ready, then shut down (20) | 123 ms | 107 ms | 110 ms | 437 ms (first spawn) |
| `invoke add` (no capability call) | 13.6 µs | 13.1 µs | 16.8 µs | 21.1 µs |
| `invoke fetch`, allowed by the broker | 30.6 µs | 29.3 µs | 36.5 µs | 50.0 µs |
| `invoke fetch`, denied by the broker | 28.6 µs | 28.0 µs | 34.8 µs | 38.3 µs |

Start-up is several times the spike's because the module is larger and is
compiled in the host at every load; precompilation is still not done.

### Findings

- **Rust std in the guest.** On `wasm32-unknown-unknown`, `fs::read`,
  `fs::write`, `read_dir`, `env::args`, `TcpStream::connect` and
  `Command::output` return an error, while `env::vars` and
  `SystemTime::now` panic (a trap). Neither reaches the host. The
  production test `rust_std_has_no_ambient_authority_in_the_guest` accepts
  both outcomes for this reason.
- **Unbounded recursion** in a Rust guest showed up as `MemoryOutOfBounds`,
  not as a stack overflow: the Rust shadow stack in linear memory overflows
  before the native wasm stack does. A WAT guest recursing without a shadow
  stack gets `StackOverflow` (production `deep_recursion_traps`), so a host
  must treat every trap kind as fatal, which it does.
- **Epoch reset order.** The deadline has to be reset when `host_call`
  returns *before* the host calls back into the guest (`guest_alloc`, now
  `dp_alloc`), or the guest traps on re-entry after a slow host call.
- **`sandbox-exec` needed a read of `/`.** With a strictly path-scoped read
  allowlist the spike host died with SIGABRT before `main`; bisecting
  showed the missing rule was `file-read-data` on `/` itself (who reads it,
  dyld or the Rust runtime, was not determined). The production host
  applies `sandbox_init` with `(deny default)` to itself after start-up, so
  it needs no file rules at all.
- **Cranelift JIT** ran under the Seatbelt profile for an unsigned,
  non-hardened binary. A signed binary with the hardened runtime
  (`com.apple.security.cs.allow-jit`) and notarization are still untested.
- **IPC is not a sandbox.** The same host process without the OS sandbox
  read the user's files, listed the home directory, connected to a local
  TCP port and a local Unix socket and executed `/bin/echo`. A separate
  process gives crash isolation only; the probe tests always run that
  control.

### Rejected alternatives

- **A file-system capability** (`fs.read` below a granted directory). The
  spike implemented it with `canonicalize` followed by an open by path,
  which leaves a time-of-check/time-of-use window. A real one would open
  relative to a directory handle (`cap-std`, `openat2` with
  `RESOLVE_BENEATH`). The design makes the file system never grantable in
  API 1 and the production broker refuses the kind, so none was built.
- **`sandbox-exec`** (deprecated, undocumented, launcher-applied): replaced
  by the self-applied `sandbox_init`.
- **Keeping the host process after a trap** (discard the Store only): the
  design requires the process to exit; production does.
- **QuickJS through Javy**: Javy's CLI is a prebuilt third-party binary and
  its output imports WASI. See the runtime README for the `rquickjs` check.

## Trust (packages and registry)

### Measurements (spike)

Package verification (median of 51 runs):

| Package | Size | size + sha256 | + publisher signature | Full verify (sha256, gunzip, untar, manifest) | Full + publisher signature |
| --- | --- | --- | --- | --- | --- |
| 4.9 MiB incompressible asset | 5,018,931 B | 8.8 ms | 12.2 ms | 10.4 ms | 14.2 ms |
| 2 MiB script + 3 MiB asset | 3,350,900 B | 5.9 ms | 8.2 ms | 7.7 ms | 10.3 ms |

A deterministic build of the 5 MiB package took 66 to 73 ms. Verification
cost is negligible next to the download.

Registry refresh:

| Index | Size | First refresh | Unchanged refresh |
| --- | --- | --- | --- |
| 100 plugins | 46 KB | 0.7 ms | 0.2-0.5 ms |
| 1,000 plugins | 407 KB | 2.0 ms | 0.2-0.5 ms |
| 3,000 plugins | 1.2 MB | 5.0 ms | 0.2-0.5 ms |

At about 400 bytes per plugin, the 4 MiB index cap allows about 10,000
plugins. The persisted trust state was about 1.4 KB whatever the index
size.

### Findings and open questions

- **Publisher keys (U-2)** were tested as an optional extension: a key per
  namespace pinned in the reviewer-gated index and a per-version minisign
  signature with the comment
  `deckpilot-plugin v1 id:<id> version:<v> sha256:<hex>`. They worked
  (wrong key, signature of another file, comment naming another version or
  plugin, and a missing signature with a pinned key were all refused) and
  cost about 3.4 ms per 5 MiB. Adopting them needs schema changes
  (`publishers[].signing_key`, `versions[].publisher_signature`; both
  schemas forbid extra properties) and a key loss and rotation story per
  publisher. They protect only against a registry compromise that leaves
  the publisher's own key intact. Not implemented in the production
  crates; the spike's tests are not ported because there is no API to test.
  Still an owner decision.
- **Root threshold (U-10).** 1-of-2 and 2-of-2 both work (production test
  `a_threshold_counts_distinct_keys`). With a single maintainer, 2-of-2
  only helps if the two keys live on different media or in different
  places. minisign keys cannot live on a hardware token. Still an owner
  decision.
- **Index and timestamp thresholds** above 1 are supported, but a
  threshold above 1 on the unattended timestamp role has no benefit.
- **Signing tool in CI.** Feeding a password to the `minisign` CLI
  non-interactively was not verified, so the spike signed with its own Rust
  code. Production follows that: `deckpilot-registry` (feature `publish`)
  signs with `--password-env`.
- **Packer.** Determinism was shown for the Rust packer (`tar` + `flate2`),
  not for a Node builder. The gzip OS header byte has to be pinned (255);
  Node and zlib may write 3 or another value per platform. Cross-tool
  reproducibility is still spike S6.
- **Not covered by the spike, still open:** real HTTP (redirects, streaming
  abort at the declared size, `If-None-Match`, GitHub Pages caching,
  release-asset redirect hosts).

## Test mapping

Where each spike test's property is now tested. Paths are relative to
`plugins/runtime/crates/` for the runtime and to `crates/` for trust.

### Runtime: `isolation.rs`

| Spike test | Production test |
| --- | --- |
| `guest_imports_only_host_call` | `host/tests/engine.rs` `only_the_abi_import_is_linked` (the host refuses any other import) |
| `guest_std_has_no_ambient_authority` | `engine.rs` `rust_std_has_no_ambient_authority_in_the_guest` |
| `modules_importing_wasi_do_not_instantiate` | `engine.rs` `only_the_abi_import_is_linked` (the same seven WASI imports and more) |
| `fuel_stops_infinite_loop` | `engine.rs` `infinite_loop_runs_out_of_fuel`, `fuel_is_per_invocation` |
| `epoch_deadline_stops_infinite_loop` | `engine.rs` `infinite_loop_hits_the_epoch_deadline`, `start_function_runs_under_the_deadline` |
| `epoch_deadline_resets_while_host_works` | `engine.rs` `time_in_the_host_does_not_count_against_the_deadline`; `host/tests/process.rs` `slow_capability_calls_do_not_trip_the_deadline` |
| `memory_bomb_stopped_by_store_limits` | `engine.rs` `guest_traps_are_faults` (`memory_bomb`) |
| `raw_memory_grow_is_capped` | `engine.rs` `memory_growth_stops_at_the_cap`, `guest_traps_are_faults` (`grow_raw`) |
| `declared_memory_above_limit_does_not_instantiate` | `engine.rs` `structural_limits` |
| `traps_do_not_crash_the_host` | `engine.rs` `guest_traps_are_faults`, `deep_recursion_traps`; `process.rs` `host_exits_after_any_trap` (stricter: the process exits) |
| `host_call_rejects_out_of_bounds_pointers` | `engine.rs` `hostile_host_calls_are_guest_abi_violations`, `hostile_alloc_is_a_guest_abi_violation` |

### Runtime: `ipc.rs`

| Spike test | Production test |
| --- | --- |
| `ready_handshake` | `supervisor/tests/fake_host.rs` `handshake_failures`, `normal_host_works`; `process.rs` `host_starts_sandboxed_and_serves_invocations` |
| `ping_1000_round_trips` | measurement only: `host/tests/ipc_bench.rs` (ignored) |
| `guest_runs_in_host_process` | `process.rs` `host_starts_sandboxed_and_serves_invocations` |
| `host_environment_is_empty` | `process.rs` `spawned_host_gets_no_environment_and_no_extra_descriptors` |
| `guest_trap_is_reported_and_host_keeps_serving` | `process.rs` `host_exits_after_any_trap`; `host/tests/sdk_e2e.rs` `panic_ends_the_host_and_the_supervisor_restarts_it` (the design changed: exit and restart) |
| `guest_infinite_loop_is_interrupted_inside_host` | `sdk_e2e.rs` `cpu_spin_hits_the_guest_deadline`; `process.rs` `host_exits_after_any_trap` |
| `supervisor_survives_host_crash_and_respawns` | `process.rs` `supervisor_restarts_with_backoff_and_breaks_crash_loops`; `fake_host.rs` `crash_reports_exit_status` |
| `supervisor_kills_hung_host_on_timeout` | `process.rs` `hung_host_is_killed_on_timeout` |
| `invoke_deadline_kills_host_that_never_answers` | `process.rs` `hung_host_is_killed_on_timeout`; `fake_host.rs` `aborted_invocations_get_a_grace_period_then_the_host_is_killed` |
| `host_exits_on_malformed_frame` | `process.rs` `malformed_frames_end_the_host` |
| `frame_codec_rejects_oversized_and_garbage` | `abi/src/frame.rs` `oversized_prefix_is_rejected_before_allocating`, `truncated_and_malformed_frames_fail`, `round_trip` (clean end of file); `abi/src/protocol.rs` `messages_are_strict` (`getDatabaseKey`) |

### Runtime: `broker.rs`

| Spike test | Production test |
| --- | --- |
| `unknown_plugin_and_unknown_op_are_denied` | `broker/tests/broker.rs` `deny_by_default`; `abi/src/capability.rs` tests (unknown ops do not parse); `engine.rs` `malformed_requests_get_errors_and_never_reach_the_handler` |
| `grant_of_one_kind_does_not_imply_another` | **ported**: `broker/tests/grant_kinds.rs` |
| `fs_read_inside_granted_directory_is_allowed`, `fs_read_traversal_is_denied`, `fs_read_absolute_paths_are_denied`, `fs_read_of_directory_or_missing_file_is_denied`, `fs_read_symlink_escapes_are_denied`, `granted_root_reached_through_symlink_is_canonicalized` | not applicable: there is no file capability. `broker/src/permissions.rs` `never_grantable_kinds_fail` (file system never grantable); `engine.rs` `malformed_requests_get_errors_and_never_reach_the_handler` (`fs.read` refused); `fake_host.rs` `denied_requests_are_never_executed` |
| `http_fetch_only_to_allowlisted_origin` | `broker.rs` `allowed_fetch_is_normalized`, `origin_tricks_are_denied`, `methods_and_headers_are_checked`, `redirects_are_rechecked` |
| `revocation_takes_effect_on_next_call` | `broker.rs` `revocation_applies_to_the_next_call`; `process.rs` `capability_calls_go_through_the_broker` |
| `audit_records_decisions_without_values` | `broker.rs` `audit_records_never_contain_values` |
| `e2e_guest_request_is_allowed_or_denied_by_broker` | `process.rs` `capability_calls_go_through_the_broker`; `sdk_e2e.rs` `network_action_is_denied_without_the_grant`, `network_action_is_allowed_with_the_grant_and_revocation_is_immediate` |
| `e2e_identity_comes_from_the_channel_not_the_request` | `process.rs` `identity_comes_from_the_process_not_the_plugin`; `broker.rs` `identity_is_the_session_not_the_request` |
| `e2e_malformed_guest_request_never_reaches_broker` | `engine.rs` `malformed_requests_get_errors_and_never_reach_the_handler`; `fake_host.rs` `denied_requests_are_never_executed` |
| `e2e_many_calls_in_one_invocation` | `engine.rs` `host_call_flooding_is_bounded` (256 calls reach the handler, then the cap); `process.rs` `flooding_capability_calls_is_throttled_then_fatal` |

### Runtime: `sandbox_macos.rs`

| Spike test | Production test |
| --- | --- |
| `control_unsandboxed_host_has_ambient_authority` | `host/tests/sandbox.rs` `control_without_sandbox_can_do_everything` |
| `sandboxed_host_starts_and_runs_wasm` | `process.rs` `host_starts_sandboxed_and_serves_invocations`; `sdk_e2e.rs` `cpu_spin_hits_the_guest_deadline` |
| `sandboxed_host_cannot_read_write_connect_or_exec` | `sandbox.rs` `sandbox_denies_files_network_and_exec`, `internet_egress_is_denied`; the `$HOME` probes are **ported**: `host/tests/sandbox_user_data.rs` |
| `sandboxed_host_can_use_its_scratch_dir_only` | not applicable: the production host has no scratch directory (deny default, nothing writable) |
| `broker_still_works_for_sandboxed_host` | `process.rs` `capability_calls_go_through_the_broker` (the host is sandboxed) |

### Trust: `package.rs`

| Spike test | Production test (`deckpilot-plugin-package/tests/package.rs` unless noted) |
| --- | --- |
| `packages_are_deterministic` | `builds_are_deterministic` |
| `invalid_paths_are_refused_when_building` | `the_builder_refuses_bad_paths`; `src/paths.rs` `single_paths`, `path_sets`; entry count and non-ASCII paths in the builder **ported**: `tests/spike_ports.rs` |
| `correct_package_is_accepted_through_the_signed_chain` | `a_correct_package_verifies_unpacks_and_rechecks`; `deckpilot-plugin-registry/tests/registry.rs` `install_end_to_end_through_the_package_reader` |
| `every_single_byte_change_is_rejected` | `every_bit_flip_is_rejected_before_unpacking` |
| `truncated_and_extended_packages_are_rejected` | `truncation_and_extension_are_rejected` (also the zeroed body) |
| `swapped_packages_are_rejected` | `another_plugin_or_version_is_rejected` |
| `contained_manifest_must_match_id_version_and_pinned_hash` | `another_plugin_or_version_is_rejected` (`WrongPlugin`, `ManifestHashMismatch`) |
| `registry_chain_signed_by_a_wrong_key_yields_no_install` | `registry.rs` `index_signed_by_a_key_outside_the_index_role_is_rejected`, `expired_timestamp_is_rejected` (`InstallError::NeverVerified`) |
| `publisher_signature_must_be_by_the_pinned_key_for_this_exact_file` | not applicable: publisher keys (U-2) are not implemented (see above) |
| `hostile_archives_are_rejected_on_unpack` | `path_traversal_and_absolute_paths_are_rejected`, `links_devices_directories_and_extensions_are_rejected`, `symlink_to_outside_is_rejected`, `duplicate_and_colliding_names_are_rejected`, `contents_must_follow_the_manifest`, `gzip_bombs_are_stopped`, `hidden_data_and_malformed_tar_are_rejected`, `pax_path_override_is_rejected`; a zip archive **ported**: `tests/spike_ports.rs` |
| `five_megabyte_package_verification_time` | measurement only (numbers above) |

### Trust: `registry.rs`

Production `deckpilot-plugin-registry/tests/registry.rs` ports every case;
names changed slightly.

| Spike test | Production test |
| --- | --- |
| `valid_index_is_accepted_and_persisted` | `valid_index_is_accepted_and_persisted` |
| `tampered_index_cache_on_disk_is_not_trusted` | `tampered_index_cache_is_not_trusted` |
| `new_publish_and_daily_timestamp_are_followed` | `new_publishes_and_daily_timestamps_are_followed` |
| `missing_key_set_is_rejected` | `missing_key_set_is_rejected` |
| `oversized_documents_are_rejected_before_parsing` | `oversized_documents_are_rejected_before_parsing` |
| `expired_timestamp_is_rejected` | `expired_timestamp_is_rejected` |
| `clock_skew_allowance_is_five_minutes` | `clock_skew_allowance_is_five_minutes` |
| `expired_index_is_rejected_even_with_a_fresh_timestamp` | `the_timestamp_governs_freshness_not_the_index_expiry` (`IndexExpired` under `Policy::enforce_index_expiry`; registry decision 1) |
| `rollback_to_a_lower_seq_is_rejected_across_runs` | `rollback_to_a_lower_seq_is_rejected_across_restarts` |
| `a_clock_in_the_past_cannot_revive_an_old_index` | `a_clock_in_the_past_cannot_revive_an_old_index` |
| `two_different_indexes_for_one_seq_are_rejected` | `two_indexes_for_one_seq_are_rejected` |
| `freeze_attack_is_detected_at_timestamp_expiry` | `freeze_attack_is_detected_at_timestamp_expiry` |
| `index_signed_by_a_non_authorized_key_is_rejected` | `index_signed_by_a_key_outside_the_index_role_is_rejected` |
| `timestamp_signed_by_another_role_is_rejected` | `timestamp_signed_by_another_role_is_rejected` |
| `signatures_with_another_comment_are_rejected` | `signatures_with_another_comment_are_rejected`, `registry_signatures_never_pass_the_updater_verifier` |
| `tampered_documents_are_rejected` | `every_bit_flip_of_a_signed_document_is_rejected`, `every_bit_flip_of_a_signature_is_rejected_unless_unsigned`, `truncated_documents_are_rejected` |
| `index_key_rotation_via_root_signed_key_set` | `index_key_rotation_through_a_root_signed_key_set` |
| `key_set_signed_by_a_non_root_key_is_rejected` | `key_set_signed_by_a_non_root_key_is_rejected` |
| `root_rotation_needs_old_and_new_roots` | `root_rotation_needs_old_and_new_roots`; `tests/publish.rs` `key_rotation_needs_old_and_new_roots` |
| `two_of_two_root_threshold` | `a_threshold_counts_distinct_keys` |
| `seq_floor_reset_recovers_from_an_inflated_seq` | `seq_floor_reset_recovers_from_an_inflated_seq` |
| `foreign_download_prefix_is_rejected_even_if_signed` | `foreign_download_prefix_is_rejected_even_if_signed` |
| `revoked_versions_are_refused_for_install` | `revoked_and_yanked_versions_are_refused_for_install` |
| `revoked_installed_versions_are_flagged_also_offline_and_stale` | `revoked_installed_versions_are_flagged_offline_and_stale` |
| `dropping_a_revocation_is_rejected` | `revocations_cannot_be_dropped_or_weakened` |
| `tampered_detail_file_is_rejected` | `every_bit_flip_of_a_detail_is_rejected`, `a_detail_must_be_the_pinned_one_for_its_plugin` |
| `offline_behavior` | `offline_behaviour`, `offline_part_way_keeps_the_stored_state` |

The spike's `examples/measure.rs` was a measurement only (numbers above).
Its `publish-workflow.yml.example` is now
`crates/deckpilot-plugin-registry/examples/publish-workflow.yml.example`.
