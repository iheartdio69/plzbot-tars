use crate::types::CallRecord;

pub fn print_call(c: &CallRecord) {
    println!("📣 CALL {} score={} ts={}", c.mint, c.score, c.ts);
}
