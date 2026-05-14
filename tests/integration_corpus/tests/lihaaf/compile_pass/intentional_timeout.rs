//! Per-fixture timeout test. The proc-macro expansion sleeps forever;
//! lihaaf kills the worker after `fixture_timeout_secs=3` and the
//! verdict is `TIMEOUT`. Lives in `compile_pass/` because the timeout
//! verdict is orthogonal to the pass/fail directory classification —
//! either way the worker is killed before rustc can report an exit
//! code.

use integration_corpus::corpus_sleep_forever;

corpus_sleep_forever!();

fn main() {}
