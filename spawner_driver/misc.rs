use std::u64;

pub fn mb2b(mb: f64) -> u64 {
    let b = mb * 1024.0 * 1024.0;
    if b.is_infinite() {
        u64::MAX
    } else {
        b as u64
    }
}

pub fn b2mb(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}
