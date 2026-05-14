//! Per-fixture memory test. The proc-macro expansion allocates 16 MiB
//! per iteration with a 100ms pacing sleep; lihaaf samples RSS every
//! 100ms (`worker::spawn_and_monitor`) and trips
//! `per_fixture_memory_mb=128` within roughly eight iterations,
//! emitting `MEMORY_EXHAUSTED`. The pacing is deliberate: a tight
//! allocation loop races the OS OOM killer, which would surface as
//! `WORKER_CRASHED` and skip the harness-attributed reduction path
//! lihaaf exercises in this verdict class.

use integration_corpus::corpus_oom_allocate;

corpus_oom_allocate!();

fn main() {}
