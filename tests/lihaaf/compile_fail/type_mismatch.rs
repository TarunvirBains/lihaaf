// Compile_fail fixture with a deliberate type error. Exercises the
// snapshot-diff path: rustc exits non-zero, lihaaf normalizes the
// stderr per spec §6.2 and compares against the sibling
// `.stderr` file.

fn main() {
    let _x: u8 = "not a number";
}
