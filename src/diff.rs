//! Hand-rolled line-granularity unified diff.
//!
//! ## Why hand-rolled
//!
//! This stays in-tree to avoid pulling extra dependencies for a tiny amount of
//! output. For this workload (typically tens to low thousands of lines) the
//! in-tree implementation is cheap and predictable.
//!
//! ## Implementer choice — Myers
//!
//! Myers is a good fit here: it performs well for small-ish snapshots and stays
//! easy enough to audit in this repo.
//!
//! ## Output format
//!
//! GNU-compatible unified diff:
//!
//! ```text
//! --- expected
//! +++ actual
//! @@ -<a>,<b> +<c>,<d> @@
//! -removed line
//! +added line
//!  context line
//! ```
//!
//! Three lines of context around each hunk (the GNU diff `-U3` default).
//!
//! ## Size guardrails
//!
//! - Soft ceiling (10K lines per side): run full diff, no warning.
//! - Between soft and hard: run full diff and emit a warning so callers can
//!   mark snapshots as large.
//! - Above hard (100K lines): return [`DiffResult::TooLarge`] without running
//!   the full algorithm.

use crate::verdict::Verdict;

/// Soft input ceiling.
pub const SOFT_LINE_CEILING: usize = 10_000;

/// Hard input ceiling.
pub const HARD_LINE_CEILING: usize = 100_000;

/// Diff outcome. The `caller` consumes this and produces a [`Verdict`]:
/// `NoChange` → caller's expectation is met (verdict is OK or
/// EXPECTED_PASS_BUT_FAILED depending on context); `Diff` → caller
/// emits a SnapshotDiff verdict; `TooLarge` → caller emits
/// SnapshotDiffTooLarge.
#[derive(Debug, Clone)]
pub enum DiffResult {
    /// The two inputs are byte-identical (after the splitter normalized
    /// line endings via the caller's normalizer).
    NoChange,
    /// Inputs differ. Carries the rendered unified diff.
    Diff {
        /// The unified diff as a string. Always ends with `\n`.
        diff: String,
        /// True when the line count was between the soft and hard
        /// ceiling — caller emits `LARGE_SNAPSHOT` warning.
        warn: bool,
    },
    /// Inputs exceed the hard ceiling. Carries the head of each side.
    TooLarge {
        /// Actual line count.
        actual_lines: usize,
        /// Expected line count.
        expected_lines: usize,
        /// First 100 lines of actual (or fewer if shorter), joined with
        /// `\n`.
        actual_head: String,
        /// First 100 lines of expected (or fewer if shorter).
        expected_head: String,
    },
}

/// Compute a unified diff between `expected` and `actual`. Both inputs
/// are line-split internally; trailing-newline differences do not count
/// as a diff after splitting.
pub fn unified_diff(expected: &str, actual: &str) -> DiffResult {
    let expected_lines: Vec<&str> = split_lines(expected);
    let actual_lines: Vec<&str> = split_lines(actual);

    let n = expected_lines.len().max(actual_lines.len());
    if n > HARD_LINE_CEILING {
        return DiffResult::TooLarge {
            actual_lines: actual_lines.len(),
            expected_lines: expected_lines.len(),
            actual_head: head(&actual_lines, 100),
            expected_head: head(&expected_lines, 100),
        };
    }

    if expected_lines == actual_lines {
        return DiffResult::NoChange;
    }

    let warn = n > SOFT_LINE_CEILING;
    let diff = render_unified_diff(&expected_lines, &actual_lines);
    DiffResult::Diff { diff, warn }
}

/// Split a string into lines without trailing-newline ambiguity. We
/// `lines()` and then drop a trailing empty produced by a trailing
/// newline so `"a\nb\n".split_lines() == ["a", "b"]`.
fn split_lines(s: &str) -> Vec<&str> {
    s.lines().collect()
}

fn head(lines: &[&str], n: usize) -> String {
    lines.iter().take(n).copied().collect::<Vec<_>>().join("\n")
}

/// Compute the Myers edit script and render it as a unified diff with
/// 3 lines of context.
fn render_unified_diff(a: &[&str], b: &[&str]) -> String {
    let ops = myers_diff(a, b);
    let hunks = ops_to_hunks(&ops, a, b, 3);
    let mut out = String::new();
    out.push_str("--- expected\n+++ actual\n");
    for hunk in hunks {
        out.push_str(&hunk.render());
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Op {
    /// Line at (a_idx, b_idx) is unchanged.
    Equal { a: usize, b: usize },
    /// Line at a_idx in `a` was deleted.
    Delete { a: usize },
    /// Line at b_idx in `b` was inserted.
    Insert { b: usize },
}

/// Standard Myers O(ND) diff. Returns the longest-common-subsequence
/// edit script as a list of `Equal`/`Delete`/`Insert` operations in
/// order from start to end.
fn myers_diff<'a>(a: &'a [&'a str], b: &'a [&'a str]) -> Vec<Op> {
    let n = a.len();
    let m = b.len();
    let max = n + m;
    if max == 0 {
        return Vec::new();
    }
    // Single-direction Myers per the original 1986 paper. We track
    // V[k] = furthest x reached on diagonal k after d edits, and
    // record the trace per d so we can backtrack.
    let mut v: Vec<isize> = vec![0; 2 * max + 1];
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let offset = max as isize;
    'outer: for d in 0..=max as isize {
        for k in (-d..=d).step_by(2) {
            let k_idx = (k + offset) as usize;
            let mut x = if k == -d
                || (k != d && v[(k - 1 + offset) as usize] < v[(k + 1 + offset) as usize])
            {
                v[(k + 1 + offset) as usize]
            } else {
                v[(k - 1 + offset) as usize] + 1
            };
            let mut y = x - k;
            // Snake.
            while x < n as isize && y < m as isize && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[k_idx] = x;
            if x >= n as isize && y >= m as isize {
                trace.push(v.clone());
                break 'outer;
            }
        }
        // Snapshot per d.
        trace.push(v.clone());
    }

    // Backtrack.
    let mut x = n as isize;
    let mut y = m as isize;
    let mut ops: Vec<Op> = Vec::new();
    for d in (0..trace.len() as isize).rev() {
        let v = &trace[d as usize];
        let k = x - y;
        let prev_k =
            if k == -d || (k != d && v[(k - 1 + offset) as usize] < v[(k + 1 + offset) as usize]) {
                k + 1
            } else {
                k - 1
            };
        let prev_x = v[(prev_k + offset) as usize];
        let prev_y = prev_x - prev_k;
        // Snake portion: emit Equal ops while x>prev_x AND y>prev_y.
        while x > prev_x && y > prev_y {
            ops.push(Op::Equal {
                a: (x - 1) as usize,
                b: (y - 1) as usize,
            });
            x -= 1;
            y -= 1;
        }
        if d > 0 {
            if x == prev_x {
                // Insertion from b at y-1.
                ops.push(Op::Insert {
                    b: (y - 1) as usize,
                });
            } else {
                // Deletion from a at x-1.
                ops.push(Op::Delete {
                    a: (x - 1) as usize,
                });
            }
        }
        x = prev_x;
        y = prev_y;
    }
    ops.reverse();
    ops
}

#[derive(Debug)]
struct Hunk<'a> {
    a_start: usize, // 1-based
    a_count: usize,
    b_start: usize, // 1-based
    b_count: usize,
    lines: Vec<HunkLine<'a>>,
}

#[derive(Debug)]
enum HunkLine<'a> {
    Context(&'a str),
    Delete(&'a str),
    Insert(&'a str),
}

impl<'a> Hunk<'a> {
    fn render(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            self.a_start, self.a_count, self.b_start, self.b_count
        ));
        for line in &self.lines {
            match line {
                HunkLine::Context(t) => {
                    s.push(' ');
                    s.push_str(t);
                    s.push('\n');
                }
                HunkLine::Delete(t) => {
                    s.push('-');
                    s.push_str(t);
                    s.push('\n');
                }
                HunkLine::Insert(t) => {
                    s.push('+');
                    s.push_str(t);
                    s.push('\n');
                }
            }
        }
        s
    }
}

/// Group an op script into unified-diff hunks with `context` lines of
/// surrounding equal context (GNU `-U` default is 3).
fn ops_to_hunks<'a>(
    ops: &[Op],
    a: &'a [&'a str],
    b: &'a [&'a str],
    context: usize,
) -> Vec<Hunk<'a>> {
    if ops.is_empty() {
        return Vec::new();
    }
    // Group consecutive non-Equal ops into "change runs," extend each
    // by `context` Equal ops on either side, and merge runs that
    // overlap after extension.
    let mut runs: Vec<(usize, usize)> = Vec::new(); // (start, end) op indices, inclusive
    let mut i = 0;
    while i < ops.len() {
        if matches!(ops[i], Op::Equal { .. }) {
            i += 1;
            continue;
        }
        let start = i;
        while i < ops.len() && !matches!(ops[i], Op::Equal { .. }) {
            i += 1;
        }
        let end = i - 1;
        runs.push((start, end));
    }
    if runs.is_empty() {
        return Vec::new();
    }
    let mut extended: Vec<(usize, usize)> = Vec::new();
    for (s, e) in &runs {
        let new_s = s.saturating_sub(context);
        let new_e = (*e + context).min(ops.len() - 1);
        if let Some(last) = extended.last_mut()
            && new_s <= last.1 + 1
        {
            last.1 = last.1.max(new_e);
            continue;
        }
        extended.push((new_s, new_e));
    }

    let mut hunks: Vec<Hunk<'a>> = Vec::new();
    for (start, end) in extended {
        // Find first a-index and b-index covered by this hunk.
        let mut a_start: Option<usize> = None;
        let mut b_start: Option<usize> = None;
        for op in &ops[start..=end] {
            match op {
                Op::Equal { a: ai, b: bi } => {
                    a_start.get_or_insert(*ai);
                    b_start.get_or_insert(*bi);
                }
                Op::Delete { a: ai } => {
                    a_start.get_or_insert(*ai);
                    // b advances 0; keep current b_start or next-equal's
                }
                Op::Insert { b: bi } => {
                    b_start.get_or_insert(*bi);
                }
            }
            if a_start.is_some() && b_start.is_some() {
                break;
            }
        }
        let a_start_idx = a_start.unwrap_or(0);
        let b_start_idx = b_start.unwrap_or(0);

        let mut lines: Vec<HunkLine<'a>> = Vec::new();
        let mut a_count = 0usize;
        let mut b_count = 0usize;
        for op in &ops[start..=end] {
            match op {
                Op::Equal { a: ai, b: _ } => {
                    lines.push(HunkLine::Context(a[*ai]));
                    a_count += 1;
                    b_count += 1;
                }
                Op::Delete { a: ai } => {
                    lines.push(HunkLine::Delete(a[*ai]));
                    a_count += 1;
                }
                Op::Insert { b: bi } => {
                    lines.push(HunkLine::Insert(b[*bi]));
                    b_count += 1;
                }
            }
        }
        hunks.push(Hunk {
            a_start: a_start_idx + 1,
            a_count,
            b_start: b_start_idx + 1,
            b_count,
            lines,
        });
    }
    hunks
}

/// Translate a [`DiffResult`] into a [`Verdict`] for the SnapshotDiff
/// path. Caller provides the actual/expected line counts (for
/// SnapshotDiffTooLarge).
pub fn diff_to_verdict(result: DiffResult) -> Option<Verdict> {
    match result {
        DiffResult::NoChange => None,
        DiffResult::Diff { diff, .. } => Some(Verdict::SnapshotDiff { diff }),
        DiffResult::TooLarge {
            actual_lines,
            expected_lines,
            actual_head,
            expected_head,
        } => Some(Verdict::SnapshotDiffTooLarge {
            actual_lines,
            expected_lines,
            actual_head,
            expected_head,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_inputs_no_change() {
        match unified_diff("a\nb\nc", "a\nb\nc") {
            DiffResult::NoChange => {}
            other => panic!("expected NoChange, got {other:?}"),
        }
    }

    #[test]
    fn single_line_change_renders_unified_diff() {
        let a = "alpha\nbeta\ngamma";
        let b = "alpha\nBETA\ngamma";
        match unified_diff(a, b) {
            DiffResult::Diff { diff, warn } => {
                assert!(!warn);
                assert!(diff.starts_with("--- expected\n+++ actual\n"));
                assert!(diff.contains("-beta"));
                assert!(diff.contains("+BETA"));
                assert!(diff.contains("@@"));
            }
            other => panic!("expected Diff, got {other:?}"),
        }
    }

    #[test]
    fn pure_insertion_renders() {
        let a = "alpha\ngamma";
        let b = "alpha\nbeta\ngamma";
        match unified_diff(a, b) {
            DiffResult::Diff { diff, .. } => {
                assert!(diff.contains("+beta"));
            }
            other => panic!("expected Diff, got {other:?}"),
        }
    }

    #[test]
    fn pure_deletion_renders() {
        let a = "alpha\nbeta\ngamma";
        let b = "alpha\ngamma";
        match unified_diff(a, b) {
            DiffResult::Diff { diff, .. } => {
                assert!(diff.contains("-beta"));
            }
            other => panic!("expected Diff, got {other:?}"),
        }
    }

    #[test]
    fn over_hard_ceiling_returns_too_large() {
        let large_a: String = (0..(HARD_LINE_CEILING + 1))
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let large_b = format!("changed\n{large_a}");
        match unified_diff(&large_a, &large_b) {
            DiffResult::TooLarge {
                actual_lines,
                expected_lines,
                actual_head,
                expected_head,
            } => {
                assert!(actual_lines >= HARD_LINE_CEILING);
                assert!(expected_lines >= HARD_LINE_CEILING);
                assert!(actual_head.lines().count() <= 100);
                assert!(expected_head.lines().count() <= 100);
            }
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    #[test]
    fn between_soft_and_hard_emits_warn() {
        let a: String = (0..(SOFT_LINE_CEILING + 1000))
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut b = a.clone();
        b.push_str("\nextra");
        match unified_diff(&a, &b) {
            DiffResult::Diff { warn, .. } => {
                assert!(warn);
            }
            other => panic!("expected Diff with warn=true, got {other:?}"),
        }
    }

    #[test]
    fn diff_format_has_gnu_compatible_header() {
        let a = "x\ny";
        let b = "x\nY";
        match unified_diff(a, b) {
            DiffResult::Diff { diff, .. } => {
                assert!(diff.starts_with("--- expected\n+++ actual\n"));
                let header_line = diff
                    .lines()
                    .find(|l| l.starts_with("@@"))
                    .expect("hunk header present");
                // Shape: `@@ -A,B +C,D @@`
                assert!(header_line.starts_with("@@ -"));
                assert!(header_line.ends_with("@@"));
            }
            other => panic!("expected Diff, got {other:?}"),
        }
    }
}
