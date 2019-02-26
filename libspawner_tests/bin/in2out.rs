use std::io::*;
fn main() {
    let mut chunk = [0 as u8; 128];
    while let Ok(bytes) = std::io::stdin().read(&mut chunk) {
        if bytes == 0 {
            break;
        }
        let _ = stdout().write_all(&chunk[..bytes]);
        let _ = stderr().write_all(&chunk[..bytes]);
    }
}
