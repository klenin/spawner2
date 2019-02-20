use std::io::*;
fn main() {
	let mut chunk = [0 as u8; 128];
	while let Ok(bytes) = std::io::stdin().read(&mut chunk) {
		if bytes == 0 {
			break;
		}
	}
}